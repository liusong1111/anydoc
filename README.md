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

aarch64 交叉构建（产物隔离在 `dist-aarch64/`，镜像用 `Dockerfile.aarch64` + buildx）：

```bash
rustup target add aarch64-unknown-linux-musl
curl -L https://musl.cc/aarch64-linux-musl-cross.tgz | tar xz -C ~/.local/share/
just -f Justfile.aarch64 build          # 交叉编译静态二进制
just -f Justfile.aarch64 docker-build   # buildx --platform linux/arm64 打镜像
```

> 注：musl 构建需要两个链接期补丁，已内置在仓库里：`src/ocr/musl_fortify_shim.c`
> 提供 musl 缺失的 glibc `__*_chk` / `__libc_single_threaded` 符号（build.rs
> 仅在 musl target 编译它），Justfile 里的 target 级 RUSTFLAGS 处理链接顺序
> 和静态 libstdc++。普通 `cargo build`（glibc）完全不经过这些。

## Docker

```bash
just docker-build            # 构建镜像（基于 alpine，内含静态二进制 + OCR 模型）
just docker-push             # 推送到 registry（见 Justfile image_name 变量）
just deploy-dev              # push + 更新 aijoy3-dev 命名空间下的 any2md Deployment
                             # （首次部署先 kubectl apply -f k8s/ -n aijoy3-dev）

# 使用：把文件目录挂到 /data，用绝对路径读写
docker run --rm -v "$PWD:/data" <image> /data/scan.pdf --ocr -o /data/out.md

# server 模式：
docker run --rm -p 8766:8766 -v "$PWD:/data" <image> server
```

## 使用

```bash
# 基础转换（输出到 stdout）
any2md report.docx
any2md report.docx -o report.md

# 导出内嵌大插图到 img/，markdown 里以 ![](img/image-1.png) 引用
any2md report.docx -o report.md --images-dir img

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
| `--ocr-strategy` | `disabled` / `conservative` / `smart` / `aggressive` | `smart` |
| `--ocr-models <dir>` | 模型目录 | `./models` |
| `--ocr-threads <N>` | OCR 推理线程数 | 引擎自动 |

图片导出：

| 参数 | 说明 | 默认值 |
|------|------|--------|
| `--images-dir <dir>` | 把内嵌大插图导出到该目录（需配合 `-o`） | 不导出 |

- `--images-dir` 只在带 `-o` 时生效，图片写到 `<输出文件同目录>/<dir>/`，引用为 `dir/image-N.png`。
- 只导出「大插图」：`长边 ≥ 400` 且 `短边 ≥ 200`，排除细条分隔线（短/长 < 0.1）与行内小图标。装饰性小图不导出也不引用。
- 开启 OCR 时，被 OCR 转成文字的扫描图不会作为图片导出，而是直接输出文字。

## HTTP API 服务

`server` 子命令把转换能力暴露为 HTTP API（接口形态与 any2text 的 api_server 一致）：

```bash
any2md server                        # 监听 0.0.0.0:8766，默认加载 OCR 模型
any2md server --port 9000            # 换端口
any2md server --no-ocr               # 不加载模型；请求 OCR 时返回错误
any2md --ocr-models /path server     # 模型不在 ./models 时
```

端点：`POST /v2/any2md`,multipart 表单：

| 字段 | 说明 |
|------|------|
| `file` | 上传的文件内容（与 `path` 二选一） |
| `path` | 服务器本地文件路径 |
| `ocr` | `false` / `0` / `no` / `off` 关闭 OCR；**缺省为开** |

响应：`Accept: text/plain` 时返回纯 Markdown 文本；否则返回 JSON：

```json
{"code": 200, "message": "ok", "data": {"file": "scan.pdf", "full_text": "...", "ocr": true}}
```

JSON 模式下参数错误同样返回 HTTP 200，由 `code: 400` 携带错误（与 any2text 客户端契约一致）；`text/plain` 模式返回真实 HTTP 状态码。

```bash
# 上传转换（默认开 OCR）
curl -F file=@scan.pdf http://localhost:8766/v2/any2md

# 关闭 OCR，只要纯文本
curl -F file=@report.docx -F ocr=false -H 'Accept: text/plain' http://localhost:8766/v2/any2md

# 服务器本地文件
curl -F path=/data/scan.pdf http://localhost:8766/v2/any2md
```

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
