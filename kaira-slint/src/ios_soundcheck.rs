//! iOS "sound check" — make sure the user will actually HEAR Nova before she speaks.
//!
//! Stage 1 (this file): the reliable, always-available signals —
//!   - `output_volume()` : the media output volume (AVAudioSession.outputVolume, 0..1). Catches a
//!     muted phone / silent switch / volume turned way down. Works on speaker AND headphones.
//!   - `route()`         : is the user on the built-in speaker vs headphones/Bluetooth/etc, so the
//!     UI can tailor guidance (and, later, decide whether the acoustic loopback even applies).
//!
//! Stage 2 (follow-up): an ~18 kHz loopback (play a near-inaudible chirp on a dedicated 48 kHz
//! AVAudioEngine, listen in a narrow band on the mic) to prove the phone hears its own sound over
//! the ROOM — the environment-relative check. Not in this file yet.

use std::cell::{Cell, RefCell};
use std::f64::consts::PI;

use objc2::rc::{Allocated, Retained};
use objc2::runtime::AnyObject;
use objc2::{class, msg_send};
use objc2_foundation::{NSError, NSString};

// ---- Stage 2: ultrasonic loopback ("does the phone hear its own sound over the room?") ----
// Play an ~18 kHz tone (inaudible to most adults) on the loudspeaker while recording the mic to a
// WAV, then Goertzel the recording at 18 kHz vs a 16.5 kHz reference. The ratio is self-normalizing
// (both scale with level + length), so it's robust to overall gain — a strong 18 kHz bin over the
// reference means the mic caught the tone → volume is adequate over the ambient noise.

const TEST_FREQ: f64 = 18000.0;
const REF_FREQ: f64 = 16500.0;
const SR: u32 = 48000;

thread_local! {
    static PLAYER: RefCell<Option<Retained<AnyObject>>> = const { RefCell::new(None) };
    static RECORDER: RefCell<Option<Retained<AnyObject>>> = const { RefCell::new(None) };
    // Result sink: finish_loopback() hands (detected, ratio) back to the caller (set in run_loopback).
    #[allow(clippy::type_complexity)]
    static RESULT_CB: RefCell<Option<Box<dyn FnOnce(bool, f64)>>> = const { RefCell::new(None) };
    // Continuous ("live") loopback: the 18 kHz tone plays while we re-measure the mic a few times a
    // second, so the meter tracks the volume in real time. LIVE_CB gets (signal_rms, noise_rms, ratio).
    static LIVE: Cell<bool> = const { Cell::new(false) };
    #[allow(clippy::type_complexity)]
    static LIVE_CB: RefCell<Option<Box<dyn Fn(f32, f32, f32)>>> = const { RefCell::new(None) };
}

fn tmp(name: &str) -> String {
    format!("{}/{}", crate::tts::files_dir(), name)
}

/// A mono sine at `freq`, `secs` long, with a 5 ms raised-cosine fade in/out (avoids clicks).
fn gen_tone(freq: f64, secs: f64, amp: f64) -> Vec<i16> {
    let n = (SR as f64 * secs) as usize;
    let fade = (SR as f64 * 0.005) as usize; // 5 ms
    (0..n)
        .map(|i| {
            let t = i as f64 / SR as f64;
            let env = if i < fade {
                i as f64 / fade as f64
            } else if i + fade >= n {
                (n - i) as f64 / fade as f64
            } else {
                1.0
            };
            ((amp * env * (2.0 * PI * freq * t).sin()) * 32767.0) as i16
        })
        .collect()
}

/// A short pleasant two-tone chime (audible) for the headphone "did you hear that?" confirm.
fn gen_chime() -> Vec<i16> {
    let secs = 0.55;
    let n = (SR as f64 * secs) as usize;
    (0..n)
        .map(|i| {
            let t = i as f64 / SR as f64;
            let env = (PI * i as f64 / n as f64).sin(); // raised-cosine over the whole clip
            let s = 0.5 * (2.0 * PI * 880.0 * t).sin() + 0.35 * (2.0 * PI * 1320.0 * t).sin();
            ((0.5 * env * s) * 32767.0) as i16
        })
        .collect()
}

