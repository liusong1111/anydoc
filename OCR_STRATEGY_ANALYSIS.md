# OCR 策略改进方案分析

## 当前问题

现有的 `OcrStrategy` 基于简单的**形状+尺寸**启发式判断，无法智能区分：
1. **文字图片** vs **插图/装饰图**
2. **每个汉字一张小图的 PDF**（文字碎片化）
3. **多页拼接的长图**、**宽幅截图**
4. **表格内的图表** vs **独立的文档扫描**

## MinerU 的做法

### 1. PDF 分类（pdf_classify.py）

MinerU 首先对 **整个 PDF** 进行分类（`"txt"` vs `"ocr"`），判断依据：

```python
# 核心检测信号（按优先级）：
1. 文字密度：avg_chars_per_page < 50 → OCR
2. Unicode 映射错误率：>4% → OCR（乱码检测）
3. CID 字体无 ToUnicode：>1% 使用率 → OCR（每字一图的情况）
4. 字体编码异常：Latin CharSet 解码成 CJK → OCR
5. 文字质量：异常字符（null/replacement/control）>3% → OCR
6. 跨脚本混杂：多种文字系统混用 → OCR（乱码特征）
7. 可疑字符区间：U+7280-U+72DF 高占比 → OCR
8. ASCII 标点密集：连续标点 run >10% → OCR
9. 图片覆盖率：>80% 页面被图片覆盖 → OCR
```

### 2. 嵌入式图片处理

MinerU **不在文档解析阶段**判断单张图是否需要 OCR，而是：
- **统一提取所有图片**
- 在后处理阶段通过 **布局分析模型**（PDF-Extract-Kit）识别图片类型
- 使用 **语义分类**而非几何启发式

## 业界最佳实践

### 关键指标

1. **文字密度检测**（最有效）
   - 快速 OCR 采样（低分辨率）
   - 如果识别出大量文字 → 是文档扫描
   - 如果只有少量/无文字 → 是插图

2. **边缘密度分析**
   - 文字图片：高密度边缘（字形轮廓）
   - 自然图片：低密度或连续渐变

3. **颜色直方图**
   - 文档扫描：双峰分布（黑字白底）
   - 照片插图：多峰连续分布

4. **OCR 置信度**
   - Tesseract/PaddleOCR 返回的 confidence score
   - 高置信度 → 确实是文字
   - 低置信度 → 可能是噪点/装饰图案

## 推荐方案

### 方案 A：轻量级启发式（现实可行）

```rust
pub enum OcrDecision {
    /// 确定是文档扫描，执行完整 OCR
    DefinitelyText,
    /// 可能是文字，快速采样验证
    MaybeText,
    /// 确定不是文字（logo、装饰图）
    NotText,
}

fn classify_image(asset: &Asset, context: &BlockContext) -> OcrDecision {
    // 1. 快速排除
    if context.in_table { return NotText; }
    if asset.bytes.len() < 10_000 { return NotText; } // 太小
    
    let Some((w, h)) = dimensions(&asset.bytes) else { return NotText; };
    let long = w.max(h);
    if long < 800 { return NotText; } // 分辨率太低
    
    // 2. 颜色分析（快速）
    let histogram = analyze_color_histogram(&asset.bytes);
    if histogram.is_bimodal() { // 黑白二值化特征
        return DefinitelyText;
    }
    
    // 3. 边缘密度（中速）
    let edge_density = calculate_edge_density(&asset.bytes);
    if edge_density > HIGH_TEXT_THRESHOLD {
        return DefinitelyText;
    }
    if edge_density < LOW_TEXT_THRESHOLD {
        return NotText;
    }
    
    // 4. 灰度均值（快速）
    let mean_gray = calculate_mean_grayscale(&asset.bytes);
    if (200..=250).contains(&mean_gray) { // 接近白纸
        return MaybeText;
    }
    
    MaybeText
}
```

### 方案 B：基于采样 OCR（更准确但慢）

```rust
fn should_ocr_image(
    asset: &Asset,
    backend: &dyn OcrBackend,
    strategy: OcrStrategy,
) -> bool {
    // 快速启发式预筛
    let decision = classify_image(asset, context);
    match (decision, strategy) {
        (NotText, _) => false,
        (DefinitelyText, _) => true,
        (MaybeText, Conservative) => false, // 保守模式放弃
        (MaybeText, Aggressive) => {
            // 快速采样 OCR（降采样到 1/4）
            let downsampled = downsample_image(&asset.bytes, 0.5);
            let result = backend.recognize(&downsampled, &quick_options());
            
            // 判断条件：
            // 1. 识别出足够多字符
            // 2. 置信度足够高
            let char_count = result.text.chars().count();
            char_count > 20 && result.confidence > 0.6
        }
    }
}
```

### 方案 C：混合策略（推荐）

```rust
pub enum OcrStrategy {
    /// 只 OCR 高置信度文档扫描（形状+颜色+边缘）
    Conservative,
    /// 对模棱两可的图片进行快速采样验证
    Smart,
    /// OCR 所有大图
    Aggressive,
}
```

## 实现优先级

### Phase 1: 快速改进（1-2天）
- [x] 让 Aggressive 模式跳过形状限制（已完成）
- [ ] 添加颜色直方图分析（判断黑白文档 vs 彩色图片）
- [ ] 添加边缘密度检测（区分文字密集 vs 照片）

### Phase 2: 智能采样（3-5天）
- [ ] 实现快速降采样 OCR
- [ ] 添加置信度阈值判断
- [ ] 新增 `Smart` 策略

### Phase 3: 深度优化（可选）
- [ ] 集成轻量级文字检测模型（如 EAST、DBNet）
- [ ] 缓存图片特征避免重复计算
- [ ] 多线程并行处理大量图片

## 参考资料

- [MinerU PDF分类](https://github.com/opendatalab/MinerU/blob/master/mineru/utils/pdf_classify.py)
- [Tesseract Confidence Scores](https://tesseract-ocr.github.io/)
- [Document vs Natural Image Classification](https://arxiv.org/abs/2409.18839) (MinerU Paper)
