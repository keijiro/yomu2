//! Gemini TTS client (voice replication).

use base64::prelude::*;
use serde_json::{Value, json};
use std::process::{Command, Stdio};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

const API_BASE: &str = "https://generativelanguage.googleapis.com/v1beta";
const DEFAULT_KEYCHAIN_SERVICE: &str = "gemini-api-key";
const DEFAULT_SAMPLE_RATE: u32 = 24_000;
const MAX_RESPONSE_SIZE: u64 = 512 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct Request {
    pub model: String,
    pub voice: String,
    pub style: Option<String>,
    pub text: String,
}

/// Synthesizes speech and returns WAV file bytes. Blocking.
pub fn synthesize(req: &Request) -> Result<Vec<u8>, String> {
    let json = call("POST", "/interactions", Some(request_body(req)))?;
    let (data, mime) = extract_audio(&json)?;
    let bytes = BASE64_STANDARD
        .decode(data)
        .map_err(|e| format!("invalid audio data: {e}"))?;
    Ok(to_wav(bytes, mime))
}

/// Calls a Gemini API endpoint and returns the JSON response. Blocking.
pub fn call(method: &str, path: &str, body: Option<Value>) -> Result<Value, String> {
    let key = api_key()?;
    let url = format!("{API_BASE}{path}");
    let a = agent();
    let builder = match method {
        "GET" => a.get(&url).force_send_body(),
        "DELETE" => a.delete(&url).force_send_body(),
        _ => a.post(&url),
    };
    let mut res = builder
        .header("x-goog-api-key", key.as_str())
        .header("Content-Type", "application/json")
        .send(body.map(|b| b.to_string()).unwrap_or_default())
        .map_err(|e| format!("request failed: {e}"))?;
    let status = res.status();
    let raw = res
        .body_mut()
        .with_config()
        .limit(MAX_RESPONSE_SIZE)
        .read_to_vec()
        .map_err(|e| format!("failed to read response: {e}"))?;
    if !status.is_success() {
        return Err(format!("HTTP {}: {}", status.as_u16(), error_message(&raw)));
    }
    if raw.is_empty() {
        return Ok(Value::Null);
    }
    serde_json::from_slice(&raw).map_err(|e| format!("invalid response: {e}"))
}

fn agent() -> &'static ureq::Agent {
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    AGENT.get_or_init(|| {
        ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(300)))
            .http_status_as_error(false)
            .build()
            .into()
    })
}

/// Reads the API key from the macOS keychain (cached after the first call).
fn api_key() -> Result<String, String> {
    static KEY: Mutex<Option<String>> = Mutex::new(None);
    let mut cached = KEY.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(key) = cached.as_ref() {
        return Ok(key.clone());
    }
    let service = std::env::var("GEMINI_KEYCHAIN_SERVICE")
        .unwrap_or_else(|_| DEFAULT_KEYCHAIN_SERVICE.to_string());
    let out = Command::new("security")
        .args(["find-generic-password", "-s", &service, "-w"])
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("failed to run `security`: {e}"))?;
    let key = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if !out.status.success() || key.is_empty() {
        return Err(format!(
            "API key not found in keychain (service: '{service}')"
        ));
    }
    *cached = Some(key.clone());
    Ok(key)
}

fn request_body(req: &Request) -> Value {
    let mut content = json!({ "type": "text", "text": req.text });
    if let Some(style) = req.style.as_deref().filter(|s| !s.is_empty()) {
        content["annotations"] = json!([{ "type": "speech_metadata", "style": style }]);
    }
    json!({
        "model": req.model,
        "input": [{ "type": "user_input", "content": [content] }],
        "response_format": { "type": "audio" },
        "generation_config": { "speech_config": [{ "voice": req.voice }] },
    })
}

fn error_message(raw: &[u8]) -> String {
    serde_json::from_slice::<Value>(raw)
        .ok()
        .and_then(|v| v["error"]["message"].as_str().map(str::to_owned))
        .unwrap_or_else(|| truncate(&String::from_utf8_lossy(raw), 300))
}

