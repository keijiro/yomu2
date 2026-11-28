mod app;
mod audio;
mod editor;
mod script;
mod tts;
mod ui;
mod voice;

use app::App;
use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{
    self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyEventKind,
};
use ratatui::crossterm::execute;
use std::io;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

const USAGE: &str = "\
yomu - script editor and narrator (Gemini TTS voice replication)

Usage: yomu [SCRIPT]    (default: script.md)
       yomu voice register|list|delete   (manage voices; see `yomu voice`)

Keys:
  arrows / h j k l  move cursor          Enter  edit the selected cell
  r                 synthesize the row   Space  play / stop
  o / O             add paragraph        dd     delete paragraph
  e                 export numbered WAV files to <script>.export/
  u                 undo                 q      quit
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("voice") {
        return match voice::run(&args[1..]) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("error: {e}");
                ExitCode::FAILURE
            }
        };
    }
    let mut path = PathBuf::from("script.md");
    for arg in args {
        match arg.as_str() {
            "-h" | "--help" => {
                print!("{USAGE}");
                return ExitCode::SUCCESS;
            }
            "-V" | "--version" => {
                println!("yomu {}", env!("CARGO_PKG_VERSION"));
                return ExitCode::SUCCESS;
            }
            _ => path = PathBuf::from(arg),
        }
    }

    let mut app = match App::open(&path) {
        Ok(app) => app,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::FAILURE;
        }
    };

    // Installed before ratatui::init so the terminal is restored first.
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = execute!(io::stdout(), DisableBracketedPaste);
        hook(info);
    }));

    let mut terminal = ratatui::init();
    let _ = execute!(io::stdout(), EnableBracketedPaste);
    let result = run(&mut terminal, &mut app);
    let _ = execute!(io::stdout(), DisableBracketedPaste);
    ratatui::restore();

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(terminal: &mut DefaultTerminal, app: &mut App) -> io::Result<()> {
    while !app.should_quit {
        terminal.draw(|frame| ui::draw(frame, app))?;
        if event::poll(Duration::from_millis(100))? {
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => app.on_key(key),
                Event::Paste(text) => app.on_paste(&text),
                _ => {}
            }
        }
        app.on_tick();
    }
    Ok(())
}
