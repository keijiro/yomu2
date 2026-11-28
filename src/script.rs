//! Parsing and serialization of the script file (script.md).
//!
//! Format:
//!
//! ```markdown
//! ---
//! voice: voice_xxxxxxxx
//! model: gemini-3.8-flash-tts
//! style: Calm tone, speak slowly
//! ---
//!
//! First paragraph. Uses the global style.
//!
//! <!-- style: Cheerful and energetic -->
//! Second paragraph. Overrides the style.
//! ```
//!
//! The front matter is kept verbatim; only the body is rewritten on save.

use serde::Deserialize;

pub const DEFAULT_MODEL: &str = "gemini-3.8-flash-tts";
pub const LITE_MODEL: &str = "gemini-3.8-flash-lite-tts";

/// Global options from the YAML front matter.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Header {
    pub voice: Option<String>,
    pub model: Option<String>,
    pub style: Option<String>,
}

/// A paragraph: the unit of synthesis.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Paragraph {
    /// Per-paragraph style override.
    pub style: Option<String>,
    pub text: String,
}

#[derive(Debug, Clone, Default)]
pub struct Script {
    /// Raw front matter including the `---` delimiters (empty if none).
    front_matter: String,
    pub header: Header,
    /// Set when the front matter could not be parsed.
    pub header_error: Option<String>,
    pub paragraphs: Vec<Paragraph>,
}

impl Script {
    pub fn parse(src: &str) -> Self {
        let src = src.trim_start_matches('\u{feff}').replace("\r\n", "\n");
        let mut script = Script::default();

        let body = match split_front_matter(&src) {
            Some((raw, yaml, body)) => {
                script.front_matter = raw.to_string();
                match parse_header(yaml) {
                    Ok(header) => script.header = header,
                    Err(e) => script.header_error = Some(e),
                }
                body
            }
            None => src.as_str(),
        };

        let mut block: Vec<&str> = Vec::new();
        for line in body.lines().chain(std::iter::once("")) {
            if line.trim().is_empty() {
                if !block.is_empty() {
                    script.paragraphs.push(parse_block(&block));
                    block.clear();
                }
            } else {
                block.push(line);
            }
        }
        script
    }

    /// A new, empty script with a front matter template.
    pub fn template(voice: Option<&str>) -> Self {
        let voice = voice.unwrap_or("");
        Self::parse(&format!(
            "---\nvoice: {voice}\nmodel: {DEFAULT_MODEL}\nstyle:\n---\n"
        ))
    }

    pub fn render(&self) -> String {
        let mut blocks: Vec<String> = Vec::new();
        if !self.front_matter.is_empty() {
            blocks.push(self.front_matter.clone());
        }
        for p in &self.paragraphs {
            let mut lines = Vec::new();
            if let Some(style) = &p.style {
                lines.push(format!("<!-- style: {style} -->"));
            }
            if !p.text.is_empty() {
                lines.push(p.text.clone());
            }
            if !lines.is_empty() {
                blocks.push(lines.join("\n"));
            }
        }
        let mut out = blocks.join("\n\n");
        out.push('\n');
        out
    }

    /// Sets (or removes, when `value` is `None`) a top-level front matter
    /// key, leaving the rest of the front matter untouched.
    pub fn set_header_value(&mut self, key: &str, value: Option<&str>) -> Result<(), String> {
        if self.header_error.is_some() {
            return Err("The front matter is invalid; fix it in the file first.".into());
        }
        let new_line = value.map(|v| format!("{key}: {}", yaml_scalar(v)));
        let mut lines: Vec<String> = if self.front_matter.is_empty() {
            vec!["---".into(), "---".into()]
        } else {
            self.front_matter.lines().map(str::to_string).collect()
        };
        let prefix = format!("{key}:");
        let at = lines
            .iter()
            .enumerate()
            .skip(1)
            .take(lines.len().saturating_sub(2))
            .find(|(_, l)| l.starts_with(&prefix))
            .map(|(i, _)| i);
        match (at, new_line) {
            (Some(i), new_line) => {
                // Drop continuation lines of a multi-line value.
                let mut end = i + 1;
                while end < lines.len() - 1
                    && lines[end].starts_with(|c: char| c == ' ' || c == '\t')
                {
                    end += 1;
                }
                lines.splice(i..end, new_line);
            }
            (None, Some(line)) => {
                let close = lines.len() - 1;
                lines.insert(close, line);
            }
            (None, None) => {}
        }
        self.front_matter = lines.join("\n");
        let yaml = lines[1..lines.len() - 1].join("\n");
        self.header = parse_header(&yaml)?;
        Ok(())
    }

