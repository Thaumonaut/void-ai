//! Native Soniox real-time STT client — proves the Rust/Android client can reach
//! Soniox. Sends a wav (audio_format="auto" lets Soniox read the header, so no
//! client-side resampling) and prints the transcript + language timeline.
//!
//!   SONIOX_API_KEY=... stt <wav>
//!
//! Runs on host and, cross-compiled with cargo-ndk, on Android arm64.

use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

const STT_URL: &str = "wss://stt-rt.soniox.com/transcribe-websocket";

#[tokio::main]
async fn main() {
    let path = std::env::args().nth(1).expect("usage: stt <wav>");
    let key = std::env::var("SONIOX_API_KEY").expect("set SONIOX_API_KEY");
    let audio = std::fs::read(&path).expect("read wav");
    println!("→ {path}  ({} KB)  model=stt-rt-v5  hints=id,en", audio.len() / 1024);

    let (mut ws, _) = connect_async(STT_URL).await.expect("connect");

    let config = json!({
        "api_key": key,
        "model": "stt-rt-v5",
        "audio_format": "auto",
        "language_hints": ["id", "en"],
        "enable_language_identification": true,
    });
    ws.send(Message::Text(config.to_string())).await.unwrap();

    // Send the whole wav (header + samples) in chunks, then the empty-string
    // end-of-audio signal.
    for chunk in audio.chunks(8192) {
        ws.send(Message::Binary(chunk.to_vec())).await.unwrap();
    }
    ws.send(Message::Text(String::new())).await.unwrap();

    let mut transcript = String::new();
    let mut langs: Vec<String> = Vec::new();
    while let Some(Ok(msg)) = ws.next().await {
        let Message::Text(t) = msg else { continue };
        let v: Value = serde_json::from_str(&t).unwrap_or(Value::Null);
        if v.get("error_code").is_some() {
            println!("⚠ error {}: {}", v["error_code"], v["error_message"]);
            break;
        }
        if let Some(tokens) = v["tokens"].as_array() {
            for tok in tokens {
                if tok["is_final"].as_bool().unwrap_or(false) {
                    transcript.push_str(tok["text"].as_str().unwrap_or(""));
                    if let Some(l) = tok["language"].as_str() {
                        if !l.is_empty() && langs.last().map(String::as_str) != Some(l) {
                            langs.push(l.to_string());
                        }
                    }
                }
            }
        }
        if v["finished"].as_bool().unwrap_or(false) {
            break;
        }
    }

    println!("transcript: {}", transcript.trim());
    println!("languages : {}", langs.join(" → "));
}
