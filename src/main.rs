mod app;
mod keys;
mod scanner;
mod tree;
mod ui;

use std::path::PathBuf;
use std::time::Duration;

use crossterm::event::{self, Event, KeyEventKind};

fn main() -> color_eyre::Result<()> {
    color_eyre::install()?;

    let target = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().expect("Cannot access current directory"));

    if !target.is_dir() {
        eprintln!("Error: '{}' is not a directory", target.display());
        std::process::exit(1);
    }

    let mut terminal = ratatui::init();
    let result = run(&mut terminal, target);
    ratatui::restore();

    result
}

fn run(terminal: &mut ratatui::DefaultTerminal, target: PathBuf) -> color_eyre::Result<()> {
    let mut app = app::App::new(target);
    app.start_scan();

    loop {
        app.poll_scan();

        terminal.draw(|frame| ui::render(&mut app, frame))?;

        if event::poll(Duration::from_millis(50))? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    let msg = keys::handle_key(key, &app.mode);
                    if app.update(msg) {
                        break;
                    }
                }
            }
        }
    }

    Ok(())
}
