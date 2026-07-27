//! iOS GPS via CoreLocation — the iOS analogue of `native_stt::last_location()` on
//! Android. The realtime client sends the phone's coordinates to the bot on connect so
//! Nova's directions/place lookups start from where the user actually is.
//!
//! CLLocationManager delivers fixes to its delegate on the run loop of the thread it was
//! created on, so `start()` MUST be called on the main thread (we call it from the connect
//! handler, right next to `ios_audio::activate()`). The delegate stashes each fix into a
//! Send/Sync global that `last_location()` reads from the WebRTC thread. First launch shows
//! the permission prompt, so the very first connect may have no fix yet (Seattle fallback,
//! same as Android's cold `getLastKnownLocation`); it lands on subsequent turns/connects.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use objc2::rc::Retained;
use objc2::runtime::{NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{define_class, msg_send, AnyThread};
use objc2_core_location::{CLLocation, CLLocationManager, CLLocationManagerDelegate};
use objc2_foundation::NSArray;

/// Latest GPS fix (lat, lng). Written by the delegate on the main run loop, read anywhere.
static LAST_LOC: Mutex<Option<(f64, f64)>> = Mutex::new(None);
static STARTED: AtomicBool = AtomicBool::new(false);

define_class!(
    // Minimal CLLocationManagerDelegate — no ivars; the fix goes into the LAST_LOC global.
    // The delegate selector is implemented in a plain `impl` block and the (optional)
    // protocol is conformed to with an empty `unsafe impl` — the structure objc2 0.6 uses
    // for delegates (CLLocationManager checks respondsToSelector: for the optional method).
    #[unsafe(super(NSObject))]
    #[name = "VoidAILocDelegate"]
    struct LocDelegate;

    impl LocDelegate {
        #[unsafe(method(locationManager:didUpdateLocations:))]
        fn did_update(&self, _manager: &CLLocationManager, locations: &NSArray<CLLocation>) {
            if let Some(loc) = locations.lastObject() {
                let c = unsafe { loc.coordinate() };
                // Reject the "invalid coordinate" sentinel (0,0 / out-of-range).
                if c.latitude.abs() <= 90.0
                    && c.longitude.abs() <= 180.0
                    && (c.latitude != 0.0 || c.longitude != 0.0)
                {
                    if let Ok(mut g) = LAST_LOC.lock() {
                        *g = Some((c.latitude, c.longitude));
                    }
                }
            }
        }
    }

    unsafe impl NSObjectProtocol for LocDelegate {}
    unsafe impl CLLocationManagerDelegate for LocDelegate {}
);

/// Start acquiring GPS. Call ONCE, on the main thread. Idempotent + best-effort (silent if
/// CoreLocation is unavailable or permission is denied).
pub fn start() {
    if STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    unsafe {
        let manager = CLLocationManager::new();
        let delegate = LocDelegate::alloc();
        let delegate: Retained<LocDelegate> = msg_send![delegate, init];
        manager.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
        manager.requestWhenInUseAuthorization();
        manager.startUpdatingLocation();
        // Both must outlive this function for updates to keep flowing (the manager holds
        // the delegate weakly). We never touch them again from Rust, so leak them.
        std::mem::forget(manager);
        std::mem::forget(delegate);
    }
}

/// Best-effort last GPS fix (lat, lng). `None` until CoreLocation delivers the first update.
pub fn last_location() -> Option<(f64, f64)> {
    *LAST_LOC.lock().unwrap()
}
