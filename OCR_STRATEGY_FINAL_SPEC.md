# OCR 策略最终实现方案

## 目标

实现轻量级、智能的嵌入式图片文字检测，接近 MinerU 的准确率但保持轻量级（无深度学习模型依赖）。

---

## 策略定义（4 层递进）

```rust
pub enum OcrStrategy {
    /// 禁用嵌入式图片 OCR
    /// 
    /// 只处理 PDF 无文字页（scanned pages）。
    /// 适用：已有文字层的文档。
    Disabled,
    
    /// 保守模式：只 OCR 高置信度文档扫描
    /// 
    /// 判断条件（AND）：
    /// 1. 尺寸：(标准纸张比例 0.68-0.80 AND 长边≥1500)
    ///         OR (短边≥1200 AND 长边≥1800)  [高分辨率兜底]
    /// 2. 特征：黑白双峰直方图 OR 高边缘密度(>0.15)
    /// 3. 排除：行内小图（has_adjacent_text + Conservative）
    /// 
    /// 适用：标准文档扫描，避免误报。
    /// 准确率：95%+，误报率：<1%
    Conservative,
    
    /// 智能模式：特征分析 + 采样验证（推荐默认）
    /// 
    /// 三阶段决策：
    /// 1. High 置信度 + Conservative 阈值 → 直接 OCR
    /// 2. High 置信度 + 低阈值（长≥1200, 短≥600） → 直接 OCR
    /// 3. Medium 置信度 → 降采样 1/2 快速 OCR
    ///    - 识别出 ≥20 字符 AND 置信度 ≥0.6 → OCR 全图
    ///    - 否则跳过
    /// 4. Low 置信度 → 跳过
    /// 
    /// 适用：混合文档（文字 + 图表 + 扫描页）。
    /// 准确率：90%+，误报率：<5%
    Smart,
    
    /// 激进模式：最宽松阈值
    /// 
    /// = Smart 的所有情况 +
    /// - High: 长边≥800 + 短边≥400
    /// - Medium: 采样验证
    /// - Low: 长边≥1000 + 边缘密度>0.05 (排除纯色块)
    /// 
    /// 适用：截图、宽幅图、拼接图、非标准布局。
    /// 准确率：85%+，误报率：10-15%
    Aggressive,
}
```

**包含关系**：`Disabled ⊂ Conservative ⊂ Smart ⊂ Aggressive`

---

## 核心组件设计

### 1. 图像特征提取（src/ocr/image_features.rs）

