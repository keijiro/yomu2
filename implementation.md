# clone_tts.py

A one-shot script that uses **Voice replication (voice cloning)** in Gemini 3.8 Flash TTS
to generate speech in the voice of any given speaker.

- Uses only the Python standard library (no extra installs; works with the Python 3.9 bundled with macOS)
- Retrieves the API key from the macOS Keychain
- Input audio is automatically converted to 24kHz / mono / 16-bit WAV with `afconvert` (m4a, mp3, aiff, etc. are also accepted)

> ⚠️ This script was written from the official documentation (as of 2026-11) before an API key was available.
> Because the documentation doesn't specify the response format of the voice registration API,
> `register` prints the entire returned JSON, then searches it for a `voice_...` / `voicekey_...` value and saves it.

## Prerequisites

- macOS
- A Gemini API project on **Paid Tier 1 or higher** (Voice replication is not available on the free tier)
- Two recordings of the speaker whose voice you want to clone (see below)

## 1. Store the API key in the Keychain

Issue an API key in [Google AI Studio](https://aistudio.google.com/) and register it with the following command
(passing `-w` without a value prompts for it interactively, so the key doesn't end up in your shell history).

```sh
security add-generic-password -a "$USER" -s gemini-api-key -w
```

To verify:

```sh
security find-generic-password -s gemini-api-key -w
```

To use a different service name, set it with the `GEMINI_KEYCHAIN_SERVICE` environment variable.

## 2. Prepare the recordings

| File | Contents |
|---|---|
| Source audio (e.g. `source.m4a`) | **10–30 seconds** of natural speech by the speaker. Quiet environment, single speaker only |
| Consent audio (e.g. `consent.m4a`) | The same speaker reading the consent statement below |

Consent statement in Japanese:

> 私はこの音声の所有者であり、Googleがこの音声を使用して音声合成モデルを作成することを承認します。

(Meaning: "I am the owner of this voice, and I authorize Google to use this voice to create a speech synthesis model.")

Notes:

- Record both clips **in the same room with the same microphone** (this makes speaker verification less likely to fail)
- Synthesized audio and audio containing SynthID / C2PA watermarks will be rejected
- Do not register another person's voice without their consent

## 3. Register the voice

```sh
python3 clone_tts.py register --source source.m4a --consent consent.m4a --name "my-voice"
```

On success, a `voice_...` ID is displayed and saved to `.voice_id` in the same folder as the script.

- Registered voices are stored in the project and remain valid for **1 year from last use** (up to 200 per project)
- With `--stateless`, a `voicekey_...` that isn't stored on the server is issued instead (**valid for 7 days**).
  When it expires, run `register` again with the same recordings to get a new key

## 4. Generate speech

```sh
python3 clone_tts.py speak --text "こんにちは、今日もよろしくお願いします。" -o hello.wav
```

Options:

| Option | Description |
|---|---|
| `--text TEXT` / `--text-file PATH` | Text to speak (one of them is required) |
| `--style TEXT` | Speaking style instructions (e.g. `--style "calm tone, speak slowly"`) |
| `--voice ID` | Voice to use. Defaults to the value in `.voice_id` |
| `-o PATH` | Output file (default: `out.wav`; 24kHz / mono / 16-bit WAV) |
| `--model NAME` | Specify **before** the subcommand. Use `gemini-3.8-flash-lite-tts` for the cheaper model |

You can insert tags such as `<laugh>`, `<sigh>`, and `<short pause>` into the text to add laughter, sighs, and pauses.

```sh
python3 clone_tts.py --model gemini-3.8-flash-lite-tts speak --text-file script.txt --style "cheerful" -o out.wav
afplay out.wav
```

## 5. Manage voices

```sh
python3 clone_tts.py list                 # List registered voices
python3 clone_tts.py delete voice_xxxxx   # Delete (cannot be undone)
```

## Pricing guide (through 2026-12-31, Standard)

| Model | Text input | Audio output |
|---|---|---|
| gemini-3.8-flash-tts | $0.50 / 1M tokens | $9.00 / 1M tokens (approx. $0.00225 / 10 sec) |
| gemini-3.8-flash-lite-tts | $0.50 / 1M tokens | $6.00 / 1M tokens (approx. $0.0015 / 10 sec) |

Prices are scheduled to double from 2027-01-01. Check the [official page](https://aistudio.google.com/docs/pricing) for the latest pricing.

## References

- [Voice replication](https://aistudio.google.com/docs/voice-replication)
- [Speech generation](https://aistudio.google.com/docs/speech-generation)
