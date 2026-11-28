//! `yomu voice ...` subcommands: manage replicated voices.

use crate::tts;
use base64::prelude::*;
use serde_json::{Value, json};
use std::fs;
use std::path::Path;
use std::process::Command;

pub const USAGE: &str = "\
Usage:
  yomu voice register --source FILE --consent FILE [--name NAME] [--model MODEL] [--stateless]
  yomu voice list
  yomu voice delete VOICE_ID

register  Create a replicated voice from a reference recording (10-30 s) and
          a recording of the consent statement. Prints the voice ID to put
          in the `voice:` field of the script.
          --stateless issues a 7-day voicekey_... instead of a stored voice_...
list      List replicated voices in the project.
delete    Delete a stored voice (cannot be undone).
";

pub fn run(args: &[String]) -> Result<(), String> {
    match args.first().map(String::as_str) {
        Some("register") => register(&args[1..]),
        Some("list") if args.len() == 1 => list(),
        Some("delete") if args.len() == 2 => delete(&args[1]),
        _ => Err(format!("invalid arguments\n\n{USAGE}")),
    }
}

fn register(args: &[String]) -> Result<(), String> {
    let (mut source, mut consent) = (None, None);
    let mut name = "my-voice".to_string();
    let mut model = crate::script::DEFAULT_MODEL.to_string();
    let mut stateless = false;
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        let mut value = |opt: &str| {
            it.next()
                .cloned()
                .ok_or_else(|| format!("{opt} requires a value"))
        };
        match arg.as_str() {
            "--source" => source = Some(value("--source")?),
            "--consent" => consent = Some(value("--consent")?),
            "--name" => name = value("--name")?,
            "--model" => model = value("--model")?,
            "--stateless" => stateless = true,
            _ => return Err(format!("unknown option: {arg}\n\n{USAGE}")),
        }
    }
    let source = source.ok_or("--source is required")?;
    let consent = consent.ok_or("--consent is required")?;

    let body = json!({
        "store": !stateless,
        "voice": {
            "model": model,
            "type": "replicated",
            "display_name": name,
            "replicated": {
                "source_audio": { "mime_type": "audio/wav", "data": wav_base64(&source)? },
                "consent_audio": { "mime_type": "audio/wav", "data": wav_base64(&consent)? },
            },
        },
    });
    let res = tts::call("POST", "/voices", Some(body))?;
    let id = find_voice_id(&res)
        .ok_or_else(|| format!("no voice ID in response: {res}"))?;
    println!("voice: {id}");
    Ok(())
}

fn list() -> Result<(), String> {
    let res = tts::call("GET", "/voices?type=replicated", None)?;
    println!("{}", serde_json::to_string_pretty(&res).unwrap_or_default());
    Ok(())
}

fn delete(id: &str) -> Result<(), String> {
    tts::call("DELETE", &format!("/voices/{id}"), None)?;
    println!("deleted {id}");
    Ok(())
}

/// Converts any audio file afconvert can read to 24 kHz mono 16-bit WAV
/// and returns it base64-encoded.
fn wav_base64(path: &str) -> Result<String, String> {
    if !Path::new(path).is_file() {
        return Err(format!("file not found: {path}"));
    }
    let tmp = std::env::temp_dir().join(format!("yomu-{}.wav", std::process::id()));
    let status = Command::new("afconvert")
        .args(["-f", "WAVE", "-d", "LEI16@24000", "-c", "1", path])
        .arg(&tmp)
        .status()
        .map_err(|e| format!("failed to run `afconvert`: {e}"))?;
    let data = fs::read(&tmp);
    let _ = fs::remove_file(&tmp);
    if !status.success() {
        return Err(format!("afconvert failed to convert {path}"));
    }
    let data = data.map_err(|e| format!("cannot read converted audio: {e}"))?;
    Ok(BASE64_STANDARD.encode(data))
}

/// Finds the first `voice_...` / `voicekey_...` string anywhere in a response.
fn find_voice_id(v: &Value) -> Option<&str> {
    match v {
        Value::String(s) if s.starts_with("voice_") || s.starts_with("voicekey_") => Some(s),
        Value::Array(a) => a.iter().find_map(find_voice_id),
        Value::Object(o) => o.values().find_map(find_voice_id),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_nested_voice_id() {
        let v = json!({ "voice": { "name": "x", "ids": [1, "voice_abc"] } });
        assert_eq!(find_voice_id(&v), Some("voice_abc"));
        assert_eq!(find_voice_id(&json!({ "a": "b" })), None);
    }
}
