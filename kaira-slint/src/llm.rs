//! Round-trip LLM step, routed through the lean server's `/chat` (the real Kaira
//! backend — it holds the OpenRouter key + model + persona/tool-calling). The app
//! just POSTs the user turn and gets a reply, so no LLM key lives on the device.
//!
//!   POST <lean>/chat  {"messages":[{"role":"user","content":"…"}]}  ->  {"reply":"…"}

use serde_json::json;

/// Convert the lean agent's `{{written|spoken}}` / `{{text}}` speech-norm markers
/// to plain spoken text (for display + TTS). Slices only at `{{`/`}}` (ASCII), so
/// UTF-8 stays intact.
pub(crate) fn normalize(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(start) = rest.find("{{") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        match after.find("}}") {
            Some(end) => {
                let inner = &after[..end];
                out.push_str(inner.rsplit('|').next().unwrap_or(inner)); // spoken part
                rest = &after[end + 2..];
            }
            None => {
                out.push_str(&rest[start..]);
                rest = "";
                break;
            }
        }
    }
    out.push_str(rest);
    out
}

/// Send `user_text` to the lean server's `/chat` and return the assistant reply.
pub async fn chat(lean_base: &str, user_text: &str) -> Result<String, String> {
    let url = format!("{}/chat", lean_base.trim_end_matches('/'));
    let body = json!({
        "messages": [{ "role": "user", "content": user_text }],
        "language": "en",
    });
    let resp = reqwest::Client::new()
        .post(&url)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("chat request: {e}"))?;
    let status = resp.status();
    let v: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("chat response: {e}"))?;
    if !status.is_success() {
        let msg = v["error"].as_str().unwrap_or("");
        return Err(format!("lean /chat HTTP {} {msg}", status.as_u16()));
    }
    let reply = normalize(v["reply"].as_str().unwrap_or("")).trim().to_string();
    if reply.is_empty() {
        Err("empty reply".into())
    } else {
        Ok(reply)
    }
}
