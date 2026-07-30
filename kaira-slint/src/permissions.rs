//! First-run permission handling: microphone (required for Talk) + location
//! (optional, for "directions from here"). Everything is requested **in-app** so a
//! shipped build never depends on `adb grant` / out-of-band tooling.
//!
//! - iOS: `AVAudioSession` record permission + `CLLocationManager` authorization, via objc2.
//! - Android: the Activity's runtime-permission API over JNI (the context handed back by
//!   `ndk_context` is the app's Activity). NativeActivity doesn't forward
//!   `onRequestPermissionsResult`, so a grant is detected by polling `checkSelfPermission`.
//! - Desktop: always granted (no OS gate) so the fast UI loop is unaffected.
//!
//! `Perm` is a 3-state int enum shared verbatim with Slint (`VS.mic-perm` / `VS.loc-perm`):
//! 0 = undetermined (never asked), 1 = granted, 2 = denied/blocked.

/// Permission state, mirrored 1:1 into the `VS.mic-perm` / `VS.loc-perm` Slint ints.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Perm {
    Undetermined = 0,
    Granted = 1,
    Denied = 2,
}

impl Perm {
    /// The int the Slint side reads (0 undetermined / 1 granted / 2 denied).
    pub fn as_i32(self) -> i32 {
        self as i32
    }
}

/// Current microphone-permission state (no prompt).
pub fn mic_status() -> Perm {
    imp::mic_status()
}

/// Ask for microphone permission. `cb` is invoked with the outcome once the user
/// responds — possibly on another thread and possibly after this returns (iOS calls it
/// immediately if already determined; Android polls). Wire `cb` to marshal back onto the
/// Slint event loop.
pub fn request_mic(cb: impl Fn(Perm) + Send + 'static) {
    imp::request_mic(cb)
}

/// Current location-permission state (no prompt).
pub fn location_status() -> Perm {
    imp::location_status()
}

/// Ask for location permission (when-in-use). Fire-and-forget — read `location_status()`
/// afterwards; a denied location never blocks Talk, it only degrades "directions from here".
pub fn request_location() {
    imp::request_location()
}

/// Open the OS Settings page for this app so a blocked user can flip the switch and return.
pub fn open_app_settings() {
    imp::open_app_settings()
}

// ---------------------------------------------------------------------------
// iOS
// ---------------------------------------------------------------------------
#[cfg(target_os = "ios")]
mod imp {
    use super::Perm;
    use objc2::runtime::Bool;
    use objc2_avf_audio::AVAudioSession;

    // The four-char-code values `recordPermission` returns (see AVAudioSessionRecordPermission).
    const GRANTED: usize = 0x67726e74; // 'grnt'
    const DENIED: usize = 0x64656e79; // 'deny'

    #[allow(deprecated)] // recordPermission is fine on our iOS 16 floor; AVAudioApplication is iOS 17+.
    pub fn mic_status() -> Perm {
        unsafe {
            let session = AVAudioSession::sharedInstance();
            match session.recordPermission().0 {
                GRANTED => Perm::Granted,
                DENIED => Perm::Denied,
                _ => Perm::Undetermined, // 'undt'
            }
        }
    }

    #[allow(deprecated)]
    pub fn request_mic(cb: impl Fn(Perm) + Send + 'static) {
        unsafe {
            let session = AVAudioSession::sharedInstance();
            // void(^)(BOOL granted) — Apple may call it on an arbitrary thread; `cb` must
            // (and does) hop back to the Slint event loop itself.
            let block = block2::RcBlock::new(move |granted: Bool| {
                cb(if granted.as_bool() { Perm::Granted } else { Perm::Denied });
            });
            session.requestRecordPermission(&block);
        }
    }

    pub fn location_status() -> Perm {
        use objc2_core_location::CLLocationManager;
        unsafe {
            // notDetermined=0 restricted=1 denied=2 authorizedAlways=3 authorizedWhenInUse=4
            match CLLocationManager::new().authorizationStatus().0 {
                3 | 4 => Perm::Granted,
                1 | 2 => Perm::Denied,
                _ => Perm::Undetermined,
            }
        }
    }

    pub fn request_location() {
        // Reuse the CLLocationManager that also feeds the bot the user's GPS; its `start()`
        // calls requestWhenInUseAuthorization, which is the prompt.
        crate::ios_location::start();
    }

    pub fn open_app_settings() {
        // UIApplicationOpenSettingsURLString == "app-settings:" — routes to this app's page.
        crate::ios_url::launch_url("app-settings:");
    }
}

// ---------------------------------------------------------------------------
// Android
// ---------------------------------------------------------------------------
#[cfg(target_os = "android")]
mod imp {
    use super::Perm;
    use jni::objects::{JObject, JValue};
    use jni::JNIEnv;
    use std::time::Duration;

    const MIC: &str = "android.permission.RECORD_AUDIO";
    const FINE: &str = "android.permission.ACCESS_FINE_LOCATION";
    const COARSE: &str = "android.permission.ACCESS_COARSE_LOCATION";

    /// Attach the current thread and hand a JNIEnv + the app Activity/Context to `f`
    /// (same idiom as `native_stt::with_env`; the ndk context IS the Activity here).
    fn with_env<F, R>(f: F) -> Result<R, String>
    where
        F: FnOnce(&mut JNIEnv, &JObject) -> Result<R, String>,
    {
        let ctx = ndk_context::android_context();
        if ctx.vm().is_null() || ctx.context().is_null() {
            return Err("android context unavailable".into());
        }
        let vm = unsafe { jni::JavaVM::from_raw(ctx.vm().cast()) }.map_err(|e| e.to_string())?;
        let mut env = vm.attach_current_thread().map_err(|e| e.to_string())?;
        let context = unsafe { JObject::from_raw(ctx.context().cast()) };
        f(&mut env, &context)
    }

