mod app;
mod keys;
mod scanner;
mod store;
mod tree;
mod ui;

use std::path::{Path, PathBuf};
use std::time::Duration;

use crossterm::event::{self, Event, KeyEventKind};

fn main() -> std::io::Result<()> {
    let mut args = std::env::args().skip(1);
    let app = match args.next().as_deref() {
        // Reopen a saved scan: no recompute.
        Some("--load") | Some("-l") => {
            let Some(file) = args.next() else {
                eprintln!("Usage: treesize --load <file>");
                std::process::exit(1);
            };
            match store::load(Path::new(&file)) {
                Ok((root, cache)) => {
                    let mut app = app::App::new(root);
                    app.load_scan(cache);
                    app
                }
                Err(e) => {
                    eprintln!("Cannot load '{file}': {e}");
                    std::process::exit(1);
                }
            }
        }
        // Otherwise scan a directory (arg or cwd).
        other => {
            let target = other
                .map(PathBuf::from)
                .unwrap_or_else(|| std::env::current_dir().expect("Cannot access current directory"));
            if !target.is_dir() {
                eprintln!("Error: '{}' is not a directory", target.display());
                std::process::exit(1);
            }
            let mut app = app::App::new(target);
            app.start_scan();
            app
        }
    };

    let mut terminal = ratatui::init();
    let result = run(&mut terminal, app);
    ratatui::restore();

    result
}

fn run(terminal: &mut ratatui::DefaultTerminal, mut app: app::App) -> std::io::Result<()> {
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
