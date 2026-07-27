//! On-device TTS synth probe for the compare tool. Synthesizes text with Piper
//! or Supertonic, streaming so it can report time-to-first-audio, and writes a
//! wav. Prints one machine-parseable RESULT line.
//!
//!   cargo run --example tts_synth --release -- <piper|supertonic> <model_dir> "<text>" <out.wav> [lang]

use sherpa_onnx::{
    GenerationConfig, OfflineTts, OfflineTtsConfig, OfflineTtsModelConfig,
    OfflineTtsSupertonicModelConfig, OfflineTtsVitsModelConfig,
};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Instant;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let engine = a.get(1).map(|s| s.as_str()).unwrap_or("piper");
    let dir = a.get(2).cloned().expect("need <model_dir>");
    let text = a.get(3).cloned().expect("need <text>");
    let out = a.get(4).cloned().unwrap_or_else(|| "out.wav".to_string());
    let lang = a.get(5).cloned().unwrap_or_else(|| "en".to_string());

    let mut mc = OfflineTtsModelConfig { num_threads: 4, debug: false, ..Default::default() };
    let gen: GenerationConfig;
    if engine == "supertonic" {
        let p = |f: &str| Some(format!("{dir}/{f}"));
        mc.supertonic = OfflineTtsSupertonicModelConfig {
            duration_predictor: p("duration_predictor.int8.onnx"),
            text_encoder: p("text_encoder.int8.onnx"),
            vector_estimator: p("vector_estimator.int8.onnx"),
            vocoder: p("vocoder.int8.onnx"),
            tts_json: p("tts.json"),
            unicode_indexer: p("unicode_indexer.bin"),
            voice_style: p("voice.bin"),
        };
        let mut extra = std::collections::HashMap::new();
        extra.insert("lang".to_string(), serde_json::json!(lang));
        gen = GenerationConfig { sid: 5, num_steps: 6, speed: 1.0, extra: Some(extra), ..Default::default() };
    } else {
        // piper VITS: model file name = last path segment + .onnx
        let name = dir.rsplit('/').next().unwrap_or(&dir).trim_start_matches("vits-piper-");
        let model = if std::path::Path::new(&format!("{dir}/{name}.onnx")).exists() {
            format!("{dir}/{name}.onnx")
        } else {
            // fall back: first .onnx in dir
            std::fs::read_dir(&dir).unwrap().filter_map(|e| e.ok())
                .map(|e| e.path()).find(|p| p.extension().map_or(false, |x| x == "onnx"))
                .unwrap().to_string_lossy().to_string()
        };
        mc.vits = OfflineTtsVitsModelConfig {
            model: Some(model),
            tokens: Some(format!("{dir}/tokens.txt")),
            data_dir: Some(format!("{dir}/espeak-ng-data")),
            ..Default::default()
        };
        gen = GenerationConfig { sid: 0, speed: 1.0, ..Default::default() };
    }

    let tts = OfflineTts::create(&OfflineTtsConfig { model: mc, ..Default::default() })
        .expect("create OfflineTts");
    let sr = tts.sample_rate() as u32;

    let started = Instant::now();
    let ttfa = Rc::new(Cell::new(0f32));
    let acc = Rc::new(RefCell::new(Vec::<f32>::new()));
    {
        let f = ttfa.clone();
        let a2 = acc.clone();
        let t0 = started;
        let cb = move |chunk: &[f32], _p: f32| -> bool {
            if a2.borrow().is_empty() {
                f.set(t0.elapsed().as_secs_f32());
            }
            a2.borrow_mut().extend_from_slice(chunk);
            true
        };
        tts.generate_with_config(&text, &gen, Some(cb));
    }
    let total = started.elapsed().as_secs_f32();
    let samples = acc.borrow();
    let dur = samples.len() as f32 / sr as f32;

    let spec = hound::WavSpec { channels: 1, sample_rate: sr, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
    let mut w = hound::WavWriter::create(&out, spec).expect("wav");
    for &s in samples.iter() {
        w.write_sample((s.clamp(-1.0, 1.0) * 32767.0) as i16).unwrap();
    }
    w.finalize().unwrap();

    println!(
        "RESULT engine={engine} first={:.2} total={total:.2} dur={dur:.2} rtf={:.3} out={out}",
        ttfa.get(),
        total / dur.max(0.001)
    );
}