fn write_wav(path: &str, samples: &[i16]) -> std::io::Result<()> {
    let data_len = (samples.len() * 2) as u32;
    let byte_rate = SR * 2;
    let mut b = Vec::with_capacity(44 + samples.len() * 2);
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&(36 + data_len).to_le_bytes());
    b.extend_from_slice(b"WAVE");
    b.extend_from_slice(b"fmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes()); // PCM
    b.extend_from_slice(&1u16.to_le_bytes()); // mono
    b.extend_from_slice(&SR.to_le_bytes());
    b.extend_from_slice(&byte_rate.to_le_bytes());
    b.extend_from_slice(&2u16.to_le_bytes()); // block align
    b.extend_from_slice(&16u16.to_le_bytes()); // bits
    b.extend_from_slice(b"data");
    b.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        b.extend_from_slice(&s.to_le_bytes());
    }
    std::fs::write(path, b)
}

/// Read the `data` chunk of a 16-bit mono WAV into normalized f32 samples.
fn read_wav(path: &str) -> Vec<f32> {
    let bytes = std::fs::read(path).unwrap_or_default();
    let mut i = 12;
    while i + 8 <= bytes.len() {
        let id = &bytes[i..i + 4];
        let sz = u32::from_le_bytes([bytes[i + 4], bytes[i + 5], bytes[i + 6], bytes[i + 7]]) as usize;
        if id == b"data" {
            let start = i + 8;
            let end = (start + sz).min(bytes.len());
            let mut out = Vec::with_capacity((end - start) / 2);
            let mut j = start;
            while j + 1 < end {
                out.push(i16::from_le_bytes([bytes[j], bytes[j + 1]]) as f32 / 32768.0);
                j += 2;
            }
            return out;
        }
        i += 8 + sz + (sz & 1);
    }
    Vec::new()
}

fn goertzel(samples: &[f32], freq: f64) -> f64 {
    let coeff = 2.0 * (2.0 * PI * freq / SR as f64).cos();
    let (mut s1, mut s2) = (0.0f64, 0.0f64);
    for &x in samples {
        let s = x as f64 + coeff * s1 - s2;
        s2 = s1;
        s1 = s;
    }
    (s1 * s1 + s2 * s2 - coeff * s1 * s2).max(0.0).sqrt()
}

/// (detected, ratio). ratio = 18 kHz magnitude / 16.5 kHz reference over the recording.
fn analyze(samples: &[f32]) -> (bool, f64) {
    if samples.len() < 4800 {
        return (false, 0.0);
    }
    // skip the first ~120 ms (playback/AGC settling)
    let start = (SR as usize / 8).min(samples.len() / 4);
    let s = &samples[start..];
    let m18 = goertzel(s, TEST_FREQ);
    let m_ref = goertzel(s, REF_FREQ);
    let ratio = m18 / (m_ref + 1e-9);
    let m18_norm = m18 / s.len() as f64;
    // strong 18 kHz bin relative to the neighbor AND above an absolute floor
    let detected = ratio > 3.0 && m18_norm > 0.0002;
    eprintln!(
        "[soundcheck] loopback: n={} m18={:.4} mref={:.4} ratio={:.2} m18n={:.5} -> {}",
        s.len(), m18, m_ref, ratio, m18_norm, if detected { "HEARD" } else { "not heard" }
    );
    (detected, ratio)
}

unsafe fn set_session(category: &str, mode: &str, options: usize) {
    let session: *mut AnyObject = msg_send![class!(AVAudioSession), sharedInstance];
    if session.is_null() {
        return;
    }
    let cat = NSString::from_str(category);
    let md = NSString::from_str(mode);
    let _: Result<(), Retained<NSError>> =
        msg_send![session, setCategory: &*cat, mode: &*md, options: options, error: _];
    let _: Result<(), Retained<NSError>> = msg_send![session, setActive: true, error: _];
}

unsafe fn deactivate_session() {
    let session: *mut AnyObject = msg_send![class!(AVAudioSession), sharedInstance];
    if session.is_null() {
        return;
    }
    let _: Result<(), Retained<NSError>> = msg_send![session, setActive: false, error: _];
}

unsafe fn file_url(path: &str) -> Retained<AnyObject> {
    let s = NSString::from_str(path);
    msg_send![class!(NSURL), fileURLWithPath: &*s]
}

