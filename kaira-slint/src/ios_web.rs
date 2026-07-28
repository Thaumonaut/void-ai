//! iOS: a real in-app web reader — a WKWebView that loads the ACTIVE web tab's actual URL,
//! positioned to cover the Slint "web" view's content region (reported via report-web-geom,
//! exactly like ios_map). The Slint browser toolbar (tab count · address · kebab) sits ABOVE
//! this region; Nova's summary blocks live UNDER the webview and are revealed by tucking the
//! webview away ("Nova's take"), the same trick set_hidden already uses for overlays.
//!
//! Driven by lib.rs: `load_url` navigates the webview when a new `web` op lands or the user
//! flips tabs; `show_at` (re)frames it; `hide`/`set_hidden` tuck it behind Slint chrome. The
//! target URL is double-buffered (Rust holds the latest until the webview exists).

use std::cell::RefCell;

use objc2::rc::{Allocated, Retained};
use objc2::runtime::AnyObject;
use objc2::{class, msg_send, MainThreadMarker};
use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use objc2_foundation::{NSString, NSURL};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

thread_local! {
    static WEB_VIEW: RefCell<Option<Retained<AnyObject>>> = const { RefCell::new(None) };
    // Latest URL to show, applied once the webview exists.
    static PENDING_URL: RefCell<Option<String>> = const { RefCell::new(None) };
    // URL currently loaded — so re-showing / reframing doesn't reload the page.
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

/// Point the reader at `url`. Buffered until the webview exists; a no-op if it's already the
/// loaded page (so tab-switching back to an open page doesn't reload it).
pub fn load_url(url: &str) {
    if url.is_empty() {
        return;
    }
    PENDING_URL.with(|p| *p.borrow_mut() = Some(url.to_string()));
    apply_pending();
}

/// Navigate a WKWebView to a remote URL. We deliberately do NOT hand-build an `NSURLRequest`
/// via `class!/msg_send!` — that trips objc2's debug message-send verification and aborts the
/// process (SIGABRT). Instead we load a tiny HTML shim (the same `loadHTMLString:` path the map
/// uses reliably) whose script redirects the top frame to the target — a normal navigation.
unsafe fn load_remote(webview: &AnyObject, url: &str) {
    let esc = url.replace('\\', "\\\\").replace('\'', "\\'");
    let html = format!(
        "<!doctype html><meta charset=utf-8><title>…</title>\
         <body style='background:#000'><script>location.replace('{esc}')</script>"
    );
    let html_ns = NSString::from_str(&html);
    let base = NSString::from_str("about:blank");
    if let Some(base_url) = NSURL::URLWithString(&base) {
        // loadHTMLString:baseURL: returns a WKNavigation* — declare the object return (like
        // ios_map does); declaring `()` mismatches the encoding and objc2 aborts the process.
        let _: *mut AnyObject = msg_send![webview, loadHTMLString: &*html_ns, baseURL: &*base_url];
    }
}

fn apply_pending() {
    unsafe {
        WEB_VIEW.with(|m| {
            let m = m.borrow();
            let Some(wv) = m.as_ref() else { return };
            PENDING_URL.with(|p| {
                let want = p.borrow().clone();
                let Some(url) = want else { return };
                let already = LOADED_URL.with(|l| l.borrow().as_deref() == Some(url.as_str()));
                if already {
                    return;
                }
                load_remote(&**wv, &url);
                LOADED_URL.with(|l| *l.borrow_mut() = Some(url));
            });
        });
    }
}

/// Show (creating on first call) the reader webview and position it to (x, y, w, h) in points.
/// Called from Slint's report-web-geom whenever the web content region appears or resizes.
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
        let exists = WEB_VIEW.with(|m| m.borrow().is_some());
        if !exists {
            let config: Retained<AnyObject> = msg_send![class!(WKWebViewConfiguration), new];
            let alloc: Allocated<AnyObject> = msg_send![class!(WKWebView), alloc];
            let webview: Retained<AnyObject> =
                msg_send![alloc, initWithFrame: frame, configuration: &*config];
            // Opaque + a solid white backing: real pages assume a white page, and pages with a
            // transparent background must NOT let the Slint chat/summary behind show through.
            let _: () = msg_send![&*webview, setOpaque: true];
            let white: *mut AnyObject = msg_send![class!(UIColor), whiteColor];
            let _: () = msg_send![&*webview, setBackgroundColor: white];
            let scroll: *mut AnyObject = msg_send![&*webview, scrollView];
            let _: () = msg_send![scroll, setBackgroundColor: white];
            let host: &AnyObject = &*host_ptr;
            let _: () = msg_send![host, addSubview: &*webview];
            WEB_VIEW.with(|m| *m.borrow_mut() = Some(webview));
        }
        WEB_VIEW.with(|m| {
            if let Some(wv) = m.borrow().as_ref() {
                let _: () = msg_send![&**wv, setFrame: frame];
                let _: () = msg_send![&**wv, setHidden: false];
            }
        });
        // Load whatever URL is pending (first show, or a tab-switch that happened while hidden).
        apply_pending();
    }
}

/// Navigate back within the current tab's history. No-op if it can't go back.
pub fn go_back() {
    unsafe {
        WEB_VIEW.with(|m| {
            if let Some(wv) = m.borrow().as_ref() {
                let can: bool = msg_send![&**wv, canGoBack];
                if can {
                    let _: *mut AnyObject = msg_send![&**wv, goBack];
                }
            }
        });
    }
}

/// Hide the reader webview (kept alive so the page doesn't reload next time).
pub fn hide() {
    set_hidden(true);
}

/// Toggle visibility without reframing. Tucks the webview behind Slint overlays (settings,
/// the "Nova's take" panel, the tab switcher) that would otherwise be drawn under it.
pub fn set_hidden(hidden: bool) {
    unsafe {
        WEB_VIEW.with(|m| {
            if let Some(wv) = m.borrow().as_ref() {
                let _: () = msg_send![&**wv, setHidden: hidden];
            }
        });
    }
}
