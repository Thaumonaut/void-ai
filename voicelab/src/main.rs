//! Supertonic-3 TTS probe. Loads the sherpa-onnx OfflineTts Supertonic model
//! and synthesizes WAVs, sweeping speakers (to find "Sarah") and reporting the
//! real-time factor (RTF) — the number that matters for older hardware.
//!
//! Usage:
//!   cargo run --release -- [MODEL_DIR] [NUM_STEPS] [SPEED] [SID] [TEXT]
//! Defaults: model dir = the downloaded Supertonic-3, steps=6, speed=1.0,
//! sid=all speakers, a stock sentence.

use sherpa_onnx::{
    GenerationConfig, OfflineTts, OfflineTtsConfig, OfflineTtsModelConfig,
    OfflineTtsSupertonicModelConfig,
};
use std::collections::HashMap;
use std::time::Instant;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let model_dir = args
        .get(1)
        .cloned()
        .unwrap_or_else(|| "sherpa-onnx-supertonic-3-tts-int8-2026-05-11".to_string());
    let num_steps: i32 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(6);
    let speed: f32 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(1.0);
    let sid_arg: Option<i32> = args.get(4).and_then(|s| s.parse().ok());
    let text = args.get(5).cloned().unwrap_or_else(|| {
        "Hello, I'm Kaira. This is a test of the Supertonic voice running on device.".to_string()
    });

    let p = |f: &str| Some(format!("{model_dir}/{f}"));
    let config = OfflineTtsConfig {
        model: OfflineTtsModelConfig {
            supertonic: OfflineTtsSupertonicModelConfig {
                duration_predictor: p("duration_predictor.int8.onnx"),
                text_encoder: p("text_encoder.int8.onnx"),
                vector_estimator: p("vector_estimator.int8.onnx"),
                vocoder: p("vocoder.int8.onnx"),
                tts_json: p("tts.json"),
                unicode_indexer: p("unicode_indexer.bin"),
                voice_style: p("voice.bin"),
            },
            num_threads: 2,
            debug: false,
            ..Default::default()
        },
        ..Default::default()
    };

    let tts = OfflineTts::create(&config).expect("Failed to create OfflineTts");
    let sr = tts.sample_rate();
    let nspk = tts.num_speakers();
    println!("== Supertonic-3 loaded: sample_rate={sr}  num_speakers={nspk}  num_steps={num_steps}  speed={speed}");
    println!("== text: {text:?}");

    let sids: Vec<i32> = match sid_arg {
        Some(s) => vec![s],
        None => (0..nspk.min(12)).collect(),
    };

    let mut extra = HashMap::new();
    extra.insert("lang".to_string(), serde_json::json!("en"));

    for sid in sids {
        let gen_config = GenerationConfig {
            sid,
            num_steps,
            speed,
            extra: Some(extra.clone()),
            ..Default::default()
        };
        let t0 = Instant::now();
        let audio = tts
            .generate_with_config(&text, &gen_config, None::<fn(&[f32], f32) -> bool>)
            .expect("Generation failed");
        let elapsed = t0.elapsed().as_secs_f32();
        let dur = audio.samples().len() as f32 / audio.sample_rate() as f32;
        let rtf = elapsed / dur;
        let fname = format!("out_sid{sid}_steps{num_steps}.wav");
        let ok = audio.save(&fname);
        println!(
            "sid={sid:<2} -> {fname}  dur={dur:.2}s  gen={elapsed:.2}s  RTF={rtf:.3}  saved={ok}"
        );
    }
}
