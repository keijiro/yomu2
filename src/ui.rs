//! Rendering.

use crate::app::{App, Field, MessageKind, RowStatus};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Style, Stylize};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Cell, Clear, Paragraph, Row, Table, Wrap};

const SPINNER: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

pub fn draw(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    let message_height = message_height(app, area.width);
    let [header, body, message, keys] = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(3),
        Constraint::Length(message_height),
        Constraint::Length(1),
    ])
    .areas(area);

    draw_header(frame, app, header);
    draw_table(frame, app, body);
    draw_message(frame, app, message);
    draw_keys(frame, app, keys);
    if app.edit.is_some() {
        draw_editor(frame, app, body);
    }
}

fn dim() -> Style {
    Style::new().fg(Color::DarkGray)
}

fn draw_header(frame: &mut Frame, app: &App, area: Rect) {
    let s = &app.script;
    let voice = match s.voice() {
        Some(v) => Span::raw(v.to_string()),
        None => Span::styled("(not set)", Style::new().fg(Color::Red)),
    };
    let style = match s.global_style() {
        Some(v) => Span::raw(v.to_string()),
        None => Span::styled("(none)", dim()),
    };
    let lines = vec![
        Line::from(vec![
            Span::styled(" yomu ", Style::new().reversed().bold()),
            Span::styled("  voice ", dim()),
            voice,
            Span::styled("  model ", dim()),
            Span::raw(s.model().to_string()),
        ]),
        Line::from(vec![Span::styled("        style ", dim()), style]),
    ];
    frame.render_widget(Paragraph::new(lines), area);
}

fn wrap(text: &str, width: usize) -> Vec<Line<'static>> {
    let options =
        textwrap::Options::new(width.max(1)).wrap_algorithm(textwrap::WrapAlgorithm::FirstFit);
    textwrap::wrap(text, options)
        .into_iter()
        .map(|l| Line::raw(l.into_owned()))
        .collect()
}

fn draw_table(frame: &mut Frame, app: &mut App, area: Rect) {
    let count = app.script.paragraphs.len();
    let position = app
        .selected()
        .map_or(String::new(), |i| format!(" {}/{count} ", i + 1));
    let title = Line::from(format!(" {} ", app.display_name)).bold();
    let block = Block::bordered()
        .title(title)
        .title_top(Line::from(position).right_aligned());
    let inner = block.inner(area);

    if count == 0 {
        frame.render_widget(block, area);
        let hint = Paragraph::new("Empty script. Press o (or Enter) to add a paragraph.")
            .style(dim())
            .centered();
        frame.render_widget(hint, inner.inner(ratatui::layout::Margin::new(1, 1)));
        return;
    }

    // Column widths: index, status, style, text (spacing 1).
    let index_w = count.to_string().len().max(2);
    // One extra column keeps the text off the right border.
    let rest = (inner.width as usize).saturating_sub(index_w + 1 + 3 + 1);
    let style_w = (rest / 4).clamp(8, 32).min(rest / 2).max(1);
    let text_w = rest.saturating_sub(style_w).max(1);

    let selected = app.selected();
    let global_style = app.script.global_style();
    let rows: Vec<Row> = app
        .script
        .paragraphs
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let (symbol, symbol_style) = match app.row_status(i) {
                RowStatus::None => ("·", dim()),
                RowStatus::Busy => (
                    SPINNER[app.tick % SPINNER.len()],
                    Style::new().fg(Color::Yellow),
                ),
                RowStatus::Ready => ("●", Style::new().fg(Color::Green)),
                RowStatus::Playing => ("▶", Style::new().fg(Color::Cyan).bold()),
                RowStatus::Failed => ("✗", Style::new().fg(Color::Red).bold()),
            };
            let (style_text, style_style) = match (&p.style, global_style) {
                (Some(s), _) => (s.as_str(), Style::new().fg(Color::Magenta)),
                (None, Some(g)) => (g, dim()),
                (None, None) => ("—", dim()),
            };
            let style_lines = wrap(style_text, style_w);
            let text_lines = wrap(&p.text, text_w);
            let height = style_lines.len().max(text_lines.len()).max(1);
            let index_style = if Some(i) == selected {
                Style::new().fg(Color::Yellow).bold()
            } else {
                dim()
            };
            Row::new(vec![
                Cell::from(format!("{:>index_w$}", i + 1)).style(index_style),
                Cell::from(symbol).style(symbol_style),
                Cell::from(Text::from(style_lines)).style(style_style),
                Cell::from(Text::from(text_lines)),
            ])
            .height(height as u16)
        })
        .collect();

    let header = Row::new(["#", "", "Style", "Text"]).style(dim().bold());
    let table = Table::new(
        rows,
        [
            Constraint::Length(index_w as u16),
            Constraint::Length(1),
            Constraint::Length(style_w as u16),
            Constraint::Length(text_w as u16),
        ],
    )
    .header(header)
    .block(block)
    .row_highlight_style(Style::new().bold())
    .cell_highlight_style(Style::new().reversed());

    app.table.select_column(Some(app.field.column()));
    frame.render_stateful_widget(table, area, &mut app.table);
}