/// Run the loopback test. Returns immediately; `cb(detected, ratio)` fires ~0.7 s later on the
/// main thread. Needs mic permission (gate the caller on it).
pub fn run_loopback(cb: impl FnOnce(bool, f64) + 'static) {
    RESULT_CB.with(|c| *c.borrow_mut() = Some(Box::new(cb)));
    unsafe {
        // playAndRecord + Measurement mode = NO input processing (echo cancel would delete the
        // 18 kHz echo we're trying to detect). defaultToSpeaker (8) forces the loudspeaker.
        set_session("AVAudioSessionCategoryPlayAndRecord", "AVAudioSessionModeMeasurement", 8);

        let tone_path = tmp("sc_tone18k.wav");
        if !std::path::Path::new(&tone_path).exists() {
            let _ = write_wav(&tone_path, &gen_tone(TEST_FREQ, 1.2, 0.6));
        }
        let rec_path = tmp("sc_rec.wav");
        let _ = std::fs::remove_file(&rec_path);

        // player (loop the tone so it covers the whole record window)
        let tone_url = file_url(&tone_path);
        let palloc: Allocated<AnyObject> = msg_send![class!(AVAudioPlayer), alloc];
        let player_res: Result<Retained<AnyObject>, Retained<NSError>> =
            msg_send![palloc, initWithContentsOfURL: &*tone_url, error: _];
        let Ok(player) = player_res else {
            eprintln!("[soundcheck] AVAudioPlayer init failed");
            deactivate_session();
            RESULT_CB.with(|c| { if let Some(f) = c.borrow_mut().take() { f(false, 0.0); } });
            return;
        };
        // recorder → 48 kHz / 16-bit / mono LinearPCM WAV
        let settings: Retained<AnyObject> = msg_send![class!(NSMutableDictionary), dictionary];
        let mut put = |k: &str, v: Retained<AnyObject>| {
            let key = NSString::from_str(k);
            let _: () = msg_send![&*settings, setObject: &*v, forKey: &*key];
        };
        let n_lpcm: Retained<AnyObject> = msg_send![class!(NSNumber), numberWithUnsignedInt: 1819304813u32];
        put("AVFormatIDKey", n_lpcm); // kAudioFormatLinearPCM
        let n_sr: Retained<AnyObject> = msg_send![class!(NSNumber), numberWithDouble: 48000.0f64];
        put("AVSampleRateKey", n_sr);
        let n_ch: Retained<AnyObject> = msg_send![class!(NSNumber), numberWithInt: 1i32];
        put("AVNumberOfChannelsKey", n_ch);
        let n_bd: Retained<AnyObject> = msg_send![class!(NSNumber), numberWithInt: 16i32];
        put("AVLinearPCMBitDepthKey", n_bd);
        let n_fl: Retained<AnyObject> = msg_send![class!(NSNumber), numberWithBool: false];
        put("AVLinearPCMIsFloatKey", n_fl);
        let n_be: Retained<AnyObject> = msg_send![class!(NSNumber), numberWithBool: false];
        put("AVLinearPCMIsBigEndianKey", n_be);

        let rec_url = file_url(&rec_path);
        let ralloc: Allocated<AnyObject> = msg_send![class!(AVAudioRecorder), alloc];
        let recorder_res: Result<Retained<AnyObject>, Retained<NSError>> =
            msg_send![ralloc, initWithURL: &*rec_url, settings: &*settings, error: _];
        let Ok(recorder) = recorder_res else {
            eprintln!("[soundcheck] AVAudioRecorder init failed");
            deactivate_session();
            RESULT_CB.with(|c| { if let Some(f) = c.borrow_mut().take() { f(false, 0.0); } });
            return;
        };

        let _: bool = msg_send![&*recorder, prepareToRecord];
        let _: bool = msg_send![&*recorder, record];
        let _: () = msg_send![&*player, setNumberOfLoops: -1isize];
        let _: bool = msg_send![&*player, play];
        PLAYER.with(|p| *p.borrow_mut() = Some(player));
        RECORDER.with(|r| *r.borrow_mut() = Some(recorder));
    }
    slint::Timer::single_shot(std::time::Duration::from_millis(750), finish_loopback);
}

fn finish_loopback() {
    let rec_path = tmp("sc_rec.wav");
    unsafe {
        RECORDER.with(|r| {
            if let Some(rec) = r.borrow().as_ref() {
                let _: () = msg_send![&**rec, stop];
            }
        });
        PLAYER.with(|p| {
            if let Some(pl) = p.borrow().as_ref() {
                let _: () = msg_send![&**pl, stop];
            }
        });
        deactivate_session();
    }
    let (detected, ratio) = analyze(&read_wav(&rec_path));
    PLAYER.with(|p| *p.borrow_mut() = None);
    RECORDER.with(|r| *r.borrow_mut() = None);
    RESULT_CB.with(|c| {
        if let Some(f) = c.borrow_mut().take() {
            f(detected, ratio);
        }
    });
}

