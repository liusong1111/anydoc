// Build script: only active for musl targets, where it compiles the
// _FORTIFY_SOURCE shim that MNN's C++ objects need (musl lacks glibc's
// __*_chk symbols). See src/ocr/musl_fortify_shim.c.

fn main() {
    println!("cargo:rerun-if-changed=src/ocr/musl_fortify_shim.c");
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("musl") {
        cc::Build::new()
            .file("src/ocr/musl_fortify_shim.c")
            .compile("musl_fortify_shim");
        // 注意：这里只发 rustc-link-lib。shim 自身也引用 libc（strncat、
        // vprintf 等），而 -l 顺序在 -lc 之后时会解析失败；最终的 bin 链接
        // 通过 Justfile 里 RUSTFLAGS 的 --start-group/-lc/--end-group 解决。
    }
}
