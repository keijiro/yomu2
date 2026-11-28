//! Application state and input handling.

use crate::audio::{self, Cache, Player};
use crate::editor::Editor;
use crate::script::{self, Paragraph, Script};
use crate::tts;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::widgets::TableState;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

const UNDO_LIMIT: usize = 200;
const DISK_CHECK_INTERVAL: Duration = Duration::from_millis(500);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Style,
    Text,
    /// The global style in the front matter (edited via a popup only).
    GlobalStyle,
}

impl Field {
    /// Column index in the table (#, status, style, text).
    pub fn column(self) -> usize {
        match self {
            Field::Style | Field::GlobalStyle => 2,
            Field::Text => 3,
        }
    }
}

pub struct Edit {
    pub editor: Editor,
    pub field: Field,
    pub index: usize,
    /// The paragraph was just inserted; cancel removes it.
    pub is_new: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageKind {
    Info,
    Success,
    Error,
}

pub struct Message {
    pub text: String,
    pub kind: MessageKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowStatus {
    None,
    Busy,
    Ready,
    Playing,
    Failed,
}

struct JobResult {
    key: String,
    result: Result<(), String>,
}

/// What we last wrote to / read from disk, to detect external changes.
struct DiskState {
    modified: Option<SystemTime>,
    content: String,
}

pub struct App {
    path: PathBuf,
    pub display_name: String,
    pub script: Script,
    disk: Option<DiskState>,
    pub table: TableState,
    pub field: Field,
    pub edit: Option<Edit>,
    /// Wrap width of the edit box, updated on each draw.
    pub edit_width: usize,
    cache: Cache,
    busy: HashSet<String>,
    errors: HashMap<String, String>,
    player: Player,
    tx: Sender<JobResult>,
    rx: Receiver<JobResult>,
    pub message: Option<Message>,
    undo: Vec<Vec<Paragraph>>,
    pending_d: bool,
    quit_armed: bool,
    pub tick: usize,
    last_disk_check: Instant,
    pub should_quit: bool,
}

impl App {
    pub fn open(arg: &Path) -> Result<Self, String> {
        let path = fs::canonicalize(arg).unwrap_or_else(|_| arg.to_path_buf());
        let (script, disk) = if path.exists() {
            let content = fs::read_to_string(&path)
                .map_err(|e| format!("cannot read {}: {e}", arg.display()))?;
            let modified = fs::metadata(&path).and_then(|m| m.modified()).ok();
            (
                Script::parse(&content),
                Some(DiskState { modified, content }),
            )
        } else {
            let voice = fs::read_to_string(path.with_file_name(".voice_id")).ok();
            (Script::template(voice.as_deref().map(str::trim)), None)
        };
        let (tx, rx) = mpsc::channel();
        let mut app = Self {
            cache: Cache::for_script(&path),
            path,
            display_name: arg.display().to_string(),
            script,
            disk,
            table: TableState::default(),
            field: Field::Text,
            edit: None,
            edit_width: 40,
            busy: HashSet::new(),
            errors: HashMap::new(),
            player: Player::default(),
            tx,
            rx,
            message: None,
            undo: Vec::new(),
            pending_d: false,
            quit_armed: false,
            tick: 0,
            last_disk_check: Instant::now(),
            should_quit: false,
        };
        app.clamp_selection();
        if app.disk.is_none() {
            app.info("New file. Press o to add a paragraph.");
        }
        app.report_header_error();
        Ok(app)
    }

    pub fn selected(&self) -> Option<usize> {
        self.table
            .selected()
            .filter(|&i| i < self.script.paragraphs.len())
    }

    // --- Periodic update -----------------------------------------------------

    pub fn on_tick(&mut self) {
        self.tick = self.tick.wrapping_add(1);
        self.player.poll();
        while let Ok(job) = self.rx.try_recv() {
            self.on_job_done(job);
        }
        if self.edit.is_none() && self.last_disk_check.elapsed() >= DISK_CHECK_INTERVAL {
            self.last_disk_check = Instant::now();
            self.check_disk();
        }
    }

