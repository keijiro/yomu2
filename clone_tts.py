#!/usr/bin/env python3
"""Gemini 3.8 TTS voice replication (voice cloning) one-shot tool.

Uses only the Python standard library and the macOS `security` / `afconvert`
commands. The API key is read from the macOS keychain.

Subcommands:
  register  Create a replicated voice from source + consent recordings
  speak     Synthesize speech with a replicated voice (voice_... / voicekey_...)
  list      List replicated voices in the project
  delete    Delete a stored replicated voice
"""

import argparse
import base64
import json
import os
import subprocess
import sys
import tempfile
import urllib.error
import urllib.request
from pathlib import Path

API_BASE = "https://generativelanguage.googleapis.com/v1beta"
DEFAULT_MODEL = "gemini-3.8-flash-tts"
KEYCHAIN_SERVICE = os.environ.get("GEMINI_KEYCHAIN_SERVICE", "gemini-api-key")
VOICE_FILE = Path(__file__).with_name(".voice_id")


# --- Helpers -----------------------------------------------------------------

def die(msg):
    print(f"error: {msg}", file=sys.stderr)
    sys.exit(1)


def get_api_key():
    try:
        out = subprocess.run(
            ["security", "find-generic-password", "-s", KEYCHAIN_SERVICE, "-w"],
            check=True, capture_output=True, text=True)
    except subprocess.CalledProcessError:
        die(f"API key not found in keychain (service: '{KEYCHAIN_SERVICE}'). "
            "See README.md for how to register it.")
    return out.stdout.strip()


def request(method, path, body=None):
    data = json.dumps(body).encode() if body is not None else None
    req = urllib.request.Request(f"{API_BASE}{path}", data=data, method=method)
    req.add_header("x-goog-api-key", get_api_key())
    req.add_header("Content-Type", "application/json")
    try:
        with urllib.request.urlopen(req, timeout=300) as res:
            raw = res.read()
    except urllib.error.HTTPError as e:
        die(f"HTTP {e.code}: {e.read().decode(errors='replace')}")
    return json.loads(raw) if raw else {}


def to_wav24k_b64(path):
    """Convert any audio file afconvert can read to 24 kHz mono 16-bit WAV."""
    if not Path(path).is_file():
        die(f"file not found: {path}")
    with tempfile.TemporaryDirectory() as tmp:
        out = Path(tmp) / "converted.wav"
        subprocess.run(
            ["afconvert", "-f", "WAVE", "-d", "LEI16@24000", "-c", "1",
             str(path), str(out)], check=True)
        return base64.b64encode(out.read_bytes()).decode()


def find_voice_ids(obj):
    """Collect voice_... / voicekey_... strings anywhere in a response."""
    found = []
    if isinstance(obj, dict):
        for v in obj.values():
            found += find_voice_ids(v)
    elif isinstance(obj, list):
        for v in obj:
            found += find_voice_ids(v)
    elif isinstance(obj, str) and obj.startswith(("voice_", "voicekey_")):
        found.append(obj)
    return found


def extract_audio(res):
    """Return the base64 audio data of the last audio item in the response."""
    audio = None
    for step in res.get("steps", []):
        if step.get("type") != "model_output":
            continue
        for item in step.get("content", []):
            if item.get("type") == "audio":
                audio = item.get("data")
    if audio is None:
        die("no audio in response:\n" + json.dumps(res, indent=2)[:2000])
    return audio


# --- Subcommands -------------------------------------------------------------

def cmd_register(args):
    body = {
        "store": not args.stateless,
        "voice": {
            "model": args.model,
            "type": "replicated",
            "display_name": args.name,
            "replicated": {
                "source_audio": {"mime_type": "audio/wav",
                                 "data": to_wav24k_b64(args.source)},
                "consent_audio": {"mime_type": "audio/wav",
                                  "data": to_wav24k_b64(args.consent)},
            },
        },
    }
    res = request("POST", "/voices", body)
    print(json.dumps(res, indent=2))
    ids = find_voice_ids(res)
    if not ids:
        die("could not find a voice ID in the response")
    VOICE_FILE.write_text(ids[0] + "\n")
    print(f"\nvoice: {ids[0]} (saved to {VOICE_FILE.name})")


def cmd_speak(args):
    voice = args.voice
    if not voice:
        if not VOICE_FILE.is_file():
            die("no voice given; use --voice or run 'register' first")
        voice = VOICE_FILE.read_text().strip()
    text = args.text if args.text is not None else Path(args.text_file).read_text()
    content = {"type": "text", "text": text}
    if args.style:
        content["annotations"] = [{"type": "speech_metadata",
                                   "style": args.style}]
    body = {
        "model": args.model,
        "input": [{"type": "user_input", "content": [content]}],
        "response_format": {"type": "audio"},
        "generation_config": {"speech_config": [{"voice": voice}]},
    }
    res = request("POST", "/interactions", body)
    Path(args.output).write_bytes(base64.b64decode(extract_audio(res)))
    print(f"wrote {args.output}")


def cmd_list(args):
    print(json.dumps(request("GET", "/voices?type=replicated"), indent=2))


def cmd_delete(args):
    request("DELETE", f"/voices/{args.voice}")
    print(f"deleted {args.voice}")
    if VOICE_FILE.is_file() and VOICE_FILE.read_text().strip() == args.voice:
        VOICE_FILE.unlink()


# --- Entry point -------------------------------------------------------------

def main():
    p = argparse.ArgumentParser(description=__doc__,
                                formatter_class=argparse.RawTextHelpFormatter)
    p.add_argument("--model", default=DEFAULT_MODEL,
                   help=f"TTS model (default: {DEFAULT_MODEL})")
    sub = p.add_subparsers(dest="command", required=True)

    r = sub.add_parser("register", help="create a replicated voice")
    r.add_argument("--source", required=True, help="reference speech (10-30 s)")
    r.add_argument("--consent", required=True, help="consent statement recording")
    r.add_argument("--name", default="my-voice", help="display name")
    r.add_argument("--stateless", action="store_true",
                   help="store=false: get a 7-day voicekey_ instead of voice_")
    r.set_defaults(func=cmd_register)

    s = sub.add_parser("speak", help="synthesize speech")
    g = s.add_mutually_exclusive_group(required=True)
    g.add_argument("--text", help="text to speak")
    g.add_argument("--text-file", help="read text from a file")
    s.add_argument("--voice", help="voice_... or voicekey_... (default: .voice_id)")
    s.add_argument("--style", help="delivery direction, e.g. '落ち着いたトーンで'")
    s.add_argument("-o", "--output", default="out.wav", help="output WAV path")
    s.set_defaults(func=cmd_speak)

    sub.add_parser("list", help="list replicated voices").set_defaults(func=cmd_list)

    d = sub.add_parser("delete", help="delete a stored voice")
    d.add_argument("voice", help="voice_... ID")
    d.set_defaults(func=cmd_delete)

    args = p.parse_args()
    args.func(args)


if __name__ == "__main__":
    main()
