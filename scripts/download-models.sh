#!/usr/bin/env bash
# Download the PP-OCRv5-FP16 model set for any2md --ocr.
# Usage: scripts/download-models.sh [target-dir]   (default: ./models)
set -euo pipefail

TARGET="${1:-models}"
BASE="https://raw.githubusercontent.com/zibo-chen/rust-paddle-ocr/main/models"
FILES=(PP-OCRv5_mobile_det_fp16.mnn PP-OCRv5_mobile_rec_fp16.mnn ppocr_keys_v5.txt)

mkdir -p "$TARGET"
for f in "${FILES[@]}"; do
    if [ -f "$TARGET/$f" ]; then
        echo "exists: $TARGET/$f"
    else
        echo "downloading: $f"
        curl -fL --retry 3 -o "$TARGET/$f" "$BASE/$f"
    fi
done
echo "done. models in $TARGET/"
