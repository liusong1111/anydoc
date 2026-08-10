# any2md 实施计划

> 执行中的任务清单。设计依据见 [DESIGN.md](DESIGN.md)。

## 已确认的决策

1. **项目名** `any2md`；库 crate 名沿用 `anydoc`（减少上游合并摩擦），CLI 二进制名 `any2md`
2. **代码直接在仓库根**（本仓库即 fork），不嵌套 `forked-anydoc/` 子目录
3. **第一版不做**独立 OCR 服务、Docker 镜像、GPU 支持（见 DESIGN.md §7）
4. **GitHub fork 暂缓**：本地 `feature/ocr-integration` 分支开发，需要发布时再建远程 fork
5. **OCR 方案**：嵌入式 ocr-rs + PP-OCRv5-FP16（最终决策，取代早期的独立服务方案）

---

## Phase 0: 仓库准备 ✅（已完成）

- [x] 获取 anydoc v0.1.7 源码（含上游 git 历史），置于仓库根
- [x] remote 整理：`origin` 改为 `upstream` 指向 firecrawl/anydoc
- [x] `.gitignore` 增加 `/models/`、`.claude/`、`references/`
- [x] 基线验证：`cargo build` + `cargo test` 全绿（8 passed）
- [ ] 创建开发分支 `feature/ocr-integration` 并提交文档

## Phase 1: OCR 核心（3 天）

### 1.1 OCR 抽象层 — `src/ocr/backend.rs`（新增）

- [ ] `OcrBackend` trait / `OcrOptions` / `OcrResult` / `BoundingBox` / `OcrError`
- [ ] `src/ocr/mod.rs` 模块导出；`src/lib.rs` 加 `pub mod ocr`

**验收**：`cargo check` 通过，trait 定义与 DESIGN.md §5.1 一致

### 1.2 嵌入式后端 — `src/ocr/embedded.rs`（新增）

- [ ] `Cargo.toml` 加依赖：`ocr-rs`（git）、`image`、`rayon`、`sha2`、`num_cpus`
- [ ] **先验证 ocr-rs 可编译可用**（风险最高的依赖，失败了立即切备用方案）
- [ ] `EmbeddedOcrBackend`：`from_model_dir()`、阅读顺序排序、置信度聚合、`Arc` 共享
- [ ] 单元测试（中文/英文 fixture）

**验收**：加载 PP-OCRv5-FP16，识别测试图返回文本，测试通过

### 1.3 模型管理 — `build.rs` + `models/`

- [ ] build.rs 自动下载缺失模型（det / rec / keys 三件套）
- [ ] `models/README.md` 说明来源与手动下载方式

### 1.4 PDF 集成 — `src/formats/pdf.rs`

- [ ] 保留原 `to_markdown()`；新增 `to_markdown_with_ocr()`
- [ ] 选定并实现 PDF 页渲染（pdfium-render 或 pdf_oxide，先试编译再定）
- [ ] `src/lib.rs` 新增 `to_markdown_with_ocr()` / `to_markdown_bytes_with_ocr()`

**验收**：纯文本 PDF 走快速路径；扫描 PDF OCR 出文本；混合 PDF 正确合并；未开 OCR 时行为与上游一致

## Phase 2: CLI（1 天）

- [ ] `src/bin/any2md.rs`：`--ocr` / `--ocr-strategy` / `--ocr-models` / `--ocr-threads` / `--detect` / `-v`
- [ ] `tests/integration_ocr.rs`：纯文本/扫描/混合 PDF 端到端
- [ ] 手动验证：
  ```bash
  cargo run --bin any2md -- test.pdf            # 无 OCR 快速路径
  cargo run --bin any2md -- scan.pdf --ocr      # OCR 路径
  ```

## Phase 3: Office 格式 OCR（2 天）

- [ ] `src/ocr/strategy.rs`：`OcrStrategy` / `BlockContext` / `is_document_scan()`（含 PNG/JPEG 尺寸解析）
- [ ] `src/lib.rs` Office 分支：扫描图筛选 → 批量 OCR → 替换为文本段落（原图保留在 assets）
- [ ] 单元测试：启发式各分支；集成测试：含扫描图的 DOCX

**验收**：`any2md mixed.docx --ocr` 只 OCR 扫描图，正常插图不动

## Phase 4: 优化与收尾（1 天）

- [ ] `recognize_batch` rayon 并行
- [ ] `benches/ocr_performance.rs`；hyperfine 验证性能目标（DESIGN.md §8）
- [ ] README/DESIGN 核对：文档描述与实际行为一致
- [ ] clippy / rustfmt 通过

---

## 依赖清单

```toml
[dependencies]
ocr-rs = { git = "https://github.com/zibo-chen/rust-paddle-ocr.git" }
image = "0.25"
rayon = "1.10"
sha2 = "0.11"
num_cpus = "1"
# PDF 页渲染：pdfium-render 或 pdf_oxide（Phase 1.4 定）
# CLI: clap、env_logger

[dev-dependencies]
criterion = "0.5"
```

## 测试数据

需准备：纯文本 PDF、扫描 PDF、混合 PDF 各 3 份左右；含扫描图的 DOCX 2 份；中/英文文字图片 fixture 各 1 张。来源：自制（LibreOffice 导出 / 打印扫描）。

## 里程碑

- **M1（Phase 0–2 完成）**：扫描 PDF 可用 CLI 转 Markdown，测试通过
- **M2（Phase 3 完成）**：Office 扫描图 OCR 可用
- **M3（Phase 4 完成）**：性能达标，文档齐全，可发布 release

**总工作量估算**：5–7 个工作日（1 人全职）。