    pub fn voice(&self) -> Option<&str> {
        self.header.voice.as_deref()
    }

    pub fn model(&self) -> &str {
        self.header.model.as_deref().unwrap_or(DEFAULT_MODEL)
    }

    pub fn global_style(&self) -> Option<&str> {
        self.header.style.as_deref()
    }

    /// The style actually used for a paragraph (override or global).
    pub fn effective_style<'a>(&'a self, p: &'a Paragraph) -> Option<&'a str> {
        p.style.as_deref().or(self.global_style())
    }
}

/// Normalizes paragraph text: no blank lines (they would split the
/// paragraph), no trailing whitespace.
pub fn clean_text(s: &str) -> String {
    s.lines()
        .map(str::trim_end)
        .filter(|l| !l.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string()
}

/// Normalizes a style string so it fits in a single-line HTML comment.
pub fn clean_style(s: &str) -> String {
    s.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .replace("-->", "->")
}

/// Formats a string as a single-line YAML scalar, quoting it if needed.
fn yaml_scalar(v: &str) -> String {
    let plain = !v.is_empty()
        && v == v.trim()
        && !v.starts_with(|c: char| "-?:,[]{}#&*!|>'\"%@`".contains(c))
        && !v.contains(": ")
        && !v.contains(" #")
        && !v.ends_with(':')
        && !v.contains(['\n', '\r']);
    if plain {
        v.to_string()
    } else {
        let escaped = v
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', "\\n")
            .replace('\r', "\\r");
        format!("\"{escaped}\"")
    }
}

/// Splits `src` into (raw front matter, YAML content, body).
fn split_front_matter(src: &str) -> Option<(&str, &str, &str)> {
    let first_end = src.find('\n')?;
    if src[..first_end].trim_end() != "---" {
        return None;
    }
    let yaml_start = first_end + 1;
    let mut pos = yaml_start;
    while pos < src.len() {
        let end = src[pos..].find('\n').map_or(src.len(), |i| pos + i);
        let line = src[pos..end].trim_end();
        if line == "---" || line == "..." {
            let body = src.get(end + 1..).unwrap_or("");
            return Some((&src[..end], &src[yaml_start..pos], body));
        }
        pos = end + 1;
    }
    None
}

fn parse_header(yaml: &str) -> Result<Header, String> {
    if yaml.trim().is_empty() {
        return Ok(Header::default());
    }
    let mut h: Header =
        serde_norway::from_str(yaml).map_err(|e| format!("invalid front matter: {e}"))?;
    let clean = |v: Option<String>| v.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    h.voice = clean(h.voice);
    h.model = clean(h.model);
    h.style = clean(h.style);
    Ok(h)
}

fn parse_style_directive(line: &str) -> Option<String> {
    let inner = line.trim().strip_prefix("<!--")?.strip_suffix("-->")?;
    let value = inner.trim().strip_prefix("style:")?;
    Some(value.trim().to_string())
}

fn parse_block(lines: &[&str]) -> Paragraph {
    let (style, rest) = match parse_style_directive(lines[0]) {
        Some(style) => (Some(style).filter(|s| !s.is_empty()), &lines[1..]),
        None => (None, lines),
    };
    let text = rest
        .iter()
        .map(|l| l.trim_end())
        .collect::<Vec<_>>()
        .join("\n");
    Paragraph { style, text }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "---\n# comment\nvoice: voice_abc\nstyle: 落ち着いたトーンで\n---\n\n\
        最初の段落です。\n\n\
        <!-- style: 明るく元気に -->\n二番目の段落。\n二行目。\n\n\n\
        三番目。\n";

    #[test]
    fn parses_front_matter_and_paragraphs() {
        let s = Script::parse(SAMPLE);
        assert_eq!(s.voice(), Some("voice_abc"));
        assert_eq!(s.model(), DEFAULT_MODEL);
        assert_eq!(s.global_style(), Some("落ち着いたトーンで"));
        assert_eq!(s.paragraphs.len(), 3);
        assert_eq!(s.paragraphs[0].style, None);
        assert_eq!(s.paragraphs[1].style.as_deref(), Some("明るく元気に"));
        assert_eq!(s.paragraphs[1].text, "二番目の段落。\n二行目。");
        assert_eq!(
            s.effective_style(&s.paragraphs[0]),
            Some("落ち着いたトーンで")
        );
        assert_eq!(s.effective_style(&s.paragraphs[1]), Some("明るく元気に"));
    }

    #[test]
    fn render_preserves_front_matter_and_round_trips() {
        let s = Script::parse(SAMPLE);
        let out = s.render();
        assert!(out.starts_with("---\n# comment\nvoice: voice_abc\n"));
        let again = Script::parse(&out);
        assert_eq!(again.paragraphs, s.paragraphs);
        assert_eq!(again.render(), out);
    }

    #[test]
    fn no_front_matter() {
        let s = Script::parse("Hello.\n\nWorld.");
        assert_eq!(s.voice(), None);
        assert_eq!(s.paragraphs.len(), 2);
        assert_eq!(s.render(), "Hello.\n\nWorld.\n");
    }

    #[test]
    fn invalid_front_matter_is_reported_but_kept() {
        let s = Script::parse("---\nvoice: [unclosed\n---\nText.\n");
        assert!(s.header_error.is_some());
        assert_eq!(s.paragraphs.len(), 1);
        assert!(s.render().starts_with("---\nvoice: [unclosed\n---\n"));
    }

    #[test]
    fn crlf_and_template() {
        let s = Script::parse("---\r\nvoice: v\r\n---\r\nA\r\n\r\nB\r\n");
        assert_eq!(s.voice(), Some("v"));
        assert_eq!(s.paragraphs.len(), 2);
        let t = Script::template(Some("voice_x"));
        assert_eq!(t.voice(), Some("voice_x"));
        assert_eq!(t.global_style(), None);
        assert!(t.paragraphs.is_empty());
    }

    #[test]
    fn set_header_value_edits_front_matter() {
        let mut s = Script::parse("---\n# c\nvoice: v\nstyle: old\n  more\n---\nText.\n");
        s.set_header_value("style", Some("new: \"x\"")).unwrap();
        assert_eq!(s.global_style(), Some("new: \"x\""));
        s.set_header_value("model", Some(LITE_MODEL)).unwrap();
        assert_eq!(s.model(), LITE_MODEL);
        s.set_header_value("style", None).unwrap();
        assert_eq!(s.global_style(), None);
        assert_eq!(
            s.render(),
            format!("---\n# c\nvoice: v\nmodel: {LITE_MODEL}\n---\n\nText.\n")
        );
        let mut n = Script::parse("Hi.\n");
        n.set_header_value("style", Some("calm")).unwrap();
        assert_eq!(n.render(), "---\nstyle: calm\n---\n\nHi.\n");
    }

    #[test]
    fn cleaning() {
        assert_eq!(clean_text("  a  \n\n\n b \n"), "a\n b");
        assert_eq!(clean_style(" fast\n and -->loud "), "fast and ->loud");
    }
}
