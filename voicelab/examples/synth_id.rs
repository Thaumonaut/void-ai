//! Synthesize an Indonesian sentence with the Piper id_ID model and save it to
//! a wav, so we can feed the SAME audio into different ASR engines to compare
//! how each handles Bahasa Indonesia.
//!
//!   cargo run --example synth_id --release -- [OUT.wav] ["Indonesian text"]

use sherpa_onnx::{
    GenerationConfig, OfflineTts, OfflineTtsConfig, OfflineTtsModelConfig, OfflineTtsVitsModelConfig,
};

fn main() {
    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "id_sample.wav".to_string());
    let text = std::env::args().nth(2).unwrap_or_else(|| {
        "Halo, saya Kaira. Tolong periksa saldo saya dan kirim uang ke ibu saya sekarang."
            .to_string()
    });

    let piper = "vits-piper-id_ID-news_tts-medium-int8";
    let cfg = OfflineTtsConfig {
        model: OfflineTtsModelConfig {
            vits: OfflineTtsVitsModelConfig {
                model: Some(format!("{piper}/id_ID-news_tts-medium.onnx")),
                tokens: Some(format!("{piper}/tokens.txt")),
                data_dir: Some(format!("{piper}/espeak-ng-data")),
                ..Default::default()
            },
            num_threads: 4,
            ..Default::default()
        },
        ..Default::default()
    };
    let tts = OfflineTts::create(&cfg).expect("create piper id");
    let audio = tts
        .generate_with_config(
            &text,
            &GenerationConfig { sid: 0, speed: 1.0, ..Default::default() },
            None::<fn(&[f32], f32) -> bool>,
        )
        .expect("piper synth failed");
    let ok = audio.save(&out);
    let dur = audio.samples().len() as f32 / audio.sample_rate() as f32;
    println!("saved {out} ({dur:.2}s @ {}Hz, ok={ok})", audio.sample_rate());
    println!("text: {text}");
}
