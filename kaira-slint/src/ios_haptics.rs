//! iOS haptics via UIImpactFeedbackGenerator — a light tap on connect/disconnect, pause/resume,
//! and persona switch. Main-thread only; we only call it from Slint callbacks (the UI thread).

use objc2::rc::{Allocated, Retained};
use objc2::runtime::AnyObject;
use objc2::{class, msg_send, MainThreadMarker};

/// Fire a one-shot impact. `style` is UIImpactFeedbackStyle: 0 = light, 1 = medium, 2 = heavy.
/// Best-effort — silently no-ops off the main thread or if UIKit isn't available.
pub fn impact(style: isize) {
    if MainThreadMarker::new().is_none() {
        return;
    }
    unsafe {
        let alloc: Allocated<AnyObject> = msg_send![class!(UIImpactFeedbackGenerator), alloc];
        let generator: Retained<AnyObject> = msg_send![alloc, initWithStyle: style];
        let _: () = msg_send![&*generator, prepare];
        let _: () = msg_send![&*generator, impactOccurred];
    }
}

/// A light tap — the default confirm-an-action feedback.
pub fn tap() {
    impact(0);
}