```rust
/// 图像特征
pub struct ImageFeatures {
    /// 灰度直方图特征
    pub histogram: HistogramFeatures,
    /// 边缘密度 [0.0, 1.0]
    pub edge_density: f32,
    /// 图片尺寸 (width, height)
    pub dimensions: (u32, u32),
}

/// 直方图特征
pub struct HistogramFeatures {
    /// 是否为双峰分布（黑白文档特征）
    pub is_bimodal: bool,
    /// 灰度均值 [0, 255]
    pub mean_gray: u8,
    /// 标准差（对比度）
    pub std_dev: f32,
}

/// 文字可能性
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextLikelihood {
    /// 明显不是文字（照片、纯色块、低对比度）
    Low,
    /// 可能是文字，需要进一步验证
    Medium,
    /// 高置信度文字图片（双峰 + 高边缘密度）
    High,
}

/// 提取图像特征
pub fn extract_features(bytes: &[u8]) -> Result<ImageFeatures, ImageError> {
    let img = image::load_from_memory(bytes)?;
    let (width, height) = img.dimensions();
    
    // 转灰度图
    let gray = img.to_luma8();
    
    // 降采样到 1/2 加速计算
    let small = if width > 1000 || height > 1000 {
        image::imageops::resize(
            &gray,
            width / 2,
            height / 2,
            image::imageops::FilterType::Nearest,
        )
    } else {
        gray.clone()
    };
    
    Ok(ImageFeatures {
        histogram: analyze_histogram(&small),
        edge_density: calculate_edge_density(&small),
        dimensions: (width, height),
    })
}

/// 分析灰度直方图
fn analyze_histogram(img: &GrayImage) -> HistogramFeatures {
    let mut hist = [0u32; 256];
    let mut sum = 0u64;
    
    for pixel in img.pixels() {
        let val = pixel[0] as usize;
        hist[val] += 1;
        sum += pixel[0] as u64;
    }
    
    let total = img.width() * img.height();
    let mean = (sum / total as u64) as u8;
    
    // 计算标准差
    let variance: f64 = img.pixels()
        .map(|p| {
            let diff = p[0] as f64 - mean as f64;
            diff * diff
        })
        .sum::<f64>() / total as f64;
    let std_dev = variance.sqrt() as f32;
    
    // 检测双峰
    let is_bimodal = detect_bimodal(&hist, total);
    
    HistogramFeatures {
        is_bimodal,
        mean_gray: mean,
        std_dev,
    }
}

/// 双峰检测：黑白文档特征
fn detect_bimodal(hist: &[u32; 256], total: u32) -> bool {
    // 1. 平滑直方图（5-点移动平均）
    let mut smoothed = [0u32; 256];
    for i in 2..254 {
        smoothed[i] = (hist[i-2] + hist[i-1] + hist[i] + hist[i+1] + hist[i+2]) / 5;
    }
    
    // 2. 找峰值（局部最大值）
    let mut peaks = Vec::new();
    for i in 1..255 {
        if smoothed[i] > smoothed[i-1] && smoothed[i] > smoothed[i+1] {
            peaks.push((i, smoothed[i]));
        }
    }
    
    if peaks.len() < 2 {
        return false;
    }
    
    // 3. 按高度排序，取前两个峰
    peaks.sort_by_key(|&(_, h)| std::cmp::Reverse(h));
    let (peak1_pos, peak1_height) = peaks[0];
    let (peak2_pos, peak2_height) = peaks[1];
    
    // 4. 判断条件：
    // - 两峰距离 > 100（足够分离）
    // - 两峰高度都 > 5%（显著峰值）
    // - 均值偏向白色（200-250，白纸特征）
    let distance = (peak1_pos as i32 - peak2_pos as i32).abs();
    let threshold = (total as f32 * 0.05) as u32;
    
    distance > 100 
        && peak1_height > threshold 
        && peak2_height > threshold
}

/// Sobel 边缘检测
fn calculate_edge_density(img: &GrayImage) -> f32 {
    let (w, h) = img.dimensions();
    if w < 3 || h < 3 {
        return 0.0;
    }
    
    let mut edge_count = 0u32;
    let total = (w - 2) * (h - 2);
    
    // Sobel 核
    const SOBEL_X: [i32; 9] = [-1, 0, 1, -2, 0, 2, -1, 0, 1];
    const SOBEL_Y: [i32; 9] = [-1, -2, -1, 0, 0, 0, 1, 2, 1];
    
    for y in 1..h-1 {
        for x in 1..w-1 {
            let mut gx = 0i32;
            let mut gy = 0i32;
            
            for dy in 0..3 {
                for dx in 0..3 {
                    let px = img.get_pixel(x + dx - 1, y + dy - 1)[0] as i32;
                    let idx = dy * 3 + dx;
                    gx += px * SOBEL_X[idx];
                    gy += px * SOBEL_Y[idx];
                }
            }
            
            let magnitude = ((gx * gx + gy * gy) as f32).sqrt();
            if magnitude > 128.0 {  // 边缘阈值
                edge_count += 1;
            }
        }
    }
    
    edge_count as f32 / total as f32
}

/// 综合判断文字可能性
pub fn classify_features(features: &ImageFeatures) -> TextLikelihood {
    let hist = &features.histogram;
    let edge = features.edge_density;
    
    // Low: 明显不是文字
    if edge < 0.03 {
        // 边缘太少（纯色块、渐变）
        return TextLikelihood::Low;
    }
    
    if hist.std_dev < 30.0 {
        // 对比度太低（灰蒙蒙的照片）
        return TextLikelihood::Low;
    }
    
    // High: 高置信度文字
    if hist.is_bimodal && edge > 0.15 && hist.mean_gray > 200 {
        // 黑白双峰 + 高边缘密度 + 白底
        return TextLikelihood::High;
    }
    
    if edge > 0.18 && hist.std_dev > 60.0 {
        // 极高边缘密度 + 高对比度（文字密集）
        return TextLikelihood::High;
    }
    
    // Medium: 需要进一步验证
    if edge > 0.08 || hist.is_bimodal {
        return TextLikelihood::Medium;
    }
    
    TextLikelihood::Low
}
```

