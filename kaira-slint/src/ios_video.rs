//! iOS: a YouTube video player — a WKWebView that loads the embed URL for the tapped result,
//! positioned to cover the "videos" view's player region (reported via report-video-geom, like
//! ios_web/ios_map). Inline autoplay is enabled on the configuration so it plays in-place.
//!
//! Video vision (duplex): `snapshot_to_vision` renders the CURRENT player frame with WKWebView's
//! `takeSnapshot` (which captures the composited video pixels), encodes it to JPEG, and pushes it
//! down the same channel as image-vision (`realtime::set_viewing_bytes`) so Nova can answer
//! "what's happening in this?". Driven on an interval by lib.rs while a video is playing.

use std::cell::RefCell;

use base64::Engine as _;
use block2::RcBlock;
use objc2::rc::{Allocated, Retained};
use objc2::runtime::AnyObject;
use objc2::{class, msg_send, MainThreadMarker};
use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use objc2_foundation::{NSString, NSURL};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

extern "C" {
    // UIKit C function: UIImage* → JPEG NSData* (quality 0..1). UIKit is already linked (winit).
    fn UIImageJPEGRepresentation(image: *mut AnyObject, quality: f64) -> *mut AnyObject;
}

thread_local! {
    static PLAYER: RefCell<Option<Retained<AnyObject>>> = const { RefCell::new(None) };
    static PENDING_URL: RefCell<Option<String>> = const { RefCell::new(None) };
    static LOADED_URL: RefCell<Option<String>> = const { RefCell::new(None) };
}

fn host_view(window: &slint::Window) -> Option<*mut AnyObject> {
    MainThreadMarker::new()?;
    let sh = window.window_handle();
    match sh.window_handle().ok()?.as_raw() {
        RawWindowHandle::UiKit(h) => Some(h.ui_view.as_ptr() as *mut AnyObject),
        _ => None,
    }
}

/// Start playing the YouTube video `vid` (its 11-char id). Buffered until the player region
/// reports its geometry; the actual load happens in `show_at`.
pub fn play(vid: &str) {
    if vid.is_empty() {
        return;
    }
    let url = format!(
        "https://www.youtube.com/embed/{vid}?playsinline=1&autoplay=1&rel=0&modestbranding=1"
    );
    PENDING_URL.with(|p| *p.borrow_mut() = Some(url));
    apply_pending();
}

fn apply_pending() {
    unsafe {
        PLAYER.with(|m| {
            let m = m.borrow();
            let Some(wv) = m.as_ref() else { return };
            PENDING_URL.with(|p| {
                let Some(url) = p.borrow().clone() else { return };
                let already = LOADED_URL.with(|l| l.borrow().as_deref() == Some(url.as_str()));
                if already {
                    return;
                }
                let ns = NSString::from_str(&url);
                if let Some(nsurl) = NSURL::URLWithString(&ns) {
                    let req: Retained<AnyObject> =
                        msg_send![class!(NSURLRequest), requestWithURL: &*nsurl];
                    let _: () = msg_send![&**wv, loadRequest: &*req];
                    LOADED_URL.with(|l| *l.borrow_mut() = Some(url));
                }
            });
        });
    }
}

/// Create (first call) + position the player webview to (x, y, w, h) in points. Called from
/// Slint's report-video-geom whenever the player region appears or resizes.
pub fn show_at(window: &slint::Window, x: f32, y: f32, w: f32, h: f32) {
    if w <= 0.0 || h <= 0.0 {
        return;
    }
    let Some(host_ptr) = host_view(window) else { return };
    let frame = CGRect {
        origin: CGPoint { x: x as f64, y: y as f64 },
        size: CGSize { width: w as f64, height: h as f64 },
    };
    unsafe {
        let exists = PLAYER.with(|m| m.borrow().is_some());
        if !exists {
            let config: Retained<AnyObject> = msg_send![class!(WKWebViewConfiguration), new];
            // Play the video in place (not the native fullscreen player), no tap required.
            let _: () = msg_send![&*config, setAllowsInlineMediaPlayback: true];
            let _: () = msg_send![&*config, setMediaTypesRequiringUserActionForPlayback: 0usize];
            let alloc: Allocated<AnyObject> = msg_send![class!(WKWebView), alloc];
            let webview: Retained<AnyObject> =
                msg_send![alloc, initWithFrame: frame, configuration: &*config];
            let _: () = msg_send![&*webview, setOpaque: false];
            let host: &AnyObject = &*host_ptr;
            let _: () = msg_send![host, addSubview: &*webview];
            PLAYER.with(|m| *m.borrow_mut() = Some(webview));
        }
        PLAYER.with(|m| {
            if let Some(wv) = m.borrow().as_ref() {
                let _: () = msg_send![&**wv, setFrame: frame];
                let _: () = msg_send![&**wv, setHidden: false];
            }
        });
        apply_pending();
    }
}

/// Stop playback: navigate the webview to about:blank (hiding alone keeps YouTube's audio
/// running) and hide it. Kept alive for the next play.
pub fn stop() {
    PENDING_URL.with(|p| *p.borrow_mut() = None);
    LOADED_URL.with(|l| *l.borrow_mut() = None);
    unsafe {
        PLAYER.with(|m| {
            if let Some(wv) = m.borrow().as_ref() {
                let ns = NSString::from_str("about:blank");
                if let Some(nsurl) = NSURL::URLWithString(&ns) {
                    let req: Retained<AnyObject> =
                        msg_send![class!(NSURLRequest), requestWithURL: &*nsurl];
                    let _: () = msg_send![&**wv, loadRequest: &*req];
                }
                let _: () = msg_send![&**wv, setHidden: true];
            }
        });
    }
}

/// Tuck/reveal the player without tearing it down (Slint overlays draw under the native view).
pub fn set_hidden(hidden: bool) {
    unsafe {
        PLAYER.with(|m| {
            if let Some(wv) = m.borrow().as_ref() {
                let _: () = msg_send![&**wv, setHidden: hidden];
            }
        });
    }
}

/// Snapshot the CURRENT player frame and push it to Nova as a vision image (duplex). Async:
/// takeSnapshot's completion runs on a background queue → we encode + hand off there.
pub fn snapshot_to_vision() {
    unsafe {
        PLAYER.with(|m| {
            let m = m.borrow();
            let Some(wv) = m.as_ref() else { return };
            let handler = RcBlock::new(move |image: *mut AnyObject, _err: *mut AnyObject| {
                if image.is_null() {
                    return;
                }
                let data: *mut AnyObject = UIImageJPEGRepresentation(image, 0.5);
                if data.is_null() {
                    return;
                }
                let len: usize = msg_send![data, length];
                if len == 0 {
                    return;
                }
                let mut bytes = vec![0u8; len];
                let buf_ptr = bytes.as_mut_ptr() as *mut std::ffi::c_void;
                let _: () = msg_send![data, getBytes: buf_ptr, length: len];
                let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
                crate::realtime::set_viewing_bytes(b64);
            });
            let nil_config: *const AnyObject = std::ptr::null();
            let _: () = msg_send![&**wv, takeSnapshotWithConfiguration: nil_config,
                                          completionHandler: &*handler];
        });
    }
}