fn message_height(app: &App, width: u16) -> u16 {
    match &app.message {
        Some(m) => {
            let lines = textwrap::wrap(&m.text, (width as usize).saturating_sub(2).max(1));
            lines.len().clamp(1, 3) as u16
        }
        None => 1,
    }
}

fn draw_message(frame: &mut Frame, app: &App, area: Rect) {
    let Some(message) = &app.message else { return };
    let style = match message.kind {
        MessageKind::Info => Style::new(),
        MessageKind::Success => Style::new().fg(Color::Green),
        MessageKind::Error => Style::new().fg(Color::Red),
    };
    let paragraph = Paragraph::new(format!(" {}", message.text))
        .style(style)
        .wrap(Wrap { trim: false });
    frame.render_widget(paragraph, area);
}

fn draw_keys(frame: &mut Frame, app: &App, area: Rect) {
    let keys: &[(&str, &str)] = match &app.edit {
        Some(edit) if edit.field == Field::Text => &[
            ("Enter", "save"),
            ("Esc", "cancel"),
            ("Alt+Enter", "newline"),
            ("^U/^K", "kill"),
        ],
        Some(edit) if edit.field == Field::GlobalStyle => &[
            ("Enter", "save"),
            ("Esc", "cancel"),
            ("blank", "= no global style"),
        ],
        Some(_) => &[
            ("Enter", "save"),
            ("Esc", "cancel"),
            ("blank", "= default style"),
        ],
        None => &[
            ("Enter", "edit"),
            ("r", "synth"),
            ("Space", "play"),
            ("e", "export"),
            ("m", "model"),
            ("s", "global style"),
            ("o/O", "add"),
            ("dd", "del"),
            ("u", "undo"),
            ("q", "quit"),
        ],
    };
    let mut spans = vec![Span::raw(" ")];
    for (key, desc) in keys {
        spans.push(Span::styled(*key, Style::new().fg(Color::Cyan).bold()));
        spans.push(Span::styled(format!(" {desc}  "), dim()));
    }
    frame.render_widget(Line::from(spans), area);
}