/// Returns the base64 data and MIME type of the last audio item.
fn extract_audio(res: &Value) -> Result<(&str, Option<&str>), String> {
    let mut found = None;
    for step in res["steps"].as_array().into_iter().flatten() {
        if step["type"] != "model_output" {
            continue;
        }
        for item in step["content"].as_array().into_iter().flatten() {
            if item["type"] == "audio"
                && let Some(data) = item["data"].as_str()
            {
                let mime = item["mime_type"].as_str().or(item["mimeType"].as_str());
                found = Some((data, mime));
            }
        }
    }
    found.ok_or_else(|| format!("no audio in response: {}", truncate(&res.to_string(), 300)))
}

/// Passes WAV data through; wraps raw 16-bit mono PCM in a WAV header.
fn to_wav(data: Vec<u8>, mime: Option<&str>) -> Vec<u8> {
    let is_pcm = match mime {
        None => true,
        Some(m) => {
            let m = m.to_ascii_lowercase();
            m.contains("l16") || m.contains("pcm")
        }
    };
    if data.starts_with(b"RIFF") || !is_pcm {
        return data;
    }
    let rate = mime
        .and_then(|m| {
            m.split(';')
                .find_map(|p| p.trim().strip_prefix("rate=")?.parse().ok())
        })
        .unwrap_or(DEFAULT_SAMPLE_RATE);
    let len = data.len() as u32;
    let mut wav = Vec::with_capacity(44 + data.len());
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + len).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes()); // fmt chunk size
    wav.extend_from_slice(&1u16.to_le_bytes()); // PCM
    wav.extend_from_slice(&1u16.to_le_bytes()); // mono
    wav.extend_from_slice(&rate.to_le_bytes());
    wav.extend_from_slice(&(rate * 2).to_le_bytes()); // byte rate
    wav.extend_from_slice(&2u16.to_le_bytes()); // block align
    wav.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&len.to_le_bytes());
    wav.extend_from_slice(&data);
    wav
}

fn truncate(s: &str, max: usize) -> String {
    match s.char_indices().nth(max) {
        Some((i, _)) => format!("{}…", &s[..i]),
        None => s.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn body_includes_style_only_when_given() {
        let mut req = Request {
            model: "m".into(),
            voice: "voice_x".into(),
            style: None,
            text: "hi".into(),
        };
        let body = request_body(&req);
        assert!(body["input"][0]["content"][0].get("annotations").is_none());
        assert_eq!(
            body["generation_config"]["speech_config"][0]["voice"],
            "voice_x"
        );
        req.style = Some("calm".into());
        let body = request_body(&req);
        assert_eq!(
            body["input"][0]["content"][0]["annotations"][0]["style"],
            "calm"
        );
    }

    #[test]
    fn extracts_last_audio_item() {
        let res = json!({ "steps": [
            { "type": "user_input", "content": [{ "type": "audio", "data": "x" }] },
            { "type": "model_output", "content": [
                { "type": "audio", "data": "first" },
                { "type": "text", "text": "..." },
                { "type": "audio", "data": "last", "mime_type": "audio/L16;rate=16000" },
            ]},
        ]});
        assert_eq!(
            extract_audio(&res).unwrap(),
            ("last", Some("audio/L16;rate=16000"))
        );
        assert!(extract_audio(&json!({ "steps": [] })).is_err());
    }

    #[test]
    fn wav_wrapping() {
        let wav = b"RIFF....WAVE".to_vec();
        assert_eq!(to_wav(wav.clone(), Some("audio/wav")), wav);
        let out = to_wav(vec![0; 10], Some("audio/L16;codec=pcm;rate=16000"));
        assert_eq!(out.len(), 54);
        assert_eq!(&out[..4], b"RIFF");
        assert_eq!(u32::from_le_bytes(out[24..28].try_into().unwrap()), 16000);
        let out = to_wav(vec![0; 10], None);
        assert_eq!(u32::from_le_bytes(out[24..28].try_into().unwrap()), 24000);
    }
}