---

### 2. 采样 OCR 验证（src/ocr/sampling.rs）

```rust
/// 采样 OCR 配置
pub struct SamplingOcrOptions {
    /// 降采样比例（0.5 = 1/4 像素数）
    pub downsample_ratio: f32,
    /// 最小字符数阈值
    pub min_char_count: usize,
    /// 最小置信度阈值
    pub min_confidence: f32,
}

impl Default for SamplingOcrOptions {
    fn default() -> Self {
        Self {
            downsample_ratio: 0.5,
            min_char_count: 20,
            min_confidence: 0.6,
        }
    }
}

/// 快速采样 OCR 验证
pub fn quick_sample_ocr(
    asset: &Asset,
    backend: &dyn OcrBackend,
    options: &SamplingOcrOptions,
) -> Result<bool, OcrError> {
    // 1. 降采样
    let img = image::load_from_memory(&asset.bytes)
        .map_err(|e| OcrError::ImageDecode(e.to_string()))?;
    
    let new_width = (img.width() as f32 * options.downsample_ratio) as u32;
    let new_height = (img.height() as f32 * options.downsample_ratio) as u32;
    
    let small = image::imageops::resize(
        &img,
        new_width.max(400),  // 最小 400px，保证 OCR 可用
        new_height.max(400),
        image::imageops::FilterType::Triangle,
    );
    
    // 2. 编码为 PNG
    let mut buf = Vec::new();
    small.write_to(
        &mut std::io::Cursor::new(&mut buf),
        image::ImageFormat::Png,
    ).map_err(|e| OcrError::ImageEncode(e.to_string()))?;
    
    // 3. 快速 OCR
    let ocr_options = OcrOptions::default();
    let result = backend.recognize(&buf, &ocr_options)?;
    
    // 4. 判断
    let char_count = result.text.chars().filter(|c| !c.is_whitespace()).count();
    let is_text = char_count >= options.min_char_count 
                  && result.confidence >= options.min_confidence;
    
    log::debug!(
        "Quick OCR sample: {} chars (min {}), confidence {:.2} (min {:.2}), is_text={}",
        char_count,
        options.min_char_count,
        result.confidence,
        options.min_confidence,
        is_text
    );
    
    Ok(is_text)
}
```

---

### 3. 策略决策核心（src/ocr/strategy.rs 重构）

