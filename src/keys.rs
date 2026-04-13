use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::app::{AppMode, Message};

/// Single entry point for all key-to-message translation.
pub fn handle_key(key: KeyEvent, mode: &AppMode) -> Message {
    // Ctrl+C always quits
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
        return match mode {
            AppMode::Scanning => Message::ForceQuit,
            _ => Message::Quit,
        };
    }

    match mode {
        AppMode::Scanning => handle_scanning_key(key),
        AppMode::Browsing => handle_browsing_key(key),
        AppMode::ConfirmDelete => handle_confirm_delete_key(key),
    }
}

fn handle_scanning_key(key: KeyEvent) -> Message {
    match key.code {
        KeyCode::Char('q') => Message::ForceQuit,
        _ => Message::None,
    }
}

fn handle_browsing_key(key: KeyEvent) -> Message {
    match key.code {
        // Navigation
        KeyCode::Up | KeyCode::Char('k') => Message::MoveUp,
        KeyCode::Down | KeyCode::Char('j') => Message::MoveDown,
        KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => Message::ExpandOrEnter,
        KeyCode::Backspace | KeyCode::Left | KeyCode::Char('h') => Message::CollapseOrBack,
        KeyCode::PageUp => Message::PageUp,
        KeyCode::PageDown => Message::PageDown,
        KeyCode::Home | KeyCode::Char('g') => Message::GoToFirst,
        KeyCode::End => Message::GoToLast,
        KeyCode::Char('G') => Message::GoToLast,

        // Actions
        KeyCode::Char('r') => Message::Rescan,
        KeyCode::Char('d') | KeyCode::Delete => Message::RequestDelete,
        KeyCode::Char('q') => Message::Quit,

        _ => Message::None,
    }
}

fn handle_confirm_delete_key(key: KeyEvent) -> Message {
    match key.code {
        KeyCode::Char('y') | KeyCode::Char('Y') => Message::ConfirmDelete,
        KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => Message::CancelDelete,
        _ => Message::None,
    }
}
