//! Android's built-in SpeechRecognizer ("Native" STT), reached from Rust via JNI.
//! A tiny Java helper (`com.kaira.voicelab.SttBridge`, dex'd + embedded below) is
//! loaded at runtime with DexClassLoader; it drives SpeechRecognizer on the main
//! thread and exposes the result via static fields we poll — no native methods,
//! so no Gradle/Java build-system change is needed.

use jni::objects::{JClass, JObject, JString, JValue};
use jni::JNIEnv;
use std::sync::OnceLock;

const DEX: &[u8] = include_bytes!("../android_stt/classes.dex");
static BRIDGE: OnceLock<jni::objects::GlobalRef> = OnceLock::new();

/// Attach the current thread and hand a JNIEnv + the app Context to `f`.
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

/// Load the dex + cache the SttBridge class (once).
fn ensure_loaded(env: &mut JNIEnv, context: &JObject) -> Result<(), String> {
    if BRIDGE.get().is_some() {
        return Ok(());
    }
    let cache_dir = env
        .call_method(context, "getCacheDir", "()Ljava/io/File;", &[])
        .and_then(|v| v.l())
        .map_err(|e| format!("getCacheDir: {e}"))?;
    let path_obj: JString = env
        .call_method(&cache_dir, "getAbsolutePath", "()Ljava/lang/String;", &[])
        .and_then(|v| v.l())
        .map_err(|e| format!("getAbsolutePath: {e}"))?
        .into();
    let cache_path: String = env.get_string(&path_obj).map_err(|e| e.to_string())?.into();
    let dex_path = format!("{cache_path}/stt_bridge.dex");
    std::fs::write(&dex_path, DEX).map_err(|e| format!("write dex: {e}"))?;

    let parent = env
        .call_method(context, "getClassLoader", "()Ljava/lang/ClassLoader;", &[])
        .and_then(|v| v.l())
        .map_err(|e| format!("getClassLoader: {e}"))?;
    let dex_path_j = env.new_string(&dex_path).map_err(|e| e.to_string())?;
    let opt_dir_j = env.new_string(&cache_path).map_err(|e| e.to_string())?;
    let null = JObject::null();
    let dcl = env
        .new_object(
            "dalvik/system/DexClassLoader",
            "(Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;Ljava/lang/ClassLoader;)V",
            &[
                JValue::Object(&dex_path_j),
                JValue::Object(&opt_dir_j),
                JValue::Object(&null),
                JValue::Object(&parent),
            ],
        )
        .map_err(|e| format!("DexClassLoader: {e}"))?;
    let name_j = env.new_string("com.kaira.voicelab.SttBridge").map_err(|e| e.to_string())?;
    let class = env
        .call_method(&dcl, "loadClass", "(Ljava/lang/String;)Ljava/lang/Class;", &[JValue::Object(&name_j)])
        .and_then(|v| v.l())
        .map_err(|e| format!("loadClass SttBridge: {e}"))?;
    let global = env.new_global_ref(&class).map_err(|e| e.to_string())?;
    let _ = BRIDGE.set(global);
    Ok(())
}

fn bridge_class() -> Result<JClass<'static>, String> {
    let g = BRIDGE.get().ok_or("SttBridge not loaded")?;
    Ok(unsafe { JClass::from_raw(g.as_raw()) })
}

/// Whether the device has speech recognition available.
pub fn available() -> bool {
    with_env(|env, ctx| {
        ensure_loaded(env, ctx)?;
        let class = bridge_class()?;
        env.call_static_method(&class, "available", "(Landroid/content/Context;)Z", &[JValue::Object(ctx)])
            .and_then(|v| v.z())
            .map_err(|e| e.to_string())
    })
    .unwrap_or(false)
}

/// Start listening on-device (`lang` e.g. "en-US" / "id-ID").
pub fn start(lang: &str) -> Result<(), String> {
    with_env(|env, ctx| {
        ensure_loaded(env, ctx)?;
        let class = bridge_class()?;
        let lang_j = env.new_string(lang).map_err(|e| e.to_string())?;
        env.call_static_method(
            &class,
            "start",
            "(Landroid/content/Context;Ljava/lang/String;)V",
            &[JValue::Object(ctx), JValue::Object(&lang_j)],
        )
        .map(|_| ())
        .map_err(|e| e.to_string())
    })
}

/// Stop listening (recognizer finalizes → result becomes ready).
pub fn stop() {
    let _ = with_env(|env, _ctx| {
        let class = bridge_class()?;
        env.call_static_method(&class, "stop", "()V", &[]).map_err(|e| e.to_string())?;
        Ok(())
    });
}

/// Whether the recognizer is ready and actively listening (onReadyForSpeech
/// fired) — used to show a "speak now" cue at the right moment.
pub fn listening() -> bool {
    with_env(|env, _ctx| {
        let class = bridge_class()?;
        env.get_static_field(&class, "listening", "Z")
            .and_then(|v| v.z())
            .map_err(|e| e.to_string())
    })
    .unwrap_or(false)
}