    fn on_job_done(&mut self, job: JobResult) {
        self.busy.remove(&job.key);
        let label = match self.row_of(&job.key) {
            Some(i) => format!("Paragraph {}", i + 1),
            None => "A paragraph".to_string(),
        };
        match job.result {
            Ok(()) => {
                self.cache.insert(job.key);
                self.success(format!("{label} synthesized."));
            }
            Err(e) => {
                self.error(format!("{label}: {e}"));
                self.errors.insert(job.key, e);
            }
        }
    }

    /// Reloads the script if it was changed by another program.
    fn check_disk(&mut self) {
        let Ok(meta) = fs::metadata(&self.path) else {
            return;
        };
        let modified = meta.modified().ok();
        if self.disk.as_ref().is_some_and(|d| d.modified == modified) {
            return;
        }
        let Ok(content) = fs::read_to_string(&self.path) else {
            return;
        };
        if let Some(disk) = &mut self.disk
            && disk.content == content
        {
            disk.modified = modified;
            return;
        }
        self.script = Script::parse(&content);
        self.disk = Some(DiskState { modified, content });
        self.undo.clear();
        self.clamp_selection();
        self.info("Reloaded: the file was changed on disk.");
        self.report_header_error();
    }

    // --- Key handling --------------------------------------------------------

    pub fn on_key(&mut self, key: KeyEvent) {
        if self.edit.is_some() {
            self.on_edit_key(key);
            return;
        }
        let pending_d = std::mem::take(&mut self.pending_d);
        let quit_armed = std::mem::take(&mut self.quit_armed);
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            if key.code == KeyCode::Char('c') {
                self.request_quit(quit_armed);
            }
            return;
        }
        match key.code {
            KeyCode::Char('q') => self.request_quit(quit_armed),
            KeyCode::Up | KeyCode::Char('k') => self.move_by(-1),
            KeyCode::Down | KeyCode::Char('j') => self.move_by(1),
            KeyCode::PageUp => self.move_by(-10),
            KeyCode::PageDown => self.move_by(10),
            KeyCode::Home | KeyCode::Char('g') => self.move_to(0),
            KeyCode::End | KeyCode::Char('G') => self.move_to(usize::MAX),
            KeyCode::Left | KeyCode::Char('h') => self.field = Field::Style,
            KeyCode::Right | KeyCode::Char('l') => self.field = Field::Text,
            KeyCode::Enter => match self.selected() {
                Some(i) => self.begin_edit(i, self.field, false),
                None => self.insert_paragraph(0),
            },
            KeyCode::Char('r') => {
                if let Some(i) = self.selected() {
                    self.synthesize(i);
                }
            }
            KeyCode::Char(' ') => {
                if let Some(i) = self.selected() {
                    self.toggle_play(i);
                }
            }
            KeyCode::Char('o') => self.insert_paragraph(self.selected().map_or(0, |i| i + 1)),
            KeyCode::Char('O') => self.insert_paragraph(self.selected().unwrap_or(0)),
            KeyCode::Char('d') if pending_d => {
                if let Some(i) = self.selected() {
                    self.delete_paragraph(i);
                }
            }
            KeyCode::Char('d') => self.pending_d = true,
            KeyCode::Char('u') => self.undo(),
            KeyCode::Char('e') => self.export(),
            KeyCode::Char('m') => self.toggle_model(),
            KeyCode::Char('s') => self.begin_global_style_edit(),
            KeyCode::Esc => self.message = None,
            _ => {}
        }
    }

    pub fn on_paste(&mut self, text: &str) {
        if let Some(edit) = &mut self.edit {
            let text = match edit.field {
                Field::Text => text.to_string(),
                Field::Style | Field::GlobalStyle => text.replace(['\r', '\n'], " "),
            };
            edit.editor.insert_str(&text);
        }
    }

