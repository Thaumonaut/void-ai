fn main() {
    slint_build::compile("ui/app.slint").unwrap();
    // Android: link libc++_shared.so (sherpa-onnx-sys emits no C++ runtime there),
    // plus the prebuilt static libopus (cross-compiled with the NDK) for the
    // in-app Opus streaming test.
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target_os == "android" {
        println!("cargo:rustc-link-lib=dylib=c++_shared");
        let dir = format!("{}/third_party/opus-android", env!("CARGO_MANIFEST_DIR"));
        println!("cargo:rustc-link-search=native={dir}");
        println!("cargo:rustc-link-lib=static=opus");
    } else if target_os == "ios" {
        // Prebuilt static libopus cross-compiled with the iOS SDK (arm64). No
        // c++_shared here — that's an Android/sherpa concern; Talk is C-only (libopus).
        // Device and simulator have different Mach-O platforms (iphoneos vs
        // iphonesimulator), so a device .a won't link into a sim binary even at the
        // same arch — pick the matching prebuilt by the target triple's `-sim` suffix.
        let is_sim = std::env::var("TARGET").unwrap_or_default().ends_with("-sim");
        let sub = if is_sim { "opus-ios-sim" } else { "opus-ios" };
        let dir = format!("{}/third_party/{sub}", env!("CARGO_MANIFEST_DIR"));
        println!("cargo:rustc-link-search=native={dir}");
        println!("cargo:rustc-link-lib=static=opus");
        // WebKit for the in-app WKWebView sub-window (see src/ios_webview.rs). Linking
        // the framework makes the WKWebView/WKWebViewConfiguration ObjC classes resolvable
        // at runtime via class!(); without it class!(WKWebView) would return nil → crash.
        println!("cargo:rustc-link-lib=framework=WebKit");
        // PhotosUI for the photo picker (see src/ios_photos.rs) — same story: without it
        // class!(PHPickerViewController)/class!(PHPickerConfiguration) can't resolve → crash.
        println!("cargo:rustc-link-lib=framework=PhotosUI");
    }
}
