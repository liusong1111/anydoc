# any2md

高性能文档转 Markdown 工具（Rust），基于 [firecrawl/anydoc](https://github.com/firecrawl/anydoc) fork 扩展，在其全格式解析能力之上集成了**本地 OCR**：扫描 PDF、文档内嵌的扫描图也能转出文字，数据不出本机。

## 功能

- **全格式支持**：DOCX / DOC / PPTX / PPT / XLSX / XLS / ODT / ODS / ODP / RTF / EPUB / CSV / PDF（文本型 + 扫描型）
- **精确格式识别**：基于二进制签名、OLE 流名、ZIP 包内容检测，不依赖扩展名；支持加密文件检测
- **可选本地 OCR**（`--ocr`）：PP-OCRv5-FP16 模型，进程内推理（MNN），无需 GPU、无需外部服务
- **智能判断**：自动识别扫描 PDF 页；Office 文档中用启发式区分"扫描图"与"插图"，只对前者做 OCR
- **输出 GFM Markdown**：表格、列表、标题结构保留，面向 LLM 友好
- **多语言绑定**：Node.js / Python / WebAssembly（继承自上游）

## 构建

```bash
# 开发：本机构建
cargo build --release        # 生成 target/release/any2md（glibc 动态链接）

# 发布：musl 全静态二进制 + tar.gz 发行包（推荐，见下）
just build                   # → dist/any2md/（static-pie，零动态依赖）
just package                 # → any2md-linux-x86_64-<tag>.tar.gz
```

musl 静态构建的额外依赖（仅 `just build` 需要）：

1. `rustup target add x86_64-unknown-linux-musl`
2. musl C++ 交叉工具链（ocr-rs 要用 g++ 编译 MNN 源码；Ubuntu 的 musl-tools 只有 C 编译器，不够）：
   ```bash
   curl -L https://musl.cc/x86_64-linux-musl-cross.tgz | tar xz -C ~/.local/share/
   ```
   默认路径 `~/.local/share/x86_64-linux-musl-cross`，不同则改 Justfile 里的 `musl_toolchain` 变量。

> 注：musl 构建需要两个链接期补丁，已内置在仓库里：`src/ocr/musl_fortify_shim.c`
> 提供 musl 缺失的 glibc `__*_chk` / `__libc_single_threaded` 符号（build.rs
> 仅在 musl target 编译它），Justfile 里的 target 级 RUSTFLAGS 处理链接顺序
> 和静态 libstdc++。普通 `cargo build`（glibc）完全不经过这些。

## Docker

```bash
just docker-build            # 构建镜像（基于 alpine，内含静态二进制 + OCR 模型）
just docker-push             # 推送到 registry（见 Justfile image_name 变量）

# 使用：把文件目录挂到 /data，用绝对路径读写
docker run --rm -v "$PWD:/data" <image> /data/scan.pdf --ocr -o /data/out.md
```

## 使用

```bash
# 基础转换（输出到 stdout）
any2md report.docx
any2md report.docx -o report.md

# 启用 OCR（扫描件）
any2md scan.pdf --ocr -o output.md
any2md mixed.docx --ocr --ocr-strategy conservative

# 只检测格式，不转换
any2md --detect unknown.bin
```

OCR 相关参数：

| 参数 | 说明 | 默认值 |
|------|------|--------|
| `--ocr` | 启用 OCR | 关 |
| `--ocr-strategy` | `conservative` / `aggressive` | `conservative` |
| `--ocr-models <dir>` | 模型目录 | `./models` |
| `--ocr-threads <N>` | OCR 推理线程数 | 引擎自动 |

## OCR 模型

OCR 需要 PP-OCRv5-FP16 模型文件（共约 11MB），放在 `models/` 目录：

- `PP-OCRv5_mobile_det_fp16.mnn`（检测）
- `PP-OCRv5_mobile_rec_fp16.mnn`（识别）
- `ppocr_keys_v5.txt`（字典）

模型不进 git。下载方式：

```bash
scripts/download-models.sh        # 下载到 ./models
scripts/download-models.sh /path  # 或指定目录，配合 --ocr-models 使用
```

未启用 `--ocr` 时不需要模型。

## 已知限制

- MNN 推理库在初始化时会向 **stdout** 打印 CPU 拓扑信息（`CPU Group: ...`），无法从库层面关闭。管道使用 Markdown 输出时建议用 `-o` 写文件，或过滤这些行。

## 作为库使用

```rust
// 普通转换（自动检测格式）
let markdown = anydoc::to_markdown("report.docx")?;

// 带 OCR 的转换
let ocr = anydoc::ocr::EmbeddedOcrBackend::from_model_dir("models")?;
let markdown = anydoc::to_markdown_bytes_with_ocr(
    &bytes, None, Some(&ocr), anydoc::ocr::OcrStrategy::Conservative,
)?;
```

> 注：库 crate 名沿用上游的 `anydoc`，以减少追踪上游时的合并摩擦；CLI 二进制名为 `any2md`。

## 文档

- [DESIGN.md](DESIGN.md) — 设计文档：架构、选型决策、模块设计
- [IMPLEMENTATION_PLAN.md](IMPLEMENTATION_PLAN.md) — 实施计划：阶段划分与进度

## 许可证

- 本项目代码：MIT（与上游 anydoc 一致）
- OCR 引擎 [rust-paddle-ocr](https://github.com/zibo-chen/rust-paddle-ocr)：Apache-2.0
- PP-OCRv5 模型：Apache-2.0