    fn on_edit_key(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let newline = key
            .modifiers
            .intersects(KeyModifiers::ALT | KeyModifiers::SHIFT);
        match key.code {
            KeyCode::Esc => return self.cancel_edit(),
            KeyCode::Char('c') if ctrl => return self.cancel_edit(),
            KeyCode::Enter if !newline => return self.commit_edit(),
            _ => {}
        }
        let width = self.edit_width;
        let Some(edit) = &mut self.edit else { return };
        let ed = &mut edit.editor;
        match key.code {
            KeyCode::Enter if edit.field == Field::Text => ed.insert_char('\n'),
            KeyCode::Char('a') if ctrl => ed.home(),
            KeyCode::Char('e') if ctrl => ed.end(),
            KeyCode::Char('b') if ctrl => ed.left(),
            KeyCode::Char('f') if ctrl => ed.right(),
            KeyCode::Char('h') if ctrl => ed.backspace(),
            KeyCode::Char('d') if ctrl => ed.delete(),
            KeyCode::Char('u') if ctrl => ed.kill_to_start(),
            KeyCode::Char('k') if ctrl => ed.kill_to_end(),
            KeyCode::Char('w') if ctrl => ed.delete_word_back(),
            KeyCode::Char(c) if !ctrl => ed.insert_char(c),
            KeyCode::Backspace => ed.backspace(),
            KeyCode::Delete => ed.delete(),
            KeyCode::Left => ed.left(),
            KeyCode::Right => ed.right(),
            KeyCode::Home => ed.home(),
            KeyCode::End => ed.end(),
            KeyCode::Up => ed.up(width),
            KeyCode::Down => ed.down(width),
            _ => {}
        }
    }

    fn request_quit(&mut self, armed: bool) {
        if self.busy.is_empty() || armed {
            self.should_quit = true;
        } else {
            self.info(format!(
                "{} synthesis job(s) still running. Press q again to quit anyway.",
                self.busy.len()
            ));
            self.quit_armed = true;
        }
    }

    // --- Navigation ----------------------------------------------------------

    fn move_by(&mut self, delta: isize) {
        let current = self.selected().unwrap_or(0) as isize;
        self.move_to(current.saturating_add(delta).max(0) as usize);
    }

    fn move_to(&mut self, index: usize) {
        self.table.select(Some(index));
        self.clamp_selection();
        // Show the error of a failed row when landing on it.
        let key = self.selected().and_then(|i| self.key_for(i));
        if let Some(e) = key.and_then(|k| self.errors.get(&k)) {
            let msg = format!("Last synthesis failed: {e}");
            self.error(msg);
        }
    }

    fn clamp_selection(&mut self) {
        let len = self.script.paragraphs.len();
        let selected = match (len, self.table.selected()) {
            (0, _) => None,
            (_, None) => Some(0),
            (_, Some(i)) => Some(i.min(len - 1)),
        };
        self.table.select(selected);
        self.table.select_column(Some(self.field.column()));
    }

    // --- Editing -------------------------------------------------------------

    fn begin_edit(&mut self, index: usize, field: Field, is_new: bool) {
        let p = &self.script.paragraphs[index];
        let initial = match field {
            Field::Text => p.text.clone(),
            Field::Style | Field::GlobalStyle => {
                self.script.effective_style(p).unwrap_or("").to_string()
            }
        };
        self.field = field;
        self.edit = Some(Edit {
            editor: Editor::new(&initial),
            field,
            index,
            is_new,
        });
    }

    fn begin_global_style_edit(&mut self) {
        let initial = self.script.global_style().unwrap_or("");
        self.edit = Some(Edit {
            editor: Editor::new(initial),
            field: Field::GlobalStyle,
            index: 0,
            is_new: false,
        });
    }

    fn toggle_model(&mut self) {
        let next = if self.script.model() == script::LITE_MODEL {
            script::DEFAULT_MODEL
        } else {
            script::LITE_MODEL
        };
        match self.script.set_header_value("model", Some(next)) {
            Ok(()) => {
                self.save();
                let label = if next == script::LITE_MODEL {
                    "lite"
                } else {
                    "standard"
                };
                self.info(format!("Model: {next} ({label})"));
            }
            Err(e) => self.error(e),
        }
    }

    fn insert_paragraph(&mut self, index: usize) {
        let index = index.min(self.script.paragraphs.len());
        self.push_undo();
        self.script.paragraphs.insert(index, Paragraph::default());
        self.table.select(Some(index));
        self.begin_edit(index, Field::Text, true);
    }