```rust
use crate::model::{Asset, Block, CellSlot, Document, ImageSource, Inline};
use super::backend::{OcrBackend, OcrOptions};
use super::image_features::{extract_features, classify_features, TextLikelihood};
use super::sampling::{quick_sample_ocr, SamplingOcrOptions};

/// OCR 策略
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OcrStrategy {
    Conservative,
    #[default]
    Smart,
    Aggressive,
    Disabled,
}

/// 图片上下文
#[derive(Debug, Clone, Copy, Default)]
pub struct BlockContext {
    /// 在表格内（微调阈值，不硬排除）
    pub in_table: bool,
    /// 与文字混排（行内小图，Conservative 跳过）
    pub has_adjacent_text: bool,
}

/// 判断是否应该 OCR 图片
pub fn should_ocr_image(
    asset: &Asset,
    context: &BlockContext,
    strategy: OcrStrategy,
    backend: Option<&dyn OcrBackend>,
) -> bool {
    // 0. 策略禁用
    if matches!(strategy, OcrStrategy::Disabled) {
        return false;
    }
    
    // 1. 文件类型检查
    if !asset.media_type.starts_with("image/") {
        return false;
    }
    
    // 2. 尺寸检查
    let Some((width, height)) = image_dimensions(&asset.bytes) else {
        return false;
    };
    let long = width.max(height);
    let short = width.min(height);
    
    // 太小的图片直接跳过
    if long < 400 || short < 200 || asset.bytes.len() < 10_000 {
        return false;
    }
    
    // 3. 行内小图：Conservative 跳过
    if context.has_adjacent_text && matches!(strategy, OcrStrategy::Conservative) {
        return false;
    }
    
    // 4. 提取图像特征
    let Ok(features) = extract_features(&asset.bytes) else {
        log::debug!("Failed to extract features, skipping OCR");
        return false;
    };
    
    let likelihood = classify_features(&features);
    
    log::debug!(
        "Image {}x{}, likelihood={:?}, edge_density={:.3}, bimodal={}, in_table={}",
        width, height, likelihood, features.edge_density,
        features.histogram.is_bimodal, context.in_table
    );
    
    // 5. 表格内：略微提高尺寸要求（避免小 logo）
    let (min_long, min_short) = if context.in_table {
        (1000, 600)
    } else {
        (800, 400)
    };
    
    // 6. 根据特征和策略判断
    match likelihood {
        TextLikelihood::High => {
            should_ocr_high_likelihood(
                width, height, long, short,
                &features, strategy, min_long, min_short
            )
        }
        
        TextLikelihood::Medium => {
            should_ocr_medium_likelihood(
                asset, backend, strategy, long, short, min_long, min_short
            )
        }
        
        TextLikelihood::Low => {
            should_ocr_low_likelihood(
                &features, strategy, long, context.in_table
            )
        }
    }
}

/// High 置信度判断
fn should_ocr_high_likelihood(
    width: u32,
    height: u32,
    long: u32,
    short: u32,
    features: &ImageFeatures,
    strategy: OcrStrategy,
    min_long: u32,
    min_short: u32,
) -> bool {
    let ratio = short as f32 / long as f32;
    let paper_like = (0.68..=0.80).contains(&ratio);
    
    // Conservative: 严格条件
    if matches!(strategy, OcrStrategy::Conservative) {
        // 标准纸张形状 OR 高分辨率兜底
        let standard = paper_like && long >= 1500;
        let high_res = short >= 1200 && long >= 1800;
        return standard || high_res;
    }
    
    // Smart: 中等阈值
    if matches!(strategy, OcrStrategy::Smart) {
        return long >= 1200 && short >= 600;
    }
    
    // Aggressive: 低阈值
    if matches!(strategy, OcrStrategy::Aggressive) {
        return long >= min_long && short >= min_short;
    }
    
    false
}

/// Medium 置信度判断
fn should_ocr_medium_likelihood(
    asset: &Asset,
    backend: Option<&dyn OcrBackend>,
    strategy: OcrStrategy,
    long: u32,
    short: u32,
    min_long: u32,
    min_short: u32,
) -> bool {
    // Conservative: 放弃
    if matches!(strategy, OcrStrategy::Conservative) {
        return false;
    }
    
    // Smart/Aggressive: 采样验证
    if matches!(strategy, OcrStrategy::Smart | OcrStrategy::Aggressive) {
        if let Some(backend) = backend {
            // 只对足够大的图片采样
            if long >= min_long && short >= min_short {
                return quick_sample_ocr(asset, backend, &SamplingOcrOptions::default())
                    .unwrap_or(false);
            }
        }
    }
    
    false
}

/// Low 置信度判断
fn should_ocr_low_likelihood(
    features: &ImageFeatures,
    strategy: OcrStrategy,
    long: u32,
    in_table: bool,
) -> bool {
    // 只有 Aggressive 且不在表格内才考虑
    if matches!(strategy, OcrStrategy::Aggressive) && !in_table {
        // 极低阈值：大尺寸 + 最低边缘密度（排除纯色块）
        return long >= 1000 && features.edge_density > 0.05;
    }
    
    false
}

/// 读取图片尺寸（快速，不解码全图）
fn image_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .ok()?
        .into_dimensions()
        .ok()
}

/// 遍历文档应用 OCR
pub(crate) fn apply_to_document(
    doc: &mut Document,
    backend: &dyn OcrBackend,
    strategy: OcrStrategy,
    options: &OcrOptions,
) {
    if matches!(strategy, OcrStrategy::Disabled) {
        return;
    }
    
    walk_blocks(&mut doc.blocks, &doc.assets, backend, strategy, options, false);
    
    for note in &mut doc.notes {
        walk_blocks(&mut note.blocks, &doc.assets, backend, strategy, options, false);
    }
}

fn walk_blocks(
    blocks: &mut [Block],
    assets: &[Asset],
    backend: &dyn OcrBackend,
    strategy: OcrStrategy,
    options: &OcrOptions,
    in_table: bool,
) {
    for block in blocks {
        match block {
            Block::Paragraph(inlines) => {
                walk_inlines(inlines, assets, backend, strategy, options, in_table);
            }
            Block::Heading { content, .. } => {
                walk_inlines(content, assets, backend, strategy, options, in_table);
            }
            Block::List(list) => {
                for item in &mut list.items {
                    walk_blocks(&mut item.blocks, assets, backend, strategy, options, in_table);
                }
            }
            Block::Table(table) => {
                for slot in table.grid.iter_mut().flatten() {
                    if let CellSlot::Origin(cell) = slot {
                        walk_blocks(&mut cell.blocks, assets, backend, strategy, options, true);
                    }
                }
            }
            Block::BlockQuote(nested) => {
                walk_blocks(nested, assets, backend, strategy, options, in_table);
            }
            Block::CodeBlock { .. } | Block::Rule => {}
        }
    }
}

fn walk_inlines(
    inlines: &mut [Inline],
    assets: &[Asset],
    backend: &dyn OcrBackend,
    strategy: OcrStrategy,
    options: &OcrOptions,
    in_table: bool,
) {
    // 检测是否有相邻文字
    let has_adjacent_text = inlines
        .iter()
        .any(|inline| matches!(inline, Inline::Text { text, .. } if !text.trim().is_empty()));
    
    let context = BlockContext {
        in_table,
        has_adjacent_text,
    };
    
    for inline in inlines {
        if let Inline::Image { alt, source } = inline {
            let ImageSource::Asset(id) = source else { continue };
            let Some(asset) = assets.get(id.0) else { continue };
            
            if !should_ocr_image(asset, &context, strategy, Some(backend)) {
                continue;
            }
            
            // 执行 OCR
            match backend.recognize(&asset.bytes, options) {
                Ok(result) if !result.text.trim().is_empty() => {
                    log::info!(
                        "OCR'd embedded image (asset {}, confidence {:.2}): {} chars",
                        id.0,
                        result.confidence,
                        result.text.len()
                    );
                    *alt = result.text;
                }
                Ok(_) => {
                    log::debug!("Embedded image (asset {}) OCR'd to no text", id.0);
                }
                Err(e) => {
                    log::warn!("OCR failed for embedded image (asset {}): {e}", id.0);
                }
            }
        } else if let Inline::Link { content, .. } = inline {
            walk_inlines(content, assets, backend, strategy, options, in_table);
        }
    }
}
```

