//! iOS native **Voice-Processing I/O** audio unit: mic capture + speaker playback with
//! hardware acoustic-echo-cancellation + noise suppression, at 48 kHz mono Float32.
//!
//! Replaces cpal (input) + rodio (output) on iOS. cpal's plain RemoteIO can't do AEC/NS,
//! so a background fan swamps the server VAD (turns never end) and the bot's own audio
//! echoes back into the mic. VPIO fixes both in hardware — and because it cancels the
//! echo, iOS doesn't need the software half-duplex gate (→ real barge-in).

use std::collections::VecDeque;
use std::os::raw::c_void;
use std::sync::{Arc, Mutex};

use coreaudio_sys::*;

const OUTPUT_BUS: u32 = 0; // element 0 = to the speaker
const INPUT_BUS: u32 = 1; // element 1 = from the mic
const SR: f64 = 48_000.0;

struct Ctx {
    unit: AudioUnit,
    mic_buf: Arc<Mutex<Vec<f32>>>,
    play_q: Arc<Mutex<VecDeque<f32>>>,
    mic_level: Arc<Mutex<f32>>,
    cb: std::sync::atomic::AtomicU64,   // input-callback counter (heartbeat log)
    peak: std::sync::atomic::AtomicU32, // running peak (f32 bits) between heartbeats
}

/// Live unit handle. `stop()` tears the unit down and frees the callback context.
pub struct Vpio {
    unit: AudioUnit,
    ctx: *mut Ctx,
}
// AudioUnit is a raw pointer; the unit is only touched from run_session's thread + the
// audio callbacks (which hold their own ref). Safe to move the handle across threads.
unsafe impl Send for Vpio {}

impl Vpio {
    /// Explicit teardown. The real work is in `Drop`, so an ABORTED session task — whose
    /// future is dropped mid-flight when a hung `write_sample` trips the controller's 6s
    /// watchdog — still tears the unit down. Without that, the leaked unit keeps capturing
    /// and the hardware mic (orange indicator) stays live until the app is force-quit.
    pub fn stop(self) {
        // `self` falls out of scope here → `Drop` runs the teardown.
    }

    /// Hard-mute: enable/disable the mic INPUT element without tearing the unit down, so the
    /// hardware mic (and the orange indicator) actually turns off while playback keeps running
    /// (Nova stays audible). `EnableIO` is a pre-initialize property, so the unit has to be
    /// stopped + uninitialized to change it, then re-initialized + restarted. There's a brief
    /// (~tens of ms) output gap across the toggle, which is fine for a mute button.
    pub fn set_input_enabled(&self, on: bool) {
        unsafe {
            AudioOutputUnitStop(self.unit);
            AudioUnitUninitialize(self.unit);
            let v: u32 = on as u32;
            let r = AudioUnitSetProperty(
                self.unit,
                kAudioOutputUnitProperty_EnableIO,
                kAudioUnitScope_Input,
                INPUT_BUS,
                &v as *const _ as *const c_void,
                std::mem::size_of::<u32>() as u32,
            );
            let i = AudioUnitInitialize(self.unit);
            let s = AudioOutputUnitStart(self.unit);
            eprintln!("[vpio] set_input_enabled({on}) → setprop={r} init={i} start={s} (0=ok)");
        }
    }
}

impl Drop for Vpio {
    fn drop(&mut self) {
        // Tear down THIS attempt's unit (releases the hardware mic). The audio SESSION is
        // NOT deactivated here — it's per-connection, not per-attempt: on a reconnect the
        // next attempt reuses the live session, so deactivating here would race the new
        // attempt's mic and kill capture. The controller deactivates once, on user hang-up.
        unsafe {
            AudioOutputUnitStop(self.unit);
            AudioUnitUninitialize(self.unit);
            AudioComponentInstanceDispose(self.unit);
            drop(Box::from_raw(self.ctx));
        }
    }
}

fn asbd() -> AudioStreamBasicDescription {
    AudioStreamBasicDescription {
        mSampleRate: SR,
        mFormatID: kAudioFormatLinearPCM as u32,
        mFormatFlags: kAudioFormatFlagIsFloat as u32 | kAudioFormatFlagIsPacked as u32,
        mBytesPerPacket: 4,
        mFramesPerPacket: 1,
        mBytesPerFrame: 4,
        mChannelsPerFrame: 1,
        mBitsPerChannel: 32,
        mReserved: 0,
    }
}

