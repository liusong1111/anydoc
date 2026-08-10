# any2md 构建 / 打包 / 镜像配方
#
# 构建：
#   just build              # musl 静态 release → dist/any2md/（二进制 + models/）
#   just build-native       # 本机原生（glibc）release
#   just package            # build + 打 tar.gz 发行包
#   just test
#   just clean
#
# 镜像：
#   just docker-build       # build + docker build（打时间戳 tag + latest）
#   just docker-push        # 推送到阿里云 registry
#
# 依赖：rustup target add x86_64-unknown-linux-musl，以及 musl C++ 交叉
# 工具链（ocr-rs 要用它编译 MNN C++ wrapper；见 README「构建」一节）。

set shell := ["bash", "-uc"]

# musl 编译 target
musl_target := "x86_64-unknown-linux-musl"
# musl C++ 交叉工具链（ocr-rs 要用 g++ 编译 MNN 源码里的 C++ wrapper；
# Ubuntu 的 musl-tools 只有 musl-gcc，所以用 musl.cc 的完整工具链）
musl_toolchain := home_directory() / ".local/share/x86_64-linux-musl-cross"
# 产物输出目录
out := "dist"

time_tag := `date +"%Y-%m-%d-%H-%M"`
git_tag := `git describe --always --dirty=-modified`
git_branch := `git rev-parse --abbrev-ref HEAD`
arch := `uname -m`
image_tag := time_tag + "-" + git_branch + "-" + git_tag + "-" + arch
# 镜像仓库：外网 registry（docker push 用）；k8s 集群内用 registry-vpc（inner，更快）
image_name := "registry.cn-shanghai.aliyuncs.com/maim1/any2md"
inner_image_name := "registry-vpc.cn-shanghai.aliyuncs.com/maim1/any2md"

default: build

# 构建 Linux musl 静态 release 二进制，连同模型一起组装到 dist/any2md/
# RUSTFLAGS 说明（用 target 级变量，避免影响 host 侧 build script 链接）：
#   - --start-group 包住 shim 和 -lc：shim（musl_fortify_shim）引用 libc 符号，
#     直接 -l 排在 -lc 后面会解析失败，group 让 ld 重扫
#   - -L target/musl-static：ocr-rs 在 Linux 上总是按动态链接声明 -lstdc++
#     （其 static-cpp-runtime feature 只对 windows-gnu 生效），这里放一个只含
#     libstdc++.a 的目录让 ld 选静态归档；-static-libstdc++/-static-libgcc
#     双保险（前者对显式 -lstdc++ 其实不生效，见上）
build:
    mkdir -p target/musl-static
    ln -sf "{{musl_toolchain}}/x86_64-linux-musl/lib/libstdc++.a" target/musl-static/libstdc++.a
    export PATH="{{musl_toolchain}}/bin:$PATH" && \
    export CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER=x86_64-linux-musl-g++ && \
    export CC_x86_64_unknown_linux_musl=x86_64-linux-musl-gcc && \
    export CXX_x86_64_unknown_linux_musl=x86_64-linux-musl-g++ && \
    export CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_RUSTFLAGS="-C link-arg=-L{{justfile_directory()}}/target/musl-static -C link-arg=-static-libstdc++ -C link-arg=-static-libgcc -C link-arg=-Wl,--start-group -C link-arg=-lmusl_fortify_shim -C link-arg=-lc -C link-arg=-Wl,--end-group" && \
    cargo build --release --target {{musl_target}} --bin any2md
    mkdir -p {{out}}/any2md
    cp target/{{musl_target}}/release/any2md {{out}}/any2md/
    rm -rf {{out}}/any2md/models
    cp -r models {{out}}/any2md/
    cp scripts/download-models.sh {{out}}/any2md/
    @echo "✔ 已产出静态二进制（file 应为 statically linked）："
    @file {{out}}/any2md/any2md
    @ls -lh {{out}}/any2md/

# build 的别名（语义一致，无需各自维护）
build-musl: build

# 本机原生（glibc）release 构建
build-native:
    cargo build --release --bin any2md

# 打 tar.gz 发行包（any2md-linux-x86_64-<tag>.tar.gz）
package: build
    tar czf any2md-linux-{{arch}}-{{git_tag}}.tar.gz -C {{out}} any2md
    @ls -lh any2md-linux-{{arch}}-{{git_tag}}.tar.gz

# 单元 + 集成测试（OCR 用例需要 models/，缺失时自动跳过）
test:
    cargo test

# 静态检查
lint:
    cargo clippy --all-targets
    cargo fmt --check

# 清理构建产物
clean:
    cargo clean
    rm -rf {{out}} any2md-linux-*.tar.gz

# 打镜像（build + docker build + tag latest）
docker-build: build
    set -e
    docker build -t {{ image_name }}:{{ image_tag }} .
    docker tag {{ image_name }}:{{ image_tag }} {{ image_name }}:latest
    @echo {{ image_name }}:{{ image_tag }}

# 推镜像（tag + latest）
docker-push: docker-build
    docker push {{ image_name }}:{{ image_tag }}
    docker push {{ image_name }}:latest
    @echo {{ image_name }}:{{ image_tag }}
