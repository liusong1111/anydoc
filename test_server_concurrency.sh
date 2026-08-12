#!/bin/bash
# 测试 any2md server 并发限流功能

set -e

PORT=8766
SERVER_PID=""

# 清理函数
cleanup() {
    if [ -n "$SERVER_PID" ]; then
        echo "停止服务器 (PID: $SERVER_PID)..."
        kill $SERVER_PID 2>/dev/null || true
    fi
}

trap cleanup EXIT

# 启动服务器
echo "启动 any2md server..."
cargo run --quiet --features server -- server -p $PORT &
SERVER_PID=$!

# 等待服务器启动
echo "等待服务器启动..."
sleep 3

# 检查服务器是否启动
if ! curl -s http://localhost:$PORT/metrics >/dev/null; then
    echo "错误：服务器启动失败"
    exit 1
fi

echo "服务器启动成功 (PID: $SERVER_PID)"
echo ""

# 查看初始 metrics
echo "=== 初始 Metrics ==="
curl -s http://localhost:$PORT/metrics
echo ""

# 创建测试文件
TEST_FILE=$(mktemp --suffix=.txt)
echo "这是一个测试文档，用于验证并发限流功能。" > $TEST_FILE
echo "测试文件: $TEST_FILE"
echo ""

# 发送 10 个并发请求
echo "=== 发送 10 个并发请求 ==="
for i in {1..10}; do
    (
        curl -s -X POST http://localhost:$PORT/v2/any2md \
            -F "file=@$TEST_FILE" \
            -F "ocr=false" \
            -H "Accept: application/json" \
            > /dev/null &
        echo "请求 $i 已发送"
    )
done

# 等待 1 秒，查看排队情况
sleep 1
echo ""
echo "=== 处理中的 Metrics (1秒后) ==="
curl -s http://localhost:$PORT/metrics
echo ""

# 等待所有请求完成
sleep 5

echo "=== 最终 Metrics ==="
curl -s http://localhost:$PORT/metrics
echo ""

# 清理
rm -f $TEST_FILE

echo "测试完成！"
