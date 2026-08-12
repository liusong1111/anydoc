#!/bin/bash
# 测试 any2md server OCR 并发限流

set -e

PORT=8767
SERVER_PID=""

cleanup() {
    if [ -n "$SERVER_PID" ]; then
        echo "停止服务器..."
        kill $SERVER_PID 2>/dev/null || true
        wait $SERVER_PID 2>/dev/null || true
    fi
}

trap cleanup EXIT

echo "启动 any2md server..."
cargo run --quiet --features server -- server -p $PORT 2>&1 | grep -E "listening|concurrency" &
SERVER_PID=$!

sleep 4

echo ""
echo "=== 初始 Metrics ==="
curl -s http://localhost:$PORT/metrics | grep -E "any2md_"
echo ""

echo "发送 5 个并发 OCR 请求（使用实际图片文档）..."

# 使用你的教案文档
if [ -f ~/文档/图片教案.docx ]; then
    DOC=~/文档/图片教案.docx
else
    echo "找不到测试文档，跳过"
    exit 0
fi

for i in {1..5}; do
    (
        echo "发送请求 $i..."
        curl -s -X POST http://localhost:$PORT/v2/any2md \
            -F "file=@$DOC" \
            -F "ocr=true" \
            -H "Accept: text/plain" \
            > /tmp/result_$i.txt 2>&1 &
    ) &
done

sleep 2
echo ""
echo "=== 处理中 Metrics (2秒后) ==="
curl -s http://localhost:$PORT/metrics | grep -E "any2md_"

sleep 15
echo ""
echo "=== 最终 Metrics (15秒后) ==="
curl -s http://localhost:$PORT/metrics | grep -E "any2md_"

rm -f /tmp/result_*.txt
echo ""
echo "测试完成！"