---

## 测试策略

### 测试数据集

```
tests/ocr_strategy/
├── standard_scans/           # 标准 A4/Letter 扫描（应该全识别）
│   ├── a4_150dpi.png        # 1240×1754, 比例 0.71
│   ├── letter_200dpi.png    # 1700×2200, 比例 0.77
│   └── landscape.png        # 2200×1700, 比例 0.77
├── photos/                   # 照片（应该全跳过）
│   ├── photo_3x2.jpg        # 1500×1000, 比例 0.67
│   ├── square_logo.png      # 800×800, 比例 1.0
│   └── portrait.jpg         # 1080×1920, 比例 0.56
├── edge_cases/               # 边界情况
│   ├── long_stitch.png      # 1504×4295, 比例 0.35（教案案例）
│   ├── near_square.png      # 1498×1633, 比例 0.92（教案案例）
│   ├── wide_screenshot.png  # 1920×1080, 比例 0.56
│   └── small_text.png       # 600×800, 文字但太小
└── mixed_docs/               # 混合文档
    ├── with_charts.docx     # 文字 + 图表 + 扫描页
    └── presentation.pptx    # 截图 + 装饰图
```

### 单元测试

```rust
// tests/ocr_strategy_tests.rs

#[test]
fn test_standard_scans() {
    let cases = [
        ("a4_150dpi.png", true, true, true),
        ("letter_200dpi.png", true, true, true),
    ];
    
    for (file, conservative, smart, aggressive) in cases {
        let asset = load_test_asset(file);
        let context = BlockContext::default();
        
        assert_eq!(
            should_ocr_image(&asset, &context, OcrStrategy::Conservative, None),
            conservative,
            "{} Conservative",
            file
        );
        // ... Smart, Aggressive
    }
}

#[test]
fn test_photos_skipped() {
    let cases = ["photo_3x2.jpg", "square_logo.png"];
    
    for file in cases {
        let asset = load_test_asset(file);
        let context = BlockContext::default();
        
        assert!(!should_ocr_image(&asset, &context, OcrStrategy::Conservative, None));
        assert!(!should_ocr_image(&asset, &context, OcrStrategy::Smart, None));
        // Aggressive 可能误报，但应该<15%
    }
}

#[test]
fn test_edge_cases() {
    // 教案案例
    let long_stitch = load_test_asset("long_stitch.png");
    let context = BlockContext::default();
    
    // Conservative: 短边 1504 ≥ 1200 且长边 4295 ≥ 1800 → 应该通过
    assert!(should_ocr_image(&long_stitch, &context, OcrStrategy::Conservative, None));
}

#[test]
fn test_in_table_context() {
    let scan = load_test_asset("a4_150dpi.png");
    
    let normal_ctx = BlockContext { in_table: false, has_adjacent_text: false };
    let table_ctx = BlockContext { in_table: true, has_adjacent_text: false };
    
    // 表格内也应该识别高置信度扫描
    assert!(should_ocr_image(&scan, &table_ctx, OcrStrategy::Smart, None));
}

#[test]
fn test_inline_image_skipped() {
    let small_icon = load_test_asset("emoji.png");
    let inline_ctx = BlockContext { in_table: false, has_adjacent_text: true };
    
    // Conservative 跳过行内图
    assert!(!should_ocr_image(&small_icon, &inline_ctx, OcrStrategy::Conservative, None));
    
    // Smart 如果特征明显也可能 OCR（采样验证）
}
```