    /// Context.checkSelfPermission(name) == PERMISSION_GRANTED(0)? Undetermined vs
    /// permanently-denied can't be told apart before a prompt, so map "not granted" to
    /// Undetermined and let `request_mic`'s post-prompt heuristic decide Denied.
    fn granted(perm: &str) -> bool {
        with_env(|env, ctx| {
            let p = env.new_string(perm).map_err(|e| e.to_string())?;
            let r = env
                .call_method(ctx, "checkSelfPermission", "(Ljava/lang/String;)I", &[JValue::Object(&JObject::from(p))])
                .and_then(|v| v.i())
                .map_err(|e| e.to_string())?;
            Ok(r == 0)
        })
        .unwrap_or(false)
    }

    fn should_show_rationale(perm: &str) -> bool {
        with_env(|env, ctx| {
            let p = env.new_string(perm).map_err(|e| e.to_string())?;
            let r = env
                .call_method(ctx, "shouldShowRequestPermissionRationale", "(Ljava/lang/String;)Z", &[JValue::Object(&JObject::from(p))])
                .and_then(|v| v.z())
                .map_err(|e| e.to_string())?;
            Ok(r)
        })
        .unwrap_or(false)
    }

    /// Activity.requestPermissions(String[], int) — shows the system dialog. Must run on
    /// the caller's thread (we call it from the main-thread Slint callback).
    fn request(perms: &[&str]) {
        let _ = with_env(|env, ctx| {
            let arr = env
                .new_object_array(perms.len() as i32, "java/lang/String", JObject::null())
                .map_err(|e| e.to_string())?;
            for (i, p) in perms.iter().enumerate() {
                let s = JObject::from(env.new_string(p).map_err(|e| e.to_string())?);
                env.set_object_array_element(&arr, i as i32, &s).map_err(|e| e.to_string())?;
            }
            // JValue::Object wants &JObject; JObjectArray derefs to JObject.
            let arr_ref: &JObject = &arr;
            env.call_method(
                ctx,
                "requestPermissions",
                "([Ljava/lang/String;I)V",
                &[JValue::Object(arr_ref), JValue::Int(0)],
            )
            .map_err(|e| e.to_string())?;
            Ok(())
        });
    }

    pub fn mic_status() -> Perm {
        if granted(MIC) {
            Perm::Granted
        } else {
            Perm::Undetermined
        }
    }

    pub fn location_status() -> Perm {
        if granted(FINE) || granted(COARSE) {
            Perm::Granted
        } else {
            Perm::Undetermined
        }
    }

    pub fn request_mic(cb: impl Fn(Perm) + Send + 'static) {
        request(&[MIC]);
        // NativeActivity gives us no result callback, so poll checkSelfPermission (~20s).
        std::thread::spawn(move || {
            for _ in 0..66 {
                std::thread::sleep(Duration::from_millis(300));
                if granted(MIC) {
                    cb(Perm::Granted);
                    return;
                }
            }
            // Timed out ungranted: no-rationale ⇒ "Don't ask again"/blocked; else just dismissed.
            cb(if should_show_rationale(MIC) {
                Perm::Undetermined
            } else {
                Perm::Denied
            });
        });
    }

    pub fn request_location() {
        request(&[FINE, COARSE]);
    }

    pub fn open_app_settings() {
        let _ = with_env(|env, ctx| {
            let action = JObject::from(
                env.new_string("android.settings.APPLICATION_DETAILS_SETTINGS").map_err(|e| e.to_string())?,
            );
            let intent = env
                .new_object("android/content/Intent", "(Ljava/lang/String;)V", &[JValue::Object(&action)])
                .map_err(|e| e.to_string())?;
            let pkg = env
                .call_method(ctx, "getPackageName", "()Ljava/lang/String;", &[])
                .map_err(|e| e.to_string())?
                .l()
                .map_err(|e| e.to_string())?;
            let scheme = JObject::from(env.new_string("package").map_err(|e| e.to_string())?);
            let null = JObject::null();
            // Uri.fromParts("package", <pkg>, null) → package:<pkg>
            let uri = env
                .call_static_method(
                    "android/net/Uri",
                    "fromParts",
                    "(Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;)Landroid/net/Uri;",
                    &[JValue::Object(&scheme), JValue::Object(&pkg), JValue::Object(&null)],
                )
                .map_err(|e| e.to_string())?
                .l()
                .map_err(|e| e.to_string())?;
            env.call_method(&intent, "setData", "(Landroid/net/Uri;)Landroid/content/Intent;", &[JValue::Object(&uri)])
                .map_err(|e| e.to_string())?;
            // FLAG_ACTIVITY_NEW_TASK — required to start an Activity from a non-Activity ctx path.
            env.call_method(&intent, "addFlags", "(I)Landroid/content/Intent;", &[JValue::Int(0x1000_0000)])
                .map_err(|e| e.to_string())?;
            env.call_method(ctx, "startActivity", "(Landroid/content/Intent;)V", &[JValue::Object(&intent)])
                .map_err(|e| e.to_string())?;
            Ok(())
        });
    }
}

// ---------------------------------------------------------------------------
// Desktop (no OS permission gate)
// ---------------------------------------------------------------------------
#[cfg(not(any(target_os = "ios", target_os = "android")))]
mod imp {
    use super::Perm;

    pub fn mic_status() -> Perm {
        Perm::Granted
    }
    pub fn request_mic(cb: impl Fn(Perm) + Send + 'static) {
        cb(Perm::Granted);
    }
    pub fn location_status() -> Perm {
        Perm::Granted
    }
    pub fn request_location() {}
    pub fn open_app_settings() {}
}