/// Whether the recognizer's VAD has detected end-of-speech (you paused). Lets us
/// submit the partial the instant you stop talking, before finalize or a Stop tap.
pub fn speech_ended() -> bool {
    with_env(|env, _ctx| {
        let class = bridge_class()?;
        env.get_static_field(&class, "speechEnded", "Z")
            .and_then(|v| v.z())
            .map_err(|e| e.to_string())
    })
    .unwrap_or(false)
}

/// The latest streaming partial transcript (may be empty). Lets the round-trip
/// start the LLM on Stop without waiting for the recognizer to finalize.
pub fn partial() -> String {
    with_env(|env, _ctx| {
        let class = bridge_class()?;
        let obj = env
            .get_static_field(&class, "partial", "Ljava/lang/String;")
            .and_then(|v| v.l())
            .map_err(|e| e.to_string())?;
        Ok(env
            .get_string(&JString::from(obj))
            .map(|s| s.into())
            .unwrap_or_default())
    })
    .unwrap_or_default()
}

/// Poll for the result: (done, transcript, error).
pub fn poll() -> (bool, String, String) {
    with_env(|env, _ctx| {
        let class = bridge_class()?;
        let done = env.get_static_field(&class, "done", "Z").and_then(|v| v.z()).map_err(|e| e.to_string())?;
        let mut read = |field: &str| -> String {
            match env.get_static_field(&class, field, "Ljava/lang/String;").and_then(|v| v.l()) {
                Ok(obj) => {
                    let js = JString::from(obj);
                    env.get_string(&js).map(|s| s.into()).unwrap_or_default()
                }
                Err(_) => String::new(),
            }
        };
        let result = read("result");
        let error = read("error");
        Ok((done, result, error))
    })
    .unwrap_or((true, String::new(), "jni error".into()))
}

/// Open `url` in the phone's default handler (browser / nav app) via an
/// ACTION_VIEW intent. Used for plan-then-handoff navigation — Nova plans the
/// route, then hands the destination to Google/Apple Maps for live turn-by-turn.
pub fn launch_url(url: &str) -> Result<(), String> {
    with_env(|env, context| {
        let action = env.new_string("android.intent.action.VIEW").map_err(|e| e.to_string())?;
        let url_j = env.new_string(url).map_err(|e| e.to_string())?;
        // Uri uri = Uri.parse(url);
        let uri = env
            .call_static_method(
                "android/net/Uri",
                "parse",
                "(Ljava/lang/String;)Landroid/net/Uri;",
                &[JValue::Object(&JObject::from(url_j))],
            )
            .and_then(|v| v.l())
            .map_err(|e| e.to_string())?;
        // Intent intent = new Intent(Intent.ACTION_VIEW, uri);
        let intent = env
            .new_object(
                "android/content/Intent",
                "(Ljava/lang/String;Landroid/net/Uri;)V",
                &[JValue::Object(&JObject::from(action)), JValue::Object(&uri)],
            )
            .map_err(|e| e.to_string())?;
        // intent.addFlags(FLAG_ACTIVITY_NEW_TASK)  — required to start from a non-Activity context.
        env.call_method(&intent, "addFlags", "(I)Landroid/content/Intent;", &[JValue::Int(0x1000_0000)])
            .map_err(|e| e.to_string())?;
        // context.startActivity(intent);
        env.call_method(context, "startActivity", "(Landroid/content/Intent;)V", &[JValue::Object(&intent)])
            .map_err(|e| e.to_string())?;
        Ok(())
    })
}

/// Best-effort last-known GPS (lat, lng) via Android LocationManager. Returns None if
/// unavailable (no permission / no cached fix). Non-blocking — never waits for a live fix.
pub fn last_location() -> Option<(f64, f64)> {
    with_env(|env, context| {
        let svc = env.new_string("location").map_err(|e| e.to_string())?;
        let lm = env
            .call_method(
                context,
                "getSystemService",
                "(Ljava/lang/String;)Ljava/lang/Object;",
                &[JValue::Object(&JObject::from(svc))],
            )
            .and_then(|v| v.l())
            .map_err(|e| e.to_string())?;
        for provider in ["fused", "gps", "network", "passive"] {
            let p = match env.new_string(provider) {
                Ok(s) => s,
                Err(_) => continue,
            };
            let res = env.call_method(
                &lm,
                "getLastKnownLocation",
                "(Ljava/lang/String;)Landroid/location/Location;",
                &[JValue::Object(&JObject::from(p))],
            );
            // an unknown provider / missing permission throws — clear + try the next
            if env.exception_check().unwrap_or(false) {
                let _ = env.exception_clear();
                continue;
            }
            let loc = match res.and_then(|v| v.l()) {
                Ok(o) if !o.is_null() => o,
                _ => continue,
            };
            let lat = env.call_method(&loc, "getLatitude", "()D", &[]).and_then(|v| v.d());
            let lng = env.call_method(&loc, "getLongitude", "()D", &[]).and_then(|v| v.d());
            if let (Ok(lat), Ok(lng)) = (lat, lng) {
                return Ok(Some((lat, lng)));
            }
        }
        Ok(None)
    })
    .unwrap_or(None)
}
