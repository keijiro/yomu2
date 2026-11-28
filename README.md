# yomu

A terminal app for editing a narration script (`script.md`) and turning it
into speech, paragraph by paragraph, using Gemini TTS voice replication.

## Requirements

- macOS (uses the keychain via `security` and plays audio with `afplay`)
- A Gemini API key in the keychain (service `gemini-api-key`, or set
  `GEMINI_KEYCHAIN_SERVICE`). See [implementation.md](implementation.md).
- A replicated voice ID (`voice_...`). Register one with `yomu voice register`
  (see below).

## Build and run

```sh
cargo install --path .     # or: cargo run --release -- script.md
yomu                       # opens ./script.md
yomu path/to/other.md
```

If the file doesn't exist, yomu starts from a template. The file is created
on the first edit.

## Voice management

```sh
yomu voice register --source source.wav --consent consent.wav [--name NAME] [--stateless]
yomu voice list
yomu voice delete voice_xxxxxxxxxxxx
```

`register` prints the new voice ID; copy it into the `voice:` field of the
script. Use `list` to look it up again later. Input audio is converted with
`afconvert`, so any format it can read works.

## Script format

```markdown
---
voice: voice_xxxxxxxxxxxx
model: gemini-3.8-flash-tts
style: 落ち着いたトーンで、ゆっくりと
---

こんにちは。今日は新しいツールについて紹介します。

<!-- style: 明るく元気に -->
まずは基本的な使い方から見ていきましょう。

Paragraphs are separated by blank lines. Lines inside a paragraph
are kept as they are.
```

Front matter keys:

| Key     | Description                                                    |
|---------|----------------------------------------------------------------|
| `voice` | Replicated voice ID (`voice_...` or `voicekey_...`). Required. |
| `model` | TTS model. Defaults to `gemini-3.8-flash-tts`.                 |
| `style` | Default delivery style for every paragraph.                    |

A paragraph can override the style with a `<!-- style: ... -->` line right
above its text. Gemini speech tags such as `<laugh>` or `<short pause>` can be
used in the text.

yomu keeps the front matter exactly as written. It rewrites the body on save
in a normalized form: one blank line between paragraphs.

## Keys

| Key                     | Action                                          |
|-------------------------|-------------------------------------------------|
| `↑` `↓` / `k` `j`       | Select paragraph (`g` / `G`: first / last)      |
| `←` `→` / `h` `l`       | Select the Style or Text column                 |
| `Enter`                 | Edit the selected cell                          |
| `r`                     | Synthesize the selected paragraph               |
| `Space`                 | Play / stop the paragraph (when synthesized)    |
| `e`                     | Export the audio as numbered files              |
| `m`                     | Toggle the model: standard / lite               |
| `s`                     | Edit the global style                           |
| `o` / `O`               | Add a paragraph below / above                   |
| `dd`                    | Delete the paragraph                            |
| `u`                     | Undo                                            |
| `q`                     | Quit                                            |

While editing: `Enter` saves, `Esc` cancels, `Alt+Enter` inserts a line break,
and `Ctrl-A` / `Ctrl-E` / `Ctrl-U` / `Ctrl-K` / `Ctrl-W` work as in a shell.
Changes are written to the file as soon as you press `Enter`. Clearing a
paragraph's text deletes it. Clearing a style (or making it equal to the
default) removes the override.

If another program changes the script, yomu reloads it automatically.

Status column: `·` not synthesized, `⠋` synthesizing, `●` ready, `▶` playing,
`✗` failed (move onto the row to see the error).

## Audio files

Audio is stored in `<script name>.audio/` next to the script (for example
`script.audio/`). Each file is named after a hash of the model, voice,
effective style, and text, so editing a paragraph marks it as not synthesized,
and undoing the edit brings the earlier audio back. Pressing `r` on a
synthesized paragraph makes a new take and replaces the old one.

## Export

Press `e` to copy the audio into `<script name>.export/` (for example
`script.export/`) as `01.wav`, `02.wav`, and so on. The numbers match the `#`
column. Paragraphs that aren't synthesized are skipped, and the status line
lists their numbers.

Each export replaces the numbered `.wav` files from the previous one, so a
deleted paragraph doesn't leave a stale file behind. Other files in the folder
are left alone.