/// Build + start the VPIO unit. `play_q` is drained to the speaker (filled by the inbound
/// RTP decoder); `mic_buf` is filled from the mic (drained by the outbound Opus pump);
/// `mic_level` gets the per-callback peak for the UI meter.
pub fn start(
    play_q: Arc<Mutex<VecDeque<f32>>>,
    mic_buf: Arc<Mutex<Vec<f32>>>,
    mic_level: Arc<Mutex<f32>>,
) -> Option<Vpio> {
    unsafe {
        let desc = AudioComponentDescription {
            componentType: kAudioUnitType_Output as u32,
            componentSubType: kAudioUnitSubType_VoiceProcessingIO as u32,
            componentManufacturer: kAudioUnitManufacturer_Apple as u32,
            componentFlags: 0,
            componentFlagsMask: 0,
        };
        let comp = AudioComponentFindNext(std::ptr::null_mut(), &desc);
        if comp.is_null() {
            eprintln!("[vpio] VoiceProcessingIO component not found");
            return None;
        }
        let mut unit: AudioUnit = std::ptr::null_mut();
        if AudioComponentInstanceNew(comp, &mut unit) != 0 || unit.is_null() {
            eprintln!("[vpio] AudioComponentInstanceNew failed");
            return None;
        }

        let enable: u32 = 1;
        let u32sz = std::mem::size_of::<u32>() as u32;
        AudioUnitSetProperty(
            unit,
            kAudioOutputUnitProperty_EnableIO,
            kAudioUnitScope_Output,
            OUTPUT_BUS,
            &enable as *const _ as *const c_void,
            u32sz,
        );
        AudioUnitSetProperty(
            unit,
            kAudioOutputUnitProperty_EnableIO,
            kAudioUnitScope_Input,
            INPUT_BUS,
            &enable as *const _ as *const c_void,
            u32sz,
        );

        let fmt = asbd();
        let fsz = std::mem::size_of::<AudioStreamBasicDescription>() as u32;
        // mic → us: the OUTPUT scope of the INPUT bus.
        let s1 = AudioUnitSetProperty(
            unit,
            kAudioUnitProperty_StreamFormat,
            kAudioUnitScope_Output,
            INPUT_BUS,
            &fmt as *const _ as *const c_void,
            fsz,
        );
        // us → speaker: the INPUT scope of the OUTPUT bus.
        let s2 = AudioUnitSetProperty(
            unit,
            kAudioUnitProperty_StreamFormat,
            kAudioUnitScope_Input,
            OUTPUT_BUS,
            &fmt as *const _ as *const c_void,
            fsz,
        );
        eprintln!("[vpio] set stream format 48k/mono/f32 → in={s1} out={s2} (0=ok)");
        // Read back what the unit ACTUALLY negotiated for the mic bus — if the 48k/mono
        // format didn't take, the callback samples are at the wrong rate/channels and the
        // bot hears garbage/silence (no transcription).
        let mut got = fmt;
        let mut gsz = fsz;
        let gs = AudioUnitGetProperty(
            unit,
            kAudioUnitProperty_StreamFormat,
            kAudioUnitScope_Output,
            INPUT_BUS,
            &mut got as *mut _ as *mut c_void,
            &mut gsz,
        );
        eprintln!(
            "[vpio] mic bus negotiated: {}Hz {}ch flags={:#x} (get={gs})",
            got.mSampleRate, got.mChannelsPerFrame, got.mFormatFlags
        );

        let ctx = Box::into_raw(Box::new(Ctx {
            unit,
            mic_buf,
            play_q,
            mic_level,
            cb: std::sync::atomic::AtomicU64::new(0),
            peak: std::sync::atomic::AtomicU32::new(0),
        }));
        let cbsz = std::mem::size_of::<AURenderCallbackStruct>() as u32;

        let in_cb = AURenderCallbackStruct {
            inputProc: Some(input_cb),
            inputProcRefCon: ctx as *mut c_void,
        };
        AudioUnitSetProperty(
            unit,
            kAudioOutputUnitProperty_SetInputCallback,
            kAudioUnitScope_Global,
            INPUT_BUS,
            &in_cb as *const _ as *const c_void,
            cbsz,
        );

        let render_cb = AURenderCallbackStruct {
            inputProc: Some(render_cb),
            inputProcRefCon: ctx as *mut c_void,
        };
        AudioUnitSetProperty(
            unit,
            kAudioUnitProperty_SetRenderCallback,
            kAudioUnitScope_Input,
            OUTPUT_BUS,
            &render_cb as *const _ as *const c_void,
            cbsz,
        );

        if AudioUnitInitialize(unit) != 0 {
            eprintln!("[vpio] AudioUnitInitialize failed");
            return None;
        }
        if AudioOutputUnitStart(unit) != 0 {
            eprintln!("[vpio] AudioOutputUnitStart failed");
            return None;
        }
        eprintln!("[vpio] started — 48kHz mono, hardware AEC + noise suppression");
        Some(Vpio { unit, ctx })
    }
}

