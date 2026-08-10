# any2md 实施计划

> 执行中的任务清单。设计依据见 [DESIGN.md](DESIGN.md)。
> 状态：全部 Phase 已完成并验证（2026-08-10）。

## 已确认的决策

1. **项目名** `any2md`；库 crate 名沿用 `anydoc`（减少上游合并摩擦），CLI 二进制名 `any2md`
2. **代码直接在仓库根**（本仓库即 fork），不嵌套子目录；`upstream` remote 指向 firecrawl/anydoc
3. **第一版不做**独立 OCR 服务、Docker 镜像、GPU 支持（见 DESIGN.md §7）
4. **GitHub fork 暂缓**：本地 `feature/ocr-integration` 分支开发，需要发布时再建远程 fork
5. **OCR 方案**：嵌入式 `ocr-rs = "2.4"`（crates.io，即 rust-paddle-ocr 的发布名）+ PP-OCRv5-FP16。
   早期设想的 git main 分支依赖已否决：1.4.x 在 Linux x86_64 存在 MNN 张量拷贝 bug，且 `Det`/`Rec` 非 Send
6. **PDF 页渲染用 hayro**（纯 Rust）；pdfium-render 需要外挂 libpdfium 二进制，否决
7. **模型不经 build.rs 下载**（库构建不应依赖网络），改为 `scripts/download-models.sh`

---

## Phase 0: 仓库准备 ✅

- [x] anydoc v0.1.7 源码（含上游 git 历史）置于仓库根
- [x] remote 整理：`upstream` 指向 firecrawl/anydoc
- [x] `.gitignore` 增加 `models/*.mnn`、`models/*.txt`、`/references/`、`/.claude/`
- [x] 基线验证：`cargo build` + `cargo test` 全绿
- [x] 开发分支 `feature/ocr-integration`，文档已提交

## Phase 1: OCR 核心 ✅

- [x] `src/ocr/backend.rs`：`OcrBackend` trait / `OcrOptions` / `OcrResult` / `BoundingBox` / `OcrError`
- [x] `src/ocr/embedded.rs`：`EmbeddedOcrBackend`（直接包装 `ocr_rs::OcrEngine`，Send+Sync）；
      阅读顺序排序（同行容差 20px）、平均置信度、超限图片自动缩放
- [x] `models/` 三件套就位（PP-OCRv5 FP16 det/rec + keys，共 ~11MB）
- [x] `src/formats/pdf.rs`：`to_markdown_with_ocr()`，hayro 渲染（3×，≈216 DPI），
      逐页按序拼接；快速路径与上游行为一致
- [x] `src/lib.rs`：`pub mod ocr` + `to_markdown_with_ocr()` / `to_markdown_bytes_with_ocr()`

## Phase 2: CLI + 模型管理 + 集成测试 ✅

- [x] `src/bin/any2md.rs`：`--ocr` / `--ocr-strategy` / `--ocr-models` / `--ocr-threads` / `--detect` / `-v`
- [x] `scripts/download-models.sh` + `models/README.md`
- [x] `scripts/make-ocr-fixtures.py` 生成测试夹具（`tests/fixtures-ocr/`，
      有意避开上游 snapshots/robustness 扫描的 `tests/fixtures/`）
- [x] `tests/integration_ocr.rs`：6 个端到端测试全过（模型缺失时自动跳过 OCR 用例）：
      纯文本 PDF 快速路径 / 扫描 PDF 无 OCR 报错 / 扫描 PDF OCR / 混合 PDF 页序 /
      DOCX 扫描图 OCR 进 alt / Disabled 策略不 OCR
- [x] 已知限制记录：MNN 初始化向 stdout 打印 CPU 拓扑（库层面无法关闭）

## Phase 3: Office 格式 OCR ✅

- [x] `src/ocr/strategy.rs`：`OcrStrategy` / `BlockContext` / `is_document_scan()`
      （纸张比例 0.68–0.80、长边 ≥1200/1500、表格与 inline 排除）+ 5 个单元测试
- [x] `apply_to_document()` 递归遍历（段落/标题/列表/表格/引用/批注），
      OCR 结果写入图片 alt，原图保留在 assets，单图失败 warn 降级
- [x] 端到端验证：`scan_image.docx --ocr` 输出 OCR 文本；不加 `--ocr` 不输出

## Phase 4: 优化与收尾 ✅

- [x] release 构建 + 性能实测（24 核服务器，release 二进制）：
  - DOCX 转换 <10ms（目标 <50ms）✅
  - 文本 PDF <10ms（目标 <100ms）✅
  - 扫描 PDF 含 OCR 0.22s/页（含模型加载，目标 <500ms/页）✅
  - OCR 峰值内存 ~313MB ✅
- [x] 文档最终核对：README/DESIGN/PLAN 与实际行为一致
- [x] `cargo clippy --all-targets` 0 警告 / `cargo fmt --check` 通过
- [x] 全量测试：213 lib + 6 OCR 集成 + 上游 corpus 快照/健壮性测试全绿
- 跳过项及理由：`benches/` criterion 基准（集成测试 + 实测计时已覆盖 v1 需要）；
  `recognize_batch` 并行化（OcrEngine 内部已多线程，v1 无批量场景）

---

## 依赖清单（实际）

```toml
[dependencies]
ocr-rs = "2.4"          # PP-OCRv5 + MNN（vendored，预编译库自动下载）
image = "0.25"          # 图片解码/尺寸/缩放
hayro = "0.7"           # PDF 页渲染（纯 Rust）
clap = { version = "4.5", features = ["derive"] }   # CLI
env_logger = "0.11"     # CLI 日志
```

## 里程碑

- **M1（Phase 0–2）✅**：扫描 PDF 可用 CLI 转 Markdown，测试通过
- **M2（Phase 3）✅**：Office 扫描图 OCR 可用
- **M3（Phase 4）✅**：性能达标，文档齐全；可发布 release（待建 GitHub fork 后推送）
