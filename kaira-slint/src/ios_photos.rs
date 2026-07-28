//! iOS photo import via PHPickerViewController. Presents the system photo picker over the
//! Slint view (modal, on the winit window's root VC); the picked image's ENCODED bytes
//! (JPEG/PNG) are handed back to Rust on the UI thread via a global callback. lib.rs decodes
//! them for display + ships them to the bot so Nova can see the imported photo.
//!
//! No objc2-photos-ui dep — PHPickerViewController/PHPickerConfiguration/PHPickerFilter are
//! reached with class!/msg_send! (same as ios_map does for WKWebView); the delegate is a
//! define_class! object that just implements picker:didFinishPicking: (Obj-C duck-types it).

use std::sync::Mutex;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol};
use objc2::{class, define_class, msg_send, AnyThread, MainThreadMarker};
use objc2_foundation::NSString;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

/// Called (on the Slint UI thread) with the picked image's encoded bytes. Set by lib.rs.
/// `Send` only (not `Sync`) — the callback captures the realtime `Sender`; it's stored in a
/// `Mutex` (→ the static is still `Sync`) and only ever invoked on the UI thread.
type PickedCb = Box<dyn Fn(Vec<u8>) + Send>;
static ON_PICKED: Mutex<Option<PickedCb>> = Mutex::new(None);

pub fn set_on_picked(cb: impl Fn(Vec<u8>) + Send + 'static) {
    *ON_PICKED.lock().unwrap() = Some(Box::new(cb));
}

/// The image-load completion block runs on a background queue → hop to the UI thread.
fn deliver(bytes: Vec<u8>) {
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(cb) = ON_PICKED.lock().unwrap().as_ref() {
            cb(bytes);
        }
    });
}

define_class!(
    #[unsafe(super(NSObject))]
    #[name = "VoidAIPhotoDelegate"]
    struct PhotoDelegate;

    impl PhotoDelegate {
        #[unsafe(method(picker:didFinishPicking:))]
        fn did_finish(&self, picker: &AnyObject, results: &AnyObject) {
            unsafe {
                // Dismiss the picker regardless of the outcome.
                let _: () = msg_send![picker, dismissViewControllerAnimated: true,
                                              completion: std::ptr::null::<AnyObject>()];
                let count: usize = msg_send![results, count];
                if count == 0 {
                    return; // user cancelled
                }
                let result: *mut AnyObject = msg_send![results, objectAtIndex: 0usize];
                let provider: *mut AnyObject = msg_send![result, itemProvider];
                if provider.is_null() {
                    return;
                }
                let uti = NSString::from_str("public.image");
                // completion: void(^)(NSData* _Nullable, NSError* _Nullable)
                let handler = RcBlock::new(move |data: *mut AnyObject, _err: *mut AnyObject| {
                    if data.is_null() {
                        return;
                    }
                    let len: usize = msg_send![data, length];
                    if len == 0 {
                        return;
                    }
                    // loadDataRepresentation hands back an OS_dispatch_data whose -bytes trips
                    // objc2's signature check; copy the bytes out with getBytes:length: (a void*
                    // buffer, no return-pointer to type-check) into a Rust-owned Vec instead.
                    let mut bytes = vec![0u8; len];
                    let buf_ptr = bytes.as_mut_ptr() as *mut std::ffi::c_void;
                    let _: () = msg_send![data, getBytes: buf_ptr, length: len];
                    deliver(bytes);
                });
                let _: *mut AnyObject = msg_send![provider,
                    loadDataRepresentationForTypeIdentifier: &*uti,
                    completionHandler: &*handler];
            }
        }
    }

    unsafe impl NSObjectProtocol for PhotoDelegate {}
);

fn host_view(window: &slint::Window) -> Option<*mut AnyObject> {
    MainThreadMarker::new()?;
    let sh = window.window_handle();
    match sh.window_handle().ok()?.as_raw() {
        RawWindowHandle::UiKit(h) => Some(h.ui_view.as_ptr() as *mut AnyObject),
        _ => None,
    }
}

/// Present the system photo picker (images only, single selection). Call on the main thread.
pub fn present(window: &slint::Window) {
    if MainThreadMarker::new().is_none() {
        return;
    }
    let Some(host) = host_view(window) else {
        return;
    };
    unsafe {
        // Present from the winit window's root view controller.
        let win: *mut AnyObject = msg_send![host, window];
        if win.is_null() {
            return;
        }
        let root: *mut AnyObject = msg_send![win, rootViewController];
        if root.is_null() {
            eprintln!("[photos] no rootViewController to present from");
            return;
        }
        // PHPickerConfiguration → images only, one photo.
        let config: Retained<AnyObject> = msg_send![class!(PHPickerConfiguration), new];
        let _: () = msg_send![&*config, setSelectionLimit: 1isize];
        let filter: *mut AnyObject = msg_send![class!(PHPickerFilter), imagesFilter];
        if !filter.is_null() {
            let _: () = msg_send![&*config, setFilter: filter];
        }
        // PHPickerViewController(configuration:)
        let alloc: objc2::rc::Allocated<AnyObject> = msg_send![class!(PHPickerViewController), alloc];
        let picker: Retained<AnyObject> = msg_send![alloc, initWithConfiguration: &*config];
        let delegate = PhotoDelegate::alloc();
        let delegate: Retained<PhotoDelegate> = msg_send![delegate, init];
        let _: () = msg_send![&*picker, setDelegate: &*delegate];
        // The picker holds the delegate weakly; it must outlive the modal presentation.
        std::mem::forget(delegate);
        let _: () = msg_send![root, presentViewController: &*picker, animated: true,
                                     completion: std::ptr::null::<AnyObject>()];
        eprintln!("[photos] presented picker");
    }
}
