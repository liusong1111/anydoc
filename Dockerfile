# any2md 运行时镜像（alpine 基础镜像——any2md 是 musl 静态二进制，无 glibc 依赖）
#
# 构建前置（由 Justfile `just docker-build` 完成）：
#   - dist/any2md/any2md：`just build` 产出的 musl 静态二进制
#   - models/：PP-OCRv5-FP16 模型（scripts/download-models.sh 下载）
#
# 运行约定：
#   - WORKDIR=/app，OCR 模型在 /app/models（CLI 默认模型路径即 ./models）
#   - 要转换的文件挂载到 /data，用绝对路径读写，例如：
#       docker run --rm -v "$PWD:/data" <image> /data/scan.pdf --ocr -o /data/out.md
#   - 注意 MNN 初始化会向 stdout 打印 CPU 拓扑信息，管道使用请加 -o 写文件

FROM alpine:3.21

RUN apk add --no-cache ca-certificates tzdata
ENV TZ=Asia/Shanghai
ENV NO_COLOR=1

WORKDIR /app

COPY dist/any2md/any2md /usr/local/bin/any2md
COPY models/ /app/models/

RUN mkdir -p /data
VOLUME ["/data"]

ENTRYPOINT ["any2md"]
CMD ["--help"]
