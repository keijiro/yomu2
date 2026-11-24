//! Audio cache and playback.
//!
//! Synthesized audio is stored in `<script stem>.audio/<key>.wav`, where the
//! key is a hash of everything that affects the result (model, voice, style
//! and text). Editing a paragraph changes its key, so stale audio is never
//! played for edited text.

use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

pub struct Cache {
    dir: PathBuf,
    present: HashSet<String>,
}

impl Cache {
    pub fn for_script(script: &Path) -> Self {
        let mut cache = Self {
            dir: script.with_extension("audio"),
            present: HashSet::new(),
        };
        cache.rescan();
        cache
    }

    pub fn rescan(&mut self) {
        self.present = fs::read_dir(&self.dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| {
                e.file_name()
                    .to_str()?
                    .strip_suffix(".wav")
                    .map(str::to_owned)
            })
            .collect();
    }

    pub fn path(&self, key: &str) -> PathBuf {
        self.dir.join(format!("{key}.wav"))
    }

    pub fn contains(&self, key: &str) -> bool {
        self.present.contains(key)
    }

    pub fn insert(&mut self, key: String) {
        self.present.insert(key);
    }

    pub fn remove(&mut self, key: &str) {
        self.present.remove(key);
    }
}

/// Cache key: 64-bit FNV-1a over the synthesis parameters.
pub fn cache_key(model: &str, voice: &str, style: Option<&str>, text: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for field in ["yomu1", model, voice, style.unwrap_or(""), text] {
        for b in field.bytes().chain(std::iter::once(0)) {
            hash ^= u64::from(b);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    format!("{hash:016x}")
}

/// Writes via a temporary file + rename so readers never see partial data.
pub fn write_atomic(path: &Path, data: &[u8]) -> io::Result<()> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent)?;
    }
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let tmp = path.with_file_name(format!(".{name}.tmp"));
    fs::write(&tmp, data)?;
    fs::rename(&tmp, path).inspect_err(|_| {
        let _ = fs::remove_file(&tmp);
    })
}

/// Copies audio files into `dir` as 01.wav, 02.wav, ... in paragraph order.
/// `sources[i]` is the audio of paragraph i (None: not synthesized, skipped).
/// Numbered files left over from a previous export are removed first.
pub fn export(sources: &[Option<PathBuf>], dir: &Path) -> io::Result<()> {
    let width = sources.len().to_string().len().max(2);
    fs::create_dir_all(dir)?;
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name();
        let numbered = name
            .to_str()
            .and_then(|n| n.strip_suffix(".wav"))
            .is_some_and(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()));
        if numbered {
            fs::remove_file(entry.path())?;
        }
    }
    for (i, src) in sources.iter().enumerate() {
        if let Some(src) = src {
            fs::copy(src, dir.join(format!("{:0width$}.wav", i + 1)))?;
        }
    }
    Ok(())
}

/// Plays audio files with `afplay`, one at a time.
#[derive(Default)]
pub struct Player {
    current: Option<(Child, String)>,
}

impl Player {
    pub fn play(&mut self, path: &Path, key: &str) -> io::Result<()> {
        self.stop();
        let child = Command::new("afplay")
            .arg(path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
        self.current = Some((child, key.to_string()));
        Ok(())
    }

    pub fn stop(&mut self) {
        if let Some((mut child, _)) = self.current.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    /// Key of the audio being played, if any.
    pub fn playing(&self) -> Option<&str> {
        self.current.as_ref().map(|(_, key)| key.as_str())
    }

    /// Clears the state when playback has finished.
    pub fn poll(&mut self) {
        if let Some((child, _)) = &mut self.current
            && !matches!(child.try_wait(), Ok(None))
        {
            self.current = None;
        }
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_key_is_stable_and_sensitive() {
        let k = cache_key("m", "v", Some("s"), "t");
        assert_eq!(k.len(), 16);
        assert_eq!(k, cache_key("m", "v", Some("s"), "t"));
        assert_ne!(k, cache_key("m", "v", None, "t"));
        assert_ne!(k, cache_key("m", "v", Some("s"), "t2"));
        assert_ne!(
            cache_key("m", "v", Some("ab"), "c"),
            cache_key("m", "v", Some("a"), "bc")
        );
    }

    #[test]
    fn export_numbers_files_and_removes_stale_ones() {
        let root = std::env::temp_dir().join(format!("yomu-export-test-{}", std::process::id()));
        let (src, out) = (root.join("src"), root.join("out"));
        fs::create_dir_all(&src).unwrap();
        fs::create_dir_all(&out).unwrap();
        fs::write(src.join("a.wav"), "A").unwrap();
        fs::write(src.join("c.wav"), "C").unwrap();
        fs::write(out.join("05.wav"), "stale").unwrap();
        fs::write(out.join("keep.wav"), "mine").unwrap();

        export(
            &[Some(src.join("a.wav")), None, Some(src.join("c.wav"))],
            &out,
        )
        .unwrap();

        let mut names: Vec<String> = fs::read_dir(&out)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        assert_eq!(names, ["01.wav", "03.wav", "keep.wav"]);
        assert_eq!(fs::read_to_string(out.join("03.wav")).unwrap(), "C");
        fs::remove_dir_all(&root).unwrap();
    }
}