    /// Removes a just-inserted paragraph that was never saved.
    fn discard_new(&mut self, index: usize) {
        self.script.paragraphs.remove(index);
        self.undo.pop();
        self.clamp_selection();
    }

    fn cancel_edit(&mut self) {
        if let Some(edit) = self.edit.take()
            && edit.is_new
        {
            self.discard_new(edit.index);
        }
    }

    fn commit_edit(&mut self) {
        let Some(edit) = self.edit.take() else { return };
        let i = edit.index;
        let value = edit.editor.text();
        match edit.field {
            Field::Text => {
                let text = script::clean_text(&value);
                if text.is_empty() {
                    if edit.is_new {
                        self.discard_new(i);
                    } else {
                        self.delete_paragraph(i);
                    }
                } else if edit.is_new {
                    // The undo snapshot was taken on insertion.
                    self.script.paragraphs[i].text = text;
                    self.save();
                } else if text != self.script.paragraphs[i].text {
                    self.push_undo();
                    self.script.paragraphs[i].text = text;
                    self.save();
                }
            }
            Field::GlobalStyle => {
                let style = script::clean_style(&value);
                let style = Some(style.as_str()).filter(|s| !s.is_empty());
                if style != self.script.global_style() {
                    match self.script.set_header_value("style", style) {
                        Ok(()) => self.save(),
                        Err(e) => self.error(e),
                    }
                }
            }
            Field::Style => {
                let style = script::clean_style(&value);
                // Empty or same as the global style: no override.
                let style = Some(style)
                    .filter(|s| !s.is_empty() && Some(s.as_str()) != self.script.global_style());
                if style != self.script.paragraphs[i].style {
                    self.push_undo();
                    self.script.paragraphs[i].style = style;
                    self.save();
                }
            }
        }
    }

    fn delete_paragraph(&mut self, index: usize) {
        self.push_undo();
        self.script.paragraphs.remove(index);
        self.clamp_selection();
        self.save();
        self.info(format!("Deleted paragraph {}. Press u to undo.", index + 1));
    }

    fn push_undo(&mut self) {
        self.undo.push(self.script.paragraphs.clone());
        if self.undo.len() > UNDO_LIMIT {
            self.undo.remove(0);
        }
    }

    fn undo(&mut self) {
        match self.undo.pop() {
            Some(paragraphs) => {
                self.script.paragraphs = paragraphs;
                self.clamp_selection();
                self.save();
                self.info("Undone.");
            }
            None => self.info("Nothing to undo."),
        }
    }

    fn save(&mut self) {
        let content = self.script.render();
        match audio::write_atomic(&self.path, content.as_bytes()) {
            Ok(()) => {
                let modified = fs::metadata(&self.path).and_then(|m| m.modified()).ok();
                self.disk = Some(DiskState { modified, content });
            }
            Err(e) => self.error(format!("Failed to save {}: {e}", self.display_name)),
        }
    }

    // --- Synthesis and playback ----------------------------------------------

    fn request_for(&self, index: usize) -> Option<tts::Request> {
        let p = &self.script.paragraphs[index];
        Some(tts::Request {
            model: self.script.model().to_string(),
            voice: self.script.voice()?.to_string(),
            style: self.script.effective_style(p).map(str::to_owned),
            text: p.text.clone(),
        })
    }

    fn key_for(&self, index: usize) -> Option<String> {
        let r = self.request_for(index)?;
        Some(audio::cache_key(
            &r.model,
            &r.voice,
            r.style.as_deref(),
            &r.text,
        ))
    }

    fn row_of(&self, key: &str) -> Option<usize> {
        (0..self.script.paragraphs.len()).find(|&i| self.key_for(i).as_deref() == Some(key))
    }

    pub fn row_status(&self, index: usize) -> RowStatus {
        let Some(key) = self.key_for(index) else {
            return RowStatus::None;
        };
        if self.busy.contains(&key) {
            RowStatus::Busy
        } else if self.player.playing() == Some(key.as_str()) {
            RowStatus::Playing
        } else if self.cache.contains(&key) {
            RowStatus::Ready
        } else if self.errors.contains_key(&key) {
            RowStatus::Failed
        } else {
            RowStatus::None
        }
    }

