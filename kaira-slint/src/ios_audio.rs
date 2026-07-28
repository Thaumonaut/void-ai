//! iOS AVAudioSession activation. In a pure-Slint app there's no Swift AppDelegate,
//! so cpal's mic capture returns silence until the process audio session is set to
//! `playAndRecord` and made active. Call once before starting the realtime session.

use objc2_avf_audio::{AVAudioSession, AVAudioSessionCategoryOptions, AVAudioSessionPortOverride};

/// Configure the shared audio session for the Voice-Processing I/O unit (see ios_vpio):
/// playAndRecord + voiceChat mode, 48 kHz, forced to the main speaker. Call before the
/// unit starts. Best-effort — logs but never panics if the session rejects a setting.
pub fn activate() {
    unsafe {
        let session = AVAudioSession::sharedInstance();
        let opts = AVAudioSessionCategoryOptions::DefaultToSpeaker
            | AVAudioSessionCategoryOptions::AllowBluetooth;
        // playAndRecord + voiceChat = the VoIP profile the VPIO unit expects. (VPIO does
        // the actual AEC/NS; the mode just sets the session up for it.) Category/mode
        // constants are extern statics (Option — null if AVFAudio is absent).
        let cat = objc2_avf_audio::AVAudioSessionCategoryPlayAndRecord;
        let mode = objc2_avf_audio::AVAudioSessionModeVoiceChat;
        match (cat, mode) {
            (Some(cat), Some(mode)) => {
                if let Err(e) = session.setCategory_mode_options_error(cat, mode, opts) {
                    eprintln!("[ios-audio] setCategory(voiceChat) failed: {e:?}");
                }
            }
            (Some(cat), None) => {
                let _ = session.setCategory_withOptions_error(cat, opts);
            }
            _ => {}
        }
        // Prefer 48 kHz so the VPIO client format matches the pipeline (Opus 48k) with no
        // resampling. Best-effort — iOS may pick a nearby rate.
        let _ = session.setPreferredSampleRate_error(48_000.0);
        if let Err(e) = session.setActive_error(true) {
            eprintln!("[ios-audio] setActive failed: {e:?}");
        }
        // voiceChat mode defaults to the EARPIECE (phone-call routing) — force the main
        // speaker for a hands-free agent. Must be after setActive.
        if let Err(e) = session.overrideOutputAudioPort_error(AVAudioSessionPortOverride::Speaker) {
            eprintln!("[ios-audio] overrideOutputAudioPort(speaker) failed: {e:?}");
        }
    }
}

/// Release the shared audio session when a call ends. Without this the `playAndRecord`
/// session stays active after the VPIO unit stops, so iOS keeps the mic reserved and the
/// orange in-use indicator lit — even when the app is idle or backgrounded. Call on
/// realtime teardown; the next `activate()` brings it back. `NotifyOthersOnDeactivation`
/// lets any other app's audio resume. Best-effort — logs but never panics.
pub fn deactivate() {
    unsafe {
        let session = AVAudioSession::sharedInstance();
        if let Err(e) = session.setActive_withOptions_error(
            false,
            objc2_avf_audio::AVAudioSessionSetActiveOptions::NotifyOthersOnDeactivation,
        ) {
            eprintln!("[ios-audio] setActive(false) failed: {e:?}");
        }
    }
}
