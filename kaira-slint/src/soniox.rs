//! Soniox cloud TTS client for the app. Streams synthesized audio (pcm_s16le)
//! from the Soniox real-time WebSocket and hands each chunk (as f32 samples) to
//! a callback, so the audio thread can append it to the rodio Sink live.

use base64::Engine as _;
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

const TTS_URL: &str = "wss://tts-rt.soniox.com/tts-websocket";
const STT_URL: &str = "wss://stt-rt.soniox.com/transcribe-websocket";
pub const SAMPLE_RATE: u32 = 24000;

/// Mint a short-lived Soniox key from the lean server (`GET /soniox/token`), so
/// the device never holds the long-lived key. Returns the temporary api_key.
pub async fn fetch_token(lean_base: &str) -> Result<String, String> {
    let url = format!("{}/soniox/token", lean_base.trim_end_matches('/'));
    let resp = reqwest::Client::new()
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("token request: {e}"))?;
    let status = resp.status();
    let v: Value = resp
        .json()
        .await
        .map_err(|e| format!("token response: {e}"))?;
    if !status.is_success() {
        let msg = v["error"].as_str().unwrap_or("");
        return Err(format!("lean /soniox/token HTTP {} {msg}", status.as_u16()));
    }
    v["api_key"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .ok_or_else(|| "no api_key in token response".to_string())
}

/// Transcribe captured mic audio (`samples` f32 @ `sample_rate`) with Soniox
/// real-time STT (ID+EN, auto language ID). Batches the push-to-talk buffer over
/// the WebSocket and returns the final transcript.
pub async fn transcribe(api_key: &str, samples: &[f32], sample_rate: u32) -> Result<String, String> {
    let (mut ws, _) = connect_async(STT_URL)
        .await
        .map_err(|e| format!("connect: {e}"))?;

    let config = json!({
        "api_key": api_key,
        "model": "stt-rt-v5",
        "audio_format": "pcm_s16le",
        "num_channels": 1,
        "sample_rate": sample_rate,
        "language_hints": ["id", "en"],
        "enable_language_identification": true,
    });
    ws.send(Message::Text(config.to_string()))
        .await
        .map_err(|e| e.to_string())?;

    // f32 -> pcm_s16le bytes
    let mut bytes = Vec::with_capacity(samples.len() * 2);
    for &s in samples {
        let v = (s.clamp(-1.0, 1.0) * 32767.0) as i16;
        bytes.extend_from_slice(&v.to_le_bytes());
    }
    for chunk in bytes.chunks(8192) {
        ws.send(Message::Binary(chunk.to_vec()))
            .await
            .map_err(|e| e.to_string())?;
    }
    ws.send(Message::Text(String::new())) // empty STRING = end-of-audio
        .await
        .map_err(|e| e.to_string())?;

    let mut text = String::new();
    while let Some(Ok(msg)) = ws.next().await {
        let Message::Text(t) = msg else { continue };
        let v: Value = serde_json::from_str(&t).unwrap_or(Value::Null);
        if v.get("error_code").is_some() {
            return Err(format!("{}: {}", v["error_code"], v["error_message"]));
        }
        if let Some(tokens) = v["tokens"].as_array() {
            for tok in tokens {
                if tok["is_final"].as_bool().unwrap_or(false) {
                    text.push_str(tok["text"].as_str().unwrap_or(""));
                }
            }
        }
        if v["finished"].as_bool().unwrap_or(false) {
            break;
        }
    }
    Ok(text.trim().to_string())
}

/// Synthesize `text` in `language` (e.g. "id") with `voice` (e.g. "Maya").
/// Calls `on_chunk(&[f32])` for each audio chunk as it streams in.
pub async fn synth(
    api_key: &str,
    text: &str,
    language: &str,
    voice: &str,
    mut on_chunk: impl FnMut(&[f32]),
) -> Result<(), String> {
    let (mut ws, _) = connect_async(TTS_URL)
        .await
        .map_err(|e| format!("connect: {e}"))?;

    let config = json!({
        "api_key": api_key,
        "model": "tts-rt-v1",
        "language": language,
        "voice": voice,
        "audio_format": "pcm_s16le",
        "sample_rate": SAMPLE_RATE,
        "stream_id": "kaira-app",
    });
    ws.send(Message::Text(config.to_string()))
        .await
        .map_err(|e| e.to_string())?;
    ws.send(Message::Text(
        json!({"text": text, "text_end": true, "stream_id": "kaira-app"}).to_string(),
    ))
    .await
    .map_err(|e| e.to_string())?;

    while let Some(Ok(msg)) = ws.next().await {
        let Message::Text(t) = msg else { continue };
        let v: Value = serde_json::from_str(&t).unwrap_or(Value::Null);
        if v.get("error_code").is_some() {
            return Err(format!("{}: {}", v["error_code"], v["error_message"]));
        }
        if let Some(b64) = v["audio"].as_str() {
            if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(b64) {
                let samples: Vec<f32> = bytes
                    .chunks_exact(2)
                    .map(|c| i16::from_le_bytes([c[0], c[1]]) as f32 / 32768.0)
                    .collect();
                on_chunk(&samples);
            }
        }
        if v["terminated"].as_bool().unwrap_or(false) {
            break;
        }
    }
    Ok(())
}