    fn synthesize(&mut self, index: usize) {
        let Some(req) = self.request_for(index) else {
            return self.error("No voice ID. Set `voice:` in the front matter.");
        };
        if req.text.trim().is_empty() {
            return self.error("The paragraph is empty.");
        }
        let key = audio::cache_key(&req.model, &req.voice, req.style.as_deref(), &req.text);
        if self.busy.contains(&key) {
            return self.info(format!(
                "Paragraph {} is already being synthesized.",
                index + 1
            ));
        }
        self.busy.insert(key.clone());
        self.errors.remove(&key);
        let path = self.cache.path(&key);
        let tx = self.tx.clone();
        thread::spawn(move || {
            let result = tts::synthesize(&req).and_then(|wav| {
                audio::write_atomic(&path, &wav)
                    .map_err(|e| format!("cannot write {}: {e}", path.display()))
            });
            let _ = tx.send(JobResult { key, result });
        });
        self.info(format!("Synthesizing paragraph {}…", index + 1));
    }

    fn toggle_play(&mut self, index: usize) {
        let Some(key) = self.key_for(index) else {
            return self.info("Not synthesized yet. Press r to synthesize.");
        };
        if self.player.playing() == Some(key.as_str()) {
            self.player.stop();
            return self.info("Stopped.");
        }
        let path = self.cache.path(&key);
        if !self.cache.contains(&key) || !path.is_file() {
            self.cache.remove(&key);
            return self.info("Not synthesized yet. Press r to synthesize.");
        }
        match self.player.play(&path, &key) {
            Ok(()) => self.info(format!("Playing paragraph {}.", index + 1)),
            Err(e) => self.error(format!("Cannot start afplay: {e}")),
        }
    }

    /// Exports synthesized audio as numbered files in `<script>.export/`.
    fn export(&mut self) {
        let sources: Vec<Option<PathBuf>> = (0..self.script.paragraphs.len())
            .map(|i| {
                let key = self.key_for(i).filter(|k| self.cache.contains(k))?;
                Some(self.cache.path(&key)).filter(|p| p.is_file())
            })
            .collect();
        let missing: Vec<usize> = (0..sources.len())
            .filter(|&i| sources[i].is_none())
            .map(|i| i + 1)
            .collect();
        if missing.len() == sources.len() {
            return self.error("Nothing to export. Synthesize paragraphs with r first.");
        }
        let dir = self.path.with_extension("export");
        let shown = Path::new(&self.display_name).with_extension("export");
        let (total, done) = (sources.len(), sources.len() - missing.len());
        match audio::export(&sources, &dir) {
            Ok(()) if missing.is_empty() => self.success(format!(
                "Exported {total} paragraphs to {}/",
                shown.display()
            )),
            Ok(()) => {
                let mut list: Vec<String> =
                    missing.iter().take(10).map(|n| n.to_string()).collect();
                if missing.len() > 10 {
                    list.push("…".to_string());
                }
                self.info(format!(
                    "Exported {done} of {total} paragraphs to {}/ (not synthesized: {})",
                    shown.display(),
                    list.join(", ")
                ))
            }
            Err(e) => self.error(format!("Export failed: {e}")),
        }
    }

    // --- Messages ------------------------------------------------------------

    fn set_message(&mut self, kind: MessageKind, text: impl Into<String>) {
        self.message = Some(Message {
            text: text.into(),
            kind,
        });
    }

    fn info(&mut self, text: impl Into<String>) {
        self.set_message(MessageKind::Info, text);
    }

    fn success(&mut self, text: impl Into<String>) {
        self.set_message(MessageKind::Success, text);
    }

    fn error(&mut self, text: impl Into<String>) {
        self.set_message(MessageKind::Error, text);
    }

    fn report_header_error(&mut self) {
        if let Some(e) = self.script.header_error.clone() {
            self.error(e);
        }
    }
}