fn draw_editor(frame: &mut Frame, app: &mut App, body: Rect) {
    let Some(edit) = &app.edit else { return };
    let width = body.width.saturating_sub(4).max(10).min(body.width);
    let inner_width = width.saturating_sub(2);
    let layout = edit.editor.layout(inner_width as usize);
    let max_lines = body.height.saturating_sub(2).max(1);
    let lines = (layout.lines.len() as u16).clamp(1, max_lines);
    let height = lines + 2;
    let area = Rect {
        x: body.x + (body.width - width) / 2,
        y: body.y + body.height.saturating_sub(height) / 2,
        width,
        height: height.min(body.height),
    };

    let title = match edit.field {
        Field::Text => format!(" Edit text · paragraph {} ", edit.index + 1),
        Field::Style => format!(" Edit style · paragraph {} ", edit.index + 1),
        Field::GlobalStyle => " Edit global style ".to_string(),
    };
    let block = Block::bordered()
        .title(title)
        .border_style(Style::new().fg(Color::Cyan));
    let inner = block.inner(area);
    let (cx, cy) = layout.cursor;
    let scroll = cy.saturating_sub(inner.height.saturating_sub(1));
    let text: Vec<Line> = layout.lines.into_iter().map(Line::raw).collect();

    frame.render_widget(Clear, area);
    frame.render_widget(Paragraph::new(text).block(block).scroll((scroll, 0)), area);
    frame.set_cursor_position((inner.x + cx, inner.y + cy - scroll));
    app.edit_width = inner_width as usize;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::App;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn screen(terminal: &Terminal<TestBackend>) -> String {
        let buf = terminal.backend().buffer();
        let mut out = String::new();
        for y in 0..buf.area.height {
            let mut skip = 0;
            for x in 0..buf.area.width {
                if skip > 0 {
                    skip -= 1;
                    continue;
                }
                let sym = buf[(x, y)].symbol();
                skip = unicode_width::UnicodeWidthStr::width(sym).saturating_sub(1);
                out.push_str(sym);
            }
            out.push('\n');
        }
        out
    }

    fn key(app: &mut App, code: KeyCode) {
        app.on_key(KeyEvent::new(code, KeyModifiers::NONE));
    }

    #[test]
    fn renders_and_edits() {
        let dir = std::env::temp_dir().join(format!("yomu-ui-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("script.md");
        std::fs::write(
            &path,
            "---\nvoice: voice_test\nstyle: 落ち着いたトーンで\n---\n\n\
             こんにちは。今日は新しいツールについて紹介します。このツールを使うと台本を簡単に音声化できます。\n\n\
             <!-- style: 明るく元気に -->\nまずは基本から。\n\nThanks for watching!\n",
        )
        .unwrap();
        let mut app = App::open(&path).unwrap();
        let mut terminal = Terminal::new(TestBackend::new(80, 16)).unwrap();
        terminal.draw(|f| draw(f, &mut app)).unwrap();
        let s = screen(&terminal);
        println!("{s}");
        assert!(s.contains("voice_test"));
        assert!(s.contains("明るく元気に"));
        assert!(s.contains(" 1/3 "));

        // Edit the style of paragraph 2.
        key(&mut app, KeyCode::Char('j'));
        key(&mut app, KeyCode::Char('h'));
        key(&mut app, KeyCode::Enter);
        app.on_paste("！");
        terminal.draw(|f| draw(f, &mut app)).unwrap();
        println!("{}", screen(&terminal));
        key(&mut app, KeyCode::Enter);
        let saved = std::fs::read_to_string(&path).unwrap();
        assert!(saved.contains("<!-- style: 明るく元気に！ -->"));

        // Add a paragraph below, then delete it and undo.
        key(&mut app, KeyCode::Char('o'));
        app.on_paste("新しい段落");
        key(&mut app, KeyCode::Enter);
        assert_eq!(app.script.paragraphs[2].text, "新しい段落");
        key(&mut app, KeyCode::Char('d'));
        key(&mut app, KeyCode::Char('d'));
        assert_eq!(app.script.paragraphs.len(), 3);
        key(&mut app, KeyCode::Char('u'));
        assert_eq!(app.script.paragraphs.len(), 4);
        assert!(
            std::fs::read_to_string(&path)
                .unwrap()
                .contains("新しい段落")
        );

        // Cancelling a new paragraph leaves the file untouched.
        let before = std::fs::read_to_string(&path).unwrap();
        key(&mut app, KeyCode::Char('O'));
        key(&mut app, KeyCode::Esc);
        assert_eq!(app.script.paragraphs.len(), 4);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), before);

        // Space on an unsynthesized row only shows a message.
        key(&mut app, KeyCode::Char(' '));
        terminal.draw(|f| draw(f, &mut app)).unwrap();
        let s = screen(&terminal);
        println!("{s}");
        assert!(s.contains("Not synthesized yet"));

        // Export with nothing synthesized leaves no export folder behind.
        key(&mut app, KeyCode::Char('e'));
        assert!(
            app.message
                .as_ref()
                .unwrap()
                .text
                .starts_with("Nothing to export")
        );
        assert!(!dir.join("script.export").exists());

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn exports_synthesized_paragraphs_by_number() {
        let dir = std::env::temp_dir().join(format!("yomu-export-app-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("script.audio")).unwrap();
        let path = dir.join("script.md");
        std::fs::write(
            &path,
            "---\nvoice: v\nstyle: calm\n---\n\nOne.\n\n<!-- style: loud -->\nTwo.\n\nThree.\n",
        )
        .unwrap();
        // Fake audio for paragraphs 1 and 3, named by cache key as yomu does.
        for (style, text) in [("calm", "One."), ("calm", "Three.")] {
            let key = crate::audio::cache_key(crate::script::DEFAULT_MODEL, "v", Some(style), text);
            let wav = dir.join("script.audio").join(format!("{key}.wav"));
            std::fs::write(wav, text).unwrap();
        }

        let mut app = App::open(&path).unwrap();
        key(&mut app, KeyCode::Char('e'));

        let out = dir.join("script.export");
        assert_eq!(std::fs::read_to_string(out.join("01.wav")).unwrap(), "One.");
        assert_eq!(
            std::fs::read_to_string(out.join("03.wav")).unwrap(),
            "Three."
        );
        assert!(!out.join("02.wav").exists());
        let msg = &app.message.as_ref().unwrap().text;
        assert!(
            msg.contains("Exported 2 of 3") && msg.contains("not synthesized: 2"),
            "{msg}"
        );

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
