# any2md 设计文档

> 本文档是项目的唯一权威设计说明。实施进度见 [IMPLEMENTATION_PLAN.md](IMPLEMENTATION_PLAN.md)。

## 1. 项目目标

构建高性能的文档转 Markdown 工具 **any2md**：

1. **全格式支持**：DOCX/PPTX/XLSX、DOC/PPT/XLS（OLE2 二进制老格式）、PDF（文本型 + 扫描型）、ODF/RTF/EPUB/CSV
2. **本地 OCR**（可选，`--ocr` 开启）：自动检测扫描 PDF 页和 Office 文档内嵌扫描图，进程内推理，数据不出本机
3. **精确文件类型识别**：二进制签名 + OLE 流名 + ZIP 包内容，不依赖扩展名
4. **高性能**：纯 Rust，普通文档毫秒级转换

### 非目标

- 不做 Markdown → 文档的逆向转换
- 不处理音频/视频内嵌内容
- 不追求像素级还原（以可读性和 LLM 友好为主）
- 第一版不做独立 OCR 服务、不做 GPU（见 §7 未来方向）

---

## 2. 背景：为什么 fork anydoc

对上游 [firecrawl/anydoc](https://github.com/firecrawl/anydoc) v0.1.7 源码的调查结论（2026-08-10）：

- anydoc 已实现全部格式解析和二进制签名检测，代码质量高（无 unsafe、完整错误处理、有测试），MIT 协议
- **但故意不提供任何 OCR 扩展点**：`src/formats/pdf.rs` 对扫描 PDF 直接报 `Unsupported`；所有公开 API（Rust/Node/Python）均无 OCR 参数；CLI 无 `--ocr`
- 这是其商业模式：开源库做纯文本提取，OCR 留给付费托管服务 Firecrawl Parse

因此**无法通过"实现一个符合接口约定的外部 OCR 服务"来集成，必须 fork 修改源码**。fork 成本低：新增约 500–1000 行，修改点明确（§4）。

许可证兼容性已核实：anydoc (MIT) + rust-paddle-ocr (Apache-2.0) + PP-OCRv5 模型 (Apache-2.0)，可以安全集成。

---

## 3. 核心决策：嵌入式 OCR

第一版采用**进程内嵌入式**方案：`ocr-rs` crate（[rust-paddle-ocr](https://github.com/zibo-chen/rust-paddle-ocr) 的 crates.io 发布名，2.4.x）+ PP-OCRv5-FP16 模型，MNN 推理，CPU-only。

> 注：文档早期草稿曾设想独立 HTTP OCR 服务（GPU + 函数计算），已在选型中被嵌入式方案否决，仅在 §7 保留为未来方向。另注：该仓库的 `main` 分支（crate 名 `rust-paddle-ocr`，1.4.x）在 Linux x86_64 上有 MNN 张量拷贝 bug（NC4HW4 对齐问题），且 `Det`/`Rec` 非 Send；发布版 `ocr-rs` 2.x（对应其 `next` 分支，vendored MNN）已验证可用，选型以其为准。

### 选型依据

PP-OCRv5-FP16 实测（Mac Mini M4，CPU）：~870ms/页，峰值内存 388MB，模型体积减半（共约 20MB），精度与标准版相同、比 PP-OCRv4 提升 13%。

| 方案 | 精度 | CPU 延迟/页 | 内存 | 部署 | 结论 |
|------|------|------------|------|------|------|
| **PP-OCRv5-FP16** | 高 | ~870ms | 388MB | 嵌入 | **采用** |
| PaddleOCR-VL (0.9B) | SOTA | ~260ms (OpenVINO) | 2GB | 嵌入/服务 | 未来高精度模式 |
| olmOCR-2-7B | 94% | N/A | 14GB | GPU 服务 | 未来可选 |
| Tesseract 5.x | 中 | ~300-500ms | 500MB | 嵌入 | 备选 fallback |
| GPT-5 等 API | 95% | ~2000ms | - | 云 API | 不用（隐私/成本） |

### 为什么不用独立 OCR 服务

嵌入式优势：零运维、无网络开销、单二进制分发、无需 GPU 服务器、隐私友好。

独立服务（HTTP API + GPU + 函数计算）仅在以下场景才需要，列入未来方向（§7）：>100 并发横向扩展、需要 GPU 级大模型（PaddleOCR-VL / olmOCR）、热更新模型、多租户 SaaS。

---

## 4. 总体架构

```
┌─────────────────────────────────────────────────┐
│        any2md（anydoc fork，单进程）             │
│                                                 │
│  1. 格式检测（已有）   二进制签名 / OLE 流 / ZIP │
│  2. 文档解析（已有）   各格式 parser → Document  │
│  3. OCR 决策（新增）   strategy.rs:              │
│       - PDF: 信任 pdf-inspector 的 pages_needing_ocr │
│       - Office: is_document_scan() 启发式        │
│  4. OCR 引擎（新增）   backend.rs trait +        │
│       embedded.rs（ocr-rs + PP-OCRv5-FP16）      │
│  5. Markdown 渲染（已有） GFM 输出               │
└─────────────────────────────────────────────────┘
```

仓库布局：代码直接在仓库根（即 fork 本身），不嵌套子目录。库 crate 名沿用 `anydoc`，CLI 二进制名 `any2md`。

### Fork 修改点清单

**新增文件**：

```
src/ocr/
├── mod.rs           # 模块导出
├── backend.rs       # OcrBackend trait + OcrOptions/OcrResult/OcrError
├── embedded.rs      # EmbeddedOcrBackend（ocr-rs 集成）
└── strategy.rs      # OcrStrategy + is_document_scan() + BlockContext
src/bin/any2md.rs    # CLI
tests/integration_ocr.rs
benches/ocr_performance.rs
models/              # 模型文件（git-ignored，scripts/download-models.sh 下载）
scripts/
├── download-models.sh      # 模型下载
└── make-ocr-fixtures.py    # 测试夹具生成
```

**修改文件**：

```
src/lib.rs           # pub mod ocr + to_markdown*_with_ocr API
src/formats/pdf.rs   # to_markdown_with_ocr()
Cargo.toml           # 新增依赖
```

---

## 5. 模块设计

### 5.1 OCR 抽象层（`src/ocr/backend.rs`）

```rust
pub trait OcrBackend: Send + Sync {
    fn recognize(&self, image: &[u8], options: &OcrOptions) -> Result<OcrResult, OcrError>;
    fn recognize_batch(&self, images: &[&[u8]], options: &OcrOptions) -> Result<Vec<OcrResult>, OcrError> {
        images.iter().map(|img| self.recognize(img, options)).collect()
    }
    fn health_check(&self) -> Result<(), OcrError>;
}

pub struct OcrOptions {
    pub language: String,              // zh / en / ja / multi，默认 zh
    pub detect_orientation: bool,      // 默认 true
    pub max_size: Option<(u32, u32)>,  // 默认 Some((4096, 4096))，超限自动缩放
}

pub struct OcrResult {
    pub text: String,
    pub confidence: f32,        // 0.0–1.0 平均置信度
    pub boxes: Vec<BoundingBox>,
}

pub enum OcrError {
    InitFailed(String),
    InvalidImage(String),
    RecognitionFailed(String),
}
```

### 5.2 嵌入式后端（`src/ocr/embedded.rs`）

`EmbeddedOcrBackend` 直接包装 `ocr_rs::OcrEngine`（`Send + Sync`，`&self` 入口）：

- `from_model_dir(dir)` / `from_model_dir_with_threads(dir, n)` / `from_files(det, rec, keys, threads)`
- `recognize()`：解码图片（image crate）→ 超限缩放 → `engine.recognize()` 一次调用返回文本框 + 置信度 → 按阅读顺序排序（先 Y 后 X，同行容差 20px）→ 拼接文本 + 平均置信度

### 5.3 OCR 决策（`src/ocr/strategy.rs`）

```rust
pub enum OcrStrategy { Conservative, Aggressive, Disabled }

pub fn needs_ocr(block: &Block, assets: &[Asset], strategy: OcrStrategy, context: &BlockContext) -> bool
```

`is_document_scan()` 启发式（排除法 + 尺寸特征）：

1. 排除：非 image/* 类型；表格内的图（图表/Logo）；conservative 模式下排除同段有文本的 inline 插图
2. 尺寸特征：短边/长边在 0.68–0.80（A4/B5 ≈ 0.707，Legal ≈ 0.72，Letter ≈ 0.77；横竖版均可），且长边 ≥ 阈值
3. 阈值：conservative 长边 ≥ 1500px，aggressive ≥ 1200px
4. 已知误报：大尺寸 4:3 照片（0.75）落在纸张区间内，靠 conservative 的 inline 排除缓解

### 5.4 PDF 集成（`src/formats/pdf.rs`）

原 `to_markdown()` 保持不变（向后兼容）。新增：

```rust
pub fn to_markdown_with_ocr(bytes: &[u8], ocr: Option<&dyn OcrBackend>) -> Result<String, ConvertError>
```

逻辑：

- 快速路径与上游一致：`pdf_inspector::process_pdf_mem()`，无 OCR 页或未给 backend 时行为不变（部分页缺文本 warn 降级，整篇无文本报错提示需要 `--ocr`）
- OCR 路径：用 `pdf_inspector::extract_pages_markdown_mem()` 拿**逐页**结果，文本页直接用其 Markdown，OCR 页渲染成图后识别，**按页序拼接**（混合 PDF 不乱序）；OCR 失败的页 warn 降级回原始提取结果

PDF 页渲染用 **hayro**（纯 Rust、无 unsafe、无外部二进制依赖；已否决需要外挂 libpdfium 的 pdfium-render），渲染比例 3×（≈216 DPI）。

### 5.5 Office 集成（`src/lib.rs`）

```rust
pub fn to_markdown_with_ocr(path, ocr: Option<&dyn OcrBackend>, strategy: OcrStrategy) -> Result<String, ConvertError>
pub fn to_markdown_bytes_with_ocr(bytes, format, ocr, strategy) -> Result<String, ConvertError>
```

DOC/DOCX/PPT/PPTX：正常解析为 `Document` → `ocr::apply_to_document()` 递归遍历 blocks（含表格单元格、列表、引用、批注内的嵌套结构）→ 对 `is_document_scan()` 命中的 `Inline::Image` 调用 OCR → **识别文本写入图片的 alt**（Markdown 中图片即以 alt 呈现），原图字节保留在 `Document::assets`。OCR 失败的图片 warn 降级、不影响转换。Excel/CSV 等不走 OCR。

### 5.6 CLI（`src/bin/any2md.rs`）

```
any2md <INPUT> [-o OUTPUT] [-f FORMAT] [--ocr] [--ocr-strategy S]
       [--ocr-models DIR] [--ocr-threads N] [--detect] [-v...]
```

- 默认不启用 OCR：纯文本路径零额外开销，也不需要模型文件
- `--detect`：只输出检测到的格式
- `-v` 计数式日志级别（默认 warn）

---

## 6. 模型文件管理

模型共约 11MB（FP16），不进 git（`.gitignore` 排除 `models/*.mnn` / `models/*.txt`）。获取方式（按优先级）：

1. `--ocr-models <dir>` 显式指定
2. `./models/` 默认目录；`scripts/download-models.sh` 从 rust-paddle-ocr 仓库下载三件套（模型是该仓库维护的 PP-OCRv5 官方 MNN FP16 转换版）

（早期方案的 build.rs 编译时自动下载已否决：库的普通构建不应依赖网络。）

---

## 7. 未来方向（不在第一版范围）

- **独立 OCR 服务**：GPU + HTTP API（axum）+ 阿里云函数计算，用于高并发/多租户；any2md 侧已有 `OcrBackend` trait，加一个 `HttpOcrBackend` 实现即可
- **高精度模式**：PaddleOCR-VL (0.9B) 或 olmOCR-2-7B
- **表格 OCR**：PPStructure
- **OCR 结果缓存**：图片 SHA256 → 结果，跨文件去重
- **WASM 端 OCR**：浏览器内推理

---

## 8. 测试策略

- **单元测试**：`strategy.rs` 启发式、`embedded.rs` 识别（中文/英文 fixture 图片）
- **集成测试**：`tests/integration_ocr.rs` —— 纯文本 PDF（快速路径不触发 OCR）、扫描 PDF、混合 PDF、含扫描图的 DOCX；未启用 OCR 时的降级/报错行为
- **性能测试**：`benches/ocr_performance.rs` + hyperfine

验收指标：格式检测准确率 > 99%；OCR 中文准确率 > 95%；转换成功率 > 98%；无 panic。

性能目标：DOCX < 50ms；文本 PDF < 100ms；OCR < 1s/页（CPU，4 核）；10 页扫描 PDF < 5s。

---

## 9. 风险与缓解

| 风险 | 概率 | 影响 | 缓解 |
|------|------|------|------|
| ocr-rs 无法编译/不可用 | 低 | 高 | Phase 1 最先验证；备用：ONNX Runtime 或 tesseract-rs |
| 模型下载失败 | 中 | 中 | 多镜像地址；支持手动下载 + `--ocr-models` 指定 |
| anydoc 上游大改 | 低 | 中 | 小步提交便于 rebase；每周检查上游 |
| OCR 精度不足 | 中 | 低 | 原图保留在 assets；策略可调；未来可换 VL 模型 |
| 图文混排误判 | 中 | 中 | 默认 conservative；可切 aggressive |

---

## 10. 参考资料

- [firecrawl/anydoc](https://github.com/firecrawl/anydoc) — 文档解析基础（本仓库的上游）
- [firecrawl/pdf-inspector](https://github.com/firecrawl/pdf-inspector) — PDF 分类与提取
- [zibo-chen/rust-paddle-ocr](https://github.com/zibo-chen/rust-paddle-ocr) — OCR 引擎（crate: `ocr-rs`）
- [PaddleOCR PP-OCRv5](https://paddlepaddle.github.io/PaddleOCR/main/en/version3.x/algorithm/PP-OCRv5/PP-OCRv5.html) — 模型
- [TimmyOVO/deepseek-ocr.rs](https://github.com/TimmyOVO/deepseek-ocr.rs) — 未来独立服务的候选框架