/// Play a short audible chime (for the headphone "did you hear that?" confirm).
pub fn play_chime() {
    unsafe {
        set_session("AVAudioSessionCategoryPlayback", "AVAudioSessionModeDefault", 0);
        let path = tmp("sc_chime.wav");
        if !std::path::Path::new(&path).exists() {
            let _ = write_wav(&path, &gen_chime());
        }
        let url = file_url(&path);
        let alloc: Allocated<AnyObject> = msg_send![class!(AVAudioPlayer), alloc];
        let player_res: Result<Retained<AnyObject>, Retained<NSError>> =
            msg_send![alloc, initWithContentsOfURL: &*url, error: _];
        if let Ok(player) = player_res {
            let _: bool = msg_send![&*player, play];
            PLAYER.with(|p| *p.borrow_mut() = Some(player)); // keep it alive until it finishes
        }
    }
}

/// Current media output volume, 0.0–1.0. 1.0 if the session can't be read (fail open).
pub fn output_volume() -> f32 {
    unsafe {
        let session: *mut AnyObject = msg_send![class!(AVAudioSession), sharedInstance];
        if session.is_null() {
            return 1.0;
        }
        let vol: f32 = msg_send![session, outputVolume];
        vol.clamp(0.0, 1.0)
    }
}

/// Audio output route: 0 = built-in speaker, 1 = private listening (headphones / Bluetooth /
/// AirPlay / CarPlay), 2 = other (earpiece receiver / unknown). Headphones win if mixed.
pub fn route() -> i32 {
    unsafe {
        let session: *mut AnyObject = msg_send![class!(AVAudioSession), sharedInstance];
        if session.is_null() {
            return 0;
        }
        let cur: *mut AnyObject = msg_send![session, currentRoute];
        if cur.is_null() {
            return 0;
        }
        let outputs: *mut AnyObject = msg_send![cur, outputs];
        if outputs.is_null() {
            return 0;
        }
        let count: usize = msg_send![outputs, count];
        let mut result = 2; // other/unknown by default
        for i in 0..count {
            let port: *mut AnyObject = msg_send![outputs, objectAtIndex: i];
            if port.is_null() {
                continue;
            }
            let ptype: *const NSString = msg_send![port, portType];
            if ptype.is_null() {
                continue;
            }
            let s = (*ptype).to_string();
            // Any private/external listening path → treat as headphones (loopback won't apply).
            if s.contains("Headphone")
                || s.contains("Bluetooth")
                || s.contains("AirPlay")
                || s.contains("CarAudio")
                || s.contains("USB")
                || s.contains("HDMI")
            {
                return 1;
            } else if s.contains("Speaker") {
                result = 0;
            }
        }
        result
    }
}

// ---- continuous / "live" loopback --------------------------------------------------------------
// Split the mic energy into the 18 kHz test tone (proxy for how loud the speaker — hence the agent
// — actually is at the mic) vs everything below it (the room noise floor). When the tone RMS clears
// the noise RMS by a margin, the user will hear Nova over the room. Re-measured ~3×/sec so the meter
// tracks the volume knob live.

fn analyze_live(samples: &[f32]) -> (f32, f32, f32) {
    if samples.len() < 2400 {
        return (0.0, 0.0, 0.0);
    }
    let start = (samples.len() / 6).min(2400); // skip recorder start-up ramp
    let s = &samples[start..];
    let n = s.len() as f64;
    let mag = goertzel(s, TEST_FREQ);
    let sig_rms = (2.0f64).sqrt() * mag / n; // RMS of the 18 kHz sinusoid
    let total_power: f64 = s.iter().map(|&x| (x as f64) * (x as f64)).sum::<f64>() / n;
    let noise_rms = (total_power - sig_rms * sig_rms).max(1e-12).sqrt(); // remove the tone
    let ratio = (sig_rms / (noise_rms + 1e-9)) as f32;
    (sig_rms as f32, noise_rms as f32, ratio)
}

