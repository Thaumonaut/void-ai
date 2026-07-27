//! iOS: hand a URL off to the system (Maps app / browser) via UIApplication.openURL — the
//! iOS analogue of the Android ACTION_VIEW intent in `native_stt::launch_url`. Used by the
//! "Open in Maps" (directions) and "Open in browser" (web tabs) hand-offs.
//!
//! UIApplication is main-thread-only; the open-url callbacks are Slint callbacks, which run
//! on the main thread, so this is safe from there.

use objc2::{msg_send, MainThreadMarker};
use objc2_foundation::{NSString, NSURL};
use objc2_ui_kit::UIApplication;

/// Open `url` in whatever app the system routes it to (Safari, Maps, …). Best-effort.
pub fn launch_url(url: &str) {
    let Some(mtm) = MainThreadMarker::new() else {
        eprintln!("[ios-url] launch_url called off the main thread");
        return;
    };
    unsafe {
        let ns = NSString::from_str(url);
        let nsurl = match NSURL::URLWithString(&ns) {
            Some(u) => u,
            None => {
                eprintln!("[ios-url] not a valid URL: {url}");
                return;
            }
        };
        let app = UIApplication::sharedApplication(mtm);
        // Deprecated single-arg openURL: — still works on iOS 16 and avoids the
        // options-NSDictionary + completion-block machinery of the modern variant.
        let _: bool = msg_send![&*app, openURL: &*nsurl];
    }
}
