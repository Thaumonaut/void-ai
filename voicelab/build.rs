fn main() {
    // Android: the sherpa-onnx-sys crate doesn't emit the C++ runtime for the
    // android target, so link libc++_shared.so (provided by the NDK sysroot).
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target_os == "android" {
        println!("cargo:rustc-link-lib=dylib=c++_shared");
    }
}