---

## 实现步骤

### Phase 1: 图像特征提取（2-3 天）

**目标**：实现 `src/ocr/image_features.rs`

- [ ] Day 1: 基础框架 + 灰度直方图分析
  - `ImageFeatures` 结构
  - `analyze_histogram()`
  - `detect_bimodal()`
  
- [ ] Day 2: 边缘检测
  - Sobel 算子实现
  - `calculate_edge_density()`
  - 性能优化（降采样）
  
- [ ] Day 3: 综合判断 + 单元测试
  - `classify_features()`
  - 阈值调优
  - 单元测试覆盖

**验收标准**：
```bash
cargo test image_features
# 所有测试通过
# 标准扫描识别为 High
# 照片识别为 Low
```

---

### Phase 2: 采样 OCR（1-2 天）

**目标**：实现 `src/ocr/sampling.rs`

- [ ] Day 1: 降采样 + OCR 集成
  - `SamplingOcrOptions`
  - `quick_sample_ocr()`
  - 错误处理
  
- [ ] Day 2: 测试 + 阈值调优
  - 测试不同采样比例
  - 调整字符数/置信度阈值
  - 性能测试

**验收标准**：
```bash
# 采样 OCR 速度 < 全图 OCR 的 30%
cargo test sampling -- --nocapture
```

---

### Phase 3: 策略集成（2 天）

**目标**：重构 `src/ocr/strategy.rs`

- [ ] Day 1: 核心决策逻辑
  - `should_ocr_image()` 重写
  - 三层判断（High/Medium/Low）
  - `BlockContext` 微调
  
- [ ] Day 2: 集成测试
  - 端到端测试（完整文档）
  - 策略对比测试
  - 日志优化

**验收标准**：
```bash
cargo test ocr::strategy
cargo run -- --ocr --ocr-strategy smart ~/文档/图片教案.docx
# 两张图都识别出来
```

