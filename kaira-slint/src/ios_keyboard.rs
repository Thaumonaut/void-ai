//! iOS soft-keyboard handling for the chat composer. winit/Slint don't inset the view for the
//! keyboard on iOS, so a bottom-anchored text field sits hidden behind it. This module:
//!   • observes the keyboard's frame (UIKeyboard*Notification) and reports its height to Slint,
//!     which lifts the whole app above it (`MainWindow.keyboard-height`);
//!   • dismisses the keyboard on demand (`endEditing:` — resigns the first responder).

use std::sync::Mutex;

use block2::RcBlock;
use objc2::runtime::AnyObject;
use objc2::{class, msg_send, MainThreadMarker};
use objc2_core_foundation::CGRect;
use objc2_foundation::NSString;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

type HeightCb = Box<dyn Fn(f64) + Send>;
static ON_HEIGHT: Mutex<Option<HeightCb>> = Mutex::new(None);

/// Called (on the main thread) with the keyboard's overlap height in points; 0 when hidden.
pub fn set_on_height(cb: impl Fn(f64) + Send + 'static) {
    *ON_HEIGHT.lock().unwrap() = Some(Box::new(cb));
}

/// Register keyboard-frame observers (once, on the main thread, after the window exists). The
/// blocks are intentionally leaked — they live for the app's lifetime.
pub fn observe() {
    if MainThreadMarker::new().is_none() {
        return;
    }
    unsafe {
        let center: *mut AnyObject = msg_send![class!(NSNotificationCenter), defaultCenter];
        let queue: *mut AnyObject = msg_send![class!(NSOperationQueue), mainQueue];
        let nil: *const AnyObject = core::ptr::null();
        // WillChangeFrame covers show + interactive resize; WillHide zeroes it back out.
        for (name, hide) in [
            ("UIKeyboardWillChangeFrameNotification", false),
            ("UIKeyboardWillHideNotification", true),
        ] {
            let ns = NSString::from_str(name);
            let handler = RcBlock::new(move |note: *mut AnyObject| {
                let h = if hide { 0.0 } else { keyboard_height(note) };
                if let Some(cb) = ON_HEIGHT.lock().unwrap().as_ref() {
                    cb(h);
                }
            });
            let _: *mut AnyObject = msg_send![center,
                addObserverForName: &*ns, object: nil, queue: queue, usingBlock: &*handler];
            std::mem::forget(handler);
        }
    }
}

/// The keyboard's end-frame height (points) from a UIKeyboard notification's userInfo.
unsafe fn keyboard_height(note: *mut AnyObject) -> f64 {
    if note.is_null() {
        return 0.0;
    }
    let user_info: *mut AnyObject = msg_send![note, userInfo];
    if user_info.is_null() {
        return 0.0;
    }
    let key = NSString::from_str("UIKeyboardFrameEndUserInfoKey");
    let val: *mut AnyObject = msg_send![user_info, objectForKey: &*key];
    if val.is_null() {
        return 0.0;
    }
    // NSValue wrapping the keyboard's end CGRect (screen coords). Report how much it OVERLAPS the
    // screen bottom (screenH - frame.origin.y) — this is 0 while it animates off-screen on hide,
    // so a single WillChangeFrame observer handles show + hide + interactive drag correctly.
    let rect: CGRect = msg_send![val, CGRectValue];
    let screen: *mut AnyObject = msg_send![class!(UIScreen), mainScreen];
    let bounds: CGRect = msg_send![screen, bounds];
    (bounds.size.height - rect.origin.y).max(0.0)
}

fn host_view(window: &slint::Window) -> Option<*mut AnyObject> {
    MainThreadMarker::new()?;
    match window.window_handle().window_handle().ok()?.as_raw() {
        RawWindowHandle::UiKit(h) => Some(h.ui_view.as_ptr() as *mut AnyObject),
        _ => None,
    }
}

/// Resign the first responder → hide the keyboard.
pub fn dismiss(window: &slint::Window) {
    if let Some(host) = host_view(window) {
        unsafe {
            let _: bool = msg_send![host, endEditing: true];
        }
    }
}