// Mic frames ready → render them out of the unit and into mic_buf (+ UI level peak).
unsafe extern "C" fn input_cb(
    ref_con: *mut c_void,
    flags: *mut AudioUnitRenderActionFlags,
    ts: *const AudioTimeStamp,
    bus: u32,
    n_frames: u32,
    _io: *mut AudioBufferList,
) -> OSStatus {
    let ctx = &*(ref_con as *const Ctx);
    let mut samples = vec![0f32; n_frames as usize];
    let mut abl = AudioBufferList {
        mNumberBuffers: 1,
        mBuffers: [AudioBuffer {
            mNumberChannels: 1,
            mDataByteSize: n_frames * 4,
            mData: samples.as_mut_ptr() as *mut c_void,
        }],
    };
    let status = AudioUnitRender(ctx.unit, flags, ts, bus, n_frames, &mut abl);
    if status == 0 {
        use std::sync::atomic::Ordering;
        let peak = samples.iter().fold(0f32, |a, &x| a.max(x.abs()));
        if let Ok(mut g) = ctx.mic_level.lock() {
            if peak > *g {
                *g = peak;
            }
        }
        // running peak across this heartbeat window
        let prev = f32::from_bits(ctx.peak.load(Ordering::Relaxed));
        if peak > prev {
            ctx.peak.store(peak.to_bits(), Ordering::Relaxed);
        }
        if let Ok(mut b) = ctx.mic_buf.lock() {
            b.extend_from_slice(&samples);
        }
        // Heartbeat ~1×/s: frames per callback + the loudest sample. peak≈0 while you
        // talk = the mic/format is dead; a healthy peak = capture works (look downstream).
        let n = ctx.cb.fetch_add(1, Ordering::Relaxed);
        if n % 100 == 0 {
            let hb = f32::from_bits(ctx.peak.swap(0, Ordering::Relaxed));
            eprintln!("[vpio] mic alive: cb#{n}, {n_frames} frames/cb, peak≈{hb:.3}");
        }
    } else if ctx.cb.fetch_add(1, std::sync::atomic::Ordering::Relaxed) % 100 == 0 {
        eprintln!("[vpio] AudioUnitRender err={status}");
    }
    status
}

// Speaker wants n_frames → fill from the playback queue (silence when empty).
unsafe extern "C" fn render_cb(
    ref_con: *mut c_void,
    _flags: *mut AudioUnitRenderActionFlags,
    _ts: *const AudioTimeStamp,
    _bus: u32,
    _n_frames: u32,
    io: *mut AudioBufferList,
) -> OSStatus {
    let ctx = &*(ref_con as *const Ctx);
    let abl = &mut *io;
    if abl.mNumberBuffers >= 1 {
        let buf = &mut abl.mBuffers[0];
        let out = std::slice::from_raw_parts_mut(buf.mData as *mut f32, (buf.mDataByteSize / 4) as usize);
        match ctx.play_q.lock() {
            Ok(mut q) => {
                for s in out.iter_mut() {
                    *s = q.pop_front().unwrap_or(0.0);
                }
            }
            Err(_) => {
                for s in out.iter_mut() {
                    *s = 0.0;
                }
            }
        }
    }
    0
}