---

### Phase 4: 依赖更新 + 文档（1 天）

- [ ] 更新 `Cargo.toml`
  ```toml
  [dependencies]
  image = "0.25"         # 已有
  # imageproc = "0.25"   # 可选，暂不引入（手写 Sobel）
  ```

- [ ] 更新 CLI 默认策略
  ```rust
  // src/bin/any2md/main.rs
  #[derive(ValueEnum)]
  enum OcrStrategyArg {
      Disabled,
      Conservative,
      #[default]
      Smart,      // 改为默认
      Aggressive,
  }
  ```

- [ ] 更新 README.md
  - OCR 策略说明
  - 使用示例
  - 性能对比表

- [ ] 更新 CHANGELOG.md

---

## 性能目标

| 操作 | 目标时间 | 说明 |
|------|---------|------|
| 特征提取 | < 20ms | 降采样 + 直方图 + Sobel |
| 采样 OCR | < 200ms | 1/4 分辨率 OCR |
| 完整 OCR | 500-2000ms | 全分辨率 OCR（基准） |

**总体**：
- Conservative: 只特征提取，几乎无性能损失
- Smart: 20-30% 图片需要采样，平均增加 50-100ms/图
- Aggressive: 与 Smart 相近

---

## 回归测试

确保不破坏现有功能：

```bash
# 现有测试必须全部通过
cargo test

# 现有样本文档输出不变（不带 OCR）
cargo run -- samples/example.pdf > output.md
diff output.md expected_output.md

# 带 OCR 的输出包含图片文字
cargo run -- --ocr samples/scanned.pdf | grep "扫描文字内容"
```

---

## 调优计划

实现完成后，基于真实数据调整：

1. **阈值调优**
   - 边缘密度阈值（0.15, 0.08, 0.05）
   - 采样 OCR 阈值（字符数 20, 置信度 0.6）
   - 尺寸阈值（根据误报率调整）

2. **性能优化**
   - Sobel 算子并行化
   - 特征缓存（同一图片多次引用）
   - 批量处理（多图片文档）

3. **边界情况处理**
   - 极端宽高比（>10:1）
   - 旋转图片
   - 低质量扫描

---

## 最终验收标准

### 功能验收

- [x] 四种策略定义清晰
- [ ] Conservative: 95%+ 准确率，<1% 误报
- [ ] Smart: 90%+ 准确率，<5% 误报
- [ ] Aggressive: 85%+ 准确率，<15% 误报
- [ ] 你的教案案例两张图都能识别（Conservative 模式）

### 代码质量

- [ ] 所有单元测试通过
- [ ] 集成测试覆盖主要场景
- [ ] 代码有清晰注释
- [ ] 错误处理完善（不崩溃）

### 文档完备

- [ ] README 更新策略说明
- [ ] 代码内文档（rustdoc）
- [ ] CHANGELOG 记录变更

### 性能达标

- [ ] 特征提取 < 20ms
- [ ] Smart 策略增加的平均时间 < 100ms/图

---

## 依赖清单

```toml
[dependencies]
# 现有依赖
image = "0.25"
log = "0.4"

# 无需新增依赖（手写 Sobel 算子）
```

---

## 风险与缓解

| 风险 | 影响 | 缓解措施 |
|------|------|---------|
| 阈值难调优 | 准确率不达标 | 多轮测试 + 真实数据反馈 |
| 采样 OCR 太慢 | Smart 策略性能差 | 降低采样比例/提高判断精度 |
| 边缘检测误判 | 误报率高 | 结合多个特征（直方图+边缘） |
| Sobel 算子性能 | 特征提取慢 | 降采样 + 并行化 |

---

## 后续扩展

实现完成后，如需进一步提升：

1. **轻量级文字检测模型**（10-20MB）
   - EAST / DBNet ONNX 模型
   - 需要 `onnxruntime` 依赖

2. **机器学习分类器**
   - 训练随机森林
   - 输入：特征向量
   - 输出：文字概率

3. **用户反馈学习**
   - 记录用户修正
   - 动态调整阈值

但当前方案已满足 80-90% 需求。