/// Start continuous measurement: `cb(signal_rms, noise_rms, ratio)` fires ~3×/sec until stop_live().
/// Needs mic permission + the built-in speaker (gate the caller). Idempotent-ish; call stop first.
pub fn start_live(cb: impl Fn(f32, f32, f32) + 'static) {
    LIVE.with(|l| l.set(true));
    LIVE_CB.with(|c| *c.borrow_mut() = Some(Box::new(cb)));
    unsafe {
        set_session("AVAudioSessionCategoryPlayAndRecord", "AVAudioSessionModeMeasurement", 8);
        let tone_path = tmp("sc_tone18k.wav");
        if !std::path::Path::new(&tone_path).exists() {
            let _ = write_wav(&tone_path, &gen_tone(TEST_FREQ, 1.2, 0.6));
        }
        let tone_url = file_url(&tone_path);
        let palloc: Allocated<AnyObject> = msg_send![class!(AVAudioPlayer), alloc];
        let res: Result<Retained<AnyObject>, Retained<NSError>> =
            msg_send![palloc, initWithContentsOfURL: &*tone_url, error: _];
        if let Ok(player) = res {
            let _: () = msg_send![&*player, setNumberOfLoops: -1isize];
            let _: bool = msg_send![&*player, play];
            PLAYER.with(|p| *p.borrow_mut() = Some(player));
        }
    }
    measure_cycle();
}

fn measure_cycle() {
    if !LIVE.with(|l| l.get()) {
        return;
    }
    let rec_path = tmp("sc_rec.wav");
    let _ = std::fs::remove_file(&rec_path);
    unsafe {
        let settings: Retained<AnyObject> = msg_send![class!(NSMutableDictionary), dictionary];
        let mut put = |k: &str, v: Retained<AnyObject>| {
            let key = NSString::from_str(k);
            let _: () = msg_send![&*settings, setObject: &*v, forKey: &*key];
        };
        let a: Retained<AnyObject> = msg_send![class!(NSNumber), numberWithUnsignedInt: 1819304813u32];
        put("AVFormatIDKey", a);
        let b: Retained<AnyObject> = msg_send![class!(NSNumber), numberWithDouble: 48000.0f64];
        put("AVSampleRateKey", b);
        let c: Retained<AnyObject> = msg_send![class!(NSNumber), numberWithInt: 1i32];
        put("AVNumberOfChannelsKey", c);
        let d: Retained<AnyObject> = msg_send![class!(NSNumber), numberWithInt: 16i32];
        put("AVLinearPCMBitDepthKey", d);
        let e: Retained<AnyObject> = msg_send![class!(NSNumber), numberWithBool: false];
        put("AVLinearPCMIsFloatKey", e);
        let f: Retained<AnyObject> = msg_send![class!(NSNumber), numberWithBool: false];
        put("AVLinearPCMIsBigEndianKey", f);

        let rec_url = file_url(&rec_path);
        let ralloc: Allocated<AnyObject> = msg_send![class!(AVAudioRecorder), alloc];
        let res: Result<Retained<AnyObject>, Retained<NSError>> =
            msg_send![ralloc, initWithURL: &*rec_url, settings: &*settings, error: _];
        if let Ok(recorder) = res {
            let _: bool = msg_send![&*recorder, record];
            RECORDER.with(|r| *r.borrow_mut() = Some(recorder));
        } else {
            return;
        }
    }
    slint::Timer::single_shot(std::time::Duration::from_millis(280), || {
        unsafe {
            RECORDER.with(|r| {
                if let Some(rec) = r.borrow().as_ref() {
                    let _: () = msg_send![&**rec, stop];
                }
            });
        }
        let (sig, noise, ratio) = analyze_live(&read_wav(&tmp("sc_rec.wav")));
        RECORDER.with(|r| *r.borrow_mut() = None);
        LIVE_CB.with(|c| {
            if let Some(f) = c.borrow().as_ref() {
                f(sig, noise, ratio);
            }
        });
        if LIVE.with(|l| l.get()) {
            measure_cycle();
        }
    });
}

/// Stop continuous measurement + the tone, release the session.
pub fn stop_live() {
    LIVE.with(|l| l.set(false));
    LIVE_CB.with(|c| *c.borrow_mut() = None);
    unsafe {
        RECORDER.with(|r| {
            if let Some(rec) = r.borrow().as_ref() {
                let _: () = msg_send![&**rec, stop];
            }
        });
        PLAYER.with(|p| {
            if let Some(pl) = p.borrow().as_ref() {
                let _: () = msg_send![&**pl, stop];
            }
        });
        deactivate_session();
    }
    RECORDER.with(|r| *r.borrow_mut() = None);
    PLAYER.with(|p| *p.borrow_mut() = None);
}
