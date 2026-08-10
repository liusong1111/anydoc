# OCR 模型文件

OCR 功能（`--ocr`）需要 PP-OCRv5-FP16 模型三件套，本目录是默认加载位置（可用 `--ocr-models` 指定其他目录）：

| 文件 | 用途 | 大小 |
|------|------|------|
| `PP-OCRv5_mobile_det_fp16.mnn` | 文本检测 | ~2.4MB |
| `PP-OCRv5_mobile_rec_fp16.mnn` | 文本识别（中/英/日/拼音） | ~8.4MB |
| `ppocr_keys_v5.txt` | 字符集 | ~74KB |

本目录不进 git（见 `.gitignore`）。获取方式：

```bash
scripts/download-models.sh        # 下载到 ./models
scripts/download-models.sh /path  # 下载到指定目录
```

模型来源：[zibo-chen/rust-paddle-ocr](https://github.com/zibo-chen/rust-paddle-ocr) 仓库的 `models/` 目录（PP-OCRv5 官方模型的 MNN FP16 转换版，Apache-2.0）。
