//! Minimal FFI to the cross-compiled static libopus. Decoder (server Opus test +
//! realtime inbound audio) decodes raw Opus packets to 48 kHz PCM; Encoder
//! (realtime OUTBOUND: mic → Opus → WebRTC track) encodes 48 kHz mono f32 frames.

use std::os::raw::c_int;

#[repr(C)]
struct OpusDecoderT {
    _private: [u8; 0],
}

#[repr(C)]
struct OpusEncoderT {
    _private: [u8; 0],
}

extern "C" {
    fn opus_decoder_create(fs: i32, channels: c_int, error: *mut c_int) -> *mut OpusDecoderT;
    fn opus_decode_float(
        st: *mut OpusDecoderT,
        data: *const u8,
        len: i32,
        pcm: *mut f32,
        frame_size: c_int,
        decode_fec: c_int,
    ) -> c_int;
    fn opus_decoder_destroy(st: *mut OpusDecoderT);

    fn opus_encoder_create(
        fs: i32,
        channels: c_int,
        application: c_int,
        error: *mut c_int,
    ) -> *mut OpusEncoderT;
    fn opus_encode_float(
        st: *mut OpusEncoderT,
        pcm: *const f32,
        frame_size: c_int,
        data: *mut u8,
        max_data_bytes: i32,
    ) -> c_int;
    // MUST be declared variadic (`...`), not a fixed `value: i32`. opus_encoder_ctl is
    // C-variadic, and Apple's arm64 ABI passes variadic args on the STACK while fixed
    // args go in REGISTERS — so a fixed declaration makes opus read GARBAGE for every
    // request's value (bitrate/VBR/DTX all mis-set → the encoder emits 1-byte silence
    // packets for real speech). A true variadic call passes the value correctly.
    fn opus_encoder_ctl(st: *mut OpusEncoderT, request: c_int, ...) -> c_int;
    fn opus_encoder_destroy(st: *mut OpusEncoderT);
}

const OPUS_APPLICATION_VOIP: c_int = 2048;
const OPUS_SET_BITRATE_REQUEST: c_int = 4002;
const OPUS_SET_VBR_REQUEST: c_int = 4006;
const OPUS_SET_DTX_REQUEST: c_int = 4016;

/// Opus native sample rate — decode always yields 48 kHz.
pub const SAMPLE_RATE: u32 = 48_000;
/// Max samples a single packet can decode to (120 ms @ 48 kHz, mono).
const MAX_FRAME: usize = 5760;

pub struct Decoder {
    st: *mut OpusDecoderT,
}

// The decoder is used from a single thread (the audio thread); libopus state is
// self-contained. Safe to move across the channel boundary.
unsafe impl Send for Decoder {}

impl Decoder {
    pub fn new() -> Option<Self> {
        let mut err: c_int = 0;
        let st = unsafe { opus_decoder_create(SAMPLE_RATE as i32, 1, &mut err) };
        if st.is_null() || err != 0 {
            None
        } else {
            Some(Decoder { st })
        }
    }

    /// Decode one Opus packet, appending the mono f32 samples to `out`.
    /// Returns the number of samples decoded (negative = opus error).
    pub fn decode(&mut self, packet: &[u8], out: &mut Vec<f32>) -> i32 {
        let mut buf = [0f32; MAX_FRAME];
        let n = unsafe {
            opus_decode_float(
                self.st,
                packet.as_ptr(),
                packet.len() as i32,
                buf.as_mut_ptr(),
                MAX_FRAME as c_int,
                0,
            )
        };
        if n > 0 {
            out.extend_from_slice(&buf[..n as usize]);
        }
        n
    }
}

impl Drop for Decoder {
    fn drop(&mut self) {
        unsafe { opus_decoder_destroy(self.st) };
    }
}

/// Opus encoder for the realtime OUTBOUND path (48 kHz mono, VOIP).
pub struct Encoder {
    st: *mut OpusEncoderT,
}

unsafe impl Send for Encoder {}

impl Encoder {
    /// 48 kHz mono VOIP encoder at `bitrate` bps (e.g. 24_000).
    pub fn new(bitrate: i32) -> Option<Self> {
        let mut err: c_int = 0;
        let st = unsafe {
            opus_encoder_create(SAMPLE_RATE as i32, 1, OPUS_APPLICATION_VOIP, &mut err)
        };
        if st.is_null() || err != 0 {
            return None;
        }
        unsafe {
            opus_encoder_ctl(st, OPUS_SET_BITRATE_REQUEST, bitrate);
            // CBR + no DTX: always emit a full frame carrying the actual audio. Without
            // this the VBR encoder, after a long run of VPIO's perfectly-clean digital
            // silence, gets stuck emitting 1-byte "silence" packets even for real speech
            // (the bot then hears nothing). Android's cpal never sends pure silence — it
            // has mic background noise — which is why this only bit iOS.
            opus_encoder_ctl(st, OPUS_SET_VBR_REQUEST, 0);
            opus_encoder_ctl(st, OPUS_SET_DTX_REQUEST, 0);
        }
        Some(Encoder { st })
    }

    /// Encode one frame of 48 kHz mono f32 PCM (e.g. 960 samples = 20 ms) → Opus.
    pub fn encode(&mut self, pcm: &[f32]) -> Option<Vec<u8>> {
        let mut out = [0u8; 4000];
        let n = unsafe {
            opus_encode_float(self.st, pcm.as_ptr(), pcm.len() as c_int, out.as_mut_ptr(), 4000)
        };
        if n > 0 {
            Some(out[..n as usize].to_vec())
        } else {
            None
        }
    }
}

impl Drop for Encoder {
    fn drop(&mut self) {
        unsafe { opus_encoder_destroy(self.st) };
    }
}
