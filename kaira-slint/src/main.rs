//! iOS + desktop entry point. On iOS, Slint's winit backend calls UIApplicationMain
//! internally, so this `main` never returns. Android uses `android_main` in lib.rs
//! (this bin isn't built for Android — cargo apk builds the cdylib).

fn main() {
    if let Err(e) = kaira_slint::run_app() {
        eprintln!("[kaira-slint] {e}");
    }
}
