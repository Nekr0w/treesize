use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;
use tui_tree_widget::{Tree, TreeItem};

use crate::app::{App, AppMode};
use crate::tree::{format_size, FileNode, ScanState};

// ── Color thresholds (single source of truth) ─────────────────────

const THRESHOLD_TB: u64 = 1_099_511_627_776;
const THRESHOLD_GB: u64 = 1_073_741_824;
const THRESHOLD_100MB: u64 = 104_857_600;
const THRESHOLD_10MB: u64 = 10_485_760;
const THRESHOLD_1MB: u64 = 1_048_576;

fn size_color(bytes: u64) -> Color {
    if bytes >= THRESHOLD_TB {
        Color::Magenta
    } else if bytes >= THRESHOLD_GB {
        Color::Red
    } else if bytes >= THRESHOLD_100MB {
        Color::Yellow
    } else if bytes >= THRESHOLD_10MB {
        Color::Green
    } else if bytes >= THRESHOLD_1MB {
        Color::Cyan
    } else {
        Color::DarkGray
    }
}

// ── Size bar (single source of truth) ─────────────────────────────

const BAR_FULL: char = '\u{2588}'; // █
const BAR_EMPTY: char = '\u{2591}'; // ░
const BAR_WIDTH: usize = 12;

fn render_size_bar(percentage: f64) -> String {
    let filled = ((percentage / 100.0) * BAR_WIDTH as f64).round() as usize;
    let filled = filled.min(BAR_WIDTH);
    let empty = BAR_WIDTH - filled;
    format!(
        "{}{}",
        BAR_FULL.to_string().repeat(filled),
        BAR_EMPTY.to_string().repeat(empty),
    )
}

// ── Display line (single composition function) ────────────────────

fn build_display_line(node: &FileNode, parent_size: u64) -> Line<'static> {
    let type_indicator = if node.is_dir { "[D] " } else { "[F] " };
    let name = if node.is_dir {
        format!("{}/", node.name)
    } else {
        node.name.clone()
    };

    let size_str = format_size(node.size);
    let percentage = node.percentage_of(parent_size);
    let bar = render_size_bar(percentage);
    let color = size_color(node.size);

    let error_suffix = node
        .error
        .as_ref()
        .map(|e| format!(" ⚠ {e}"))
        .unwrap_or_default();

    Line::from(vec![
        Span::styled(type_indicator, Style::default().fg(Color::DarkGray)),
        Span::styled(
            name,
            Style::default()
                .fg(color)
                .add_modifier(if node.is_dir {
                    Modifier::BOLD
                } else {
                    Modifier::empty()
                }),
        ),
        Span::raw("  "),
        Span::styled(format!("{size_str:>10}"), Style::default().fg(color)),
        Span::raw("  "),
        Span::styled(bar, Style::default().fg(color)),
        Span::styled(
            format!("  {percentage:5.1}%"),
            Style::default().fg(Color::DarkGray),
        ),
        Span::styled(error_suffix, Style::default().fg(Color::Red)),
    ])
}

// ── FileNode → TreeItem conversion (single recursive function) ────

fn node_to_tree_item(node: &FileNode, parent_size: u64) -> TreeItem<'static, String> {
    let id = node.path.to_string_lossy().into_owned();
    let line = build_display_line(node, parent_size);

    let is_scanned = matches!(node.scan_state, ScanState::Scanned);

    // Leaf: file, or genuinely empty scanned directory
    if node.children.is_empty() && (!node.is_dir || is_scanned) {
        return TreeItem::new_leaf(id, line);
    }

    // Unscanned/scanning directory with no children yet → placeholder
    if node.children.is_empty() {
        let placeholder = loading_placeholder(&node.path);
        return TreeItem::new(id, line, vec![placeholder])
            .expect("TreeItem children should be valid");
    }

    // Has real children → render them
    let children: Vec<TreeItem<'static, String>> = node
        .children
        .iter()
        .map(|c| node_to_tree_item(c, node.size))
        .collect();
    TreeItem::new(id, line, children).expect("TreeItem children should be valid")
}

fn loading_placeholder(parent_path: &std::path::Path) -> TreeItem<'static, String> {
    let id = format!("{}/__placeholder__", parent_path.display());
    let text = "  ▸ Press Enter to load";
    TreeItem::new_leaf(
        id,
        Line::from(Span::styled(
            text,
            Style::default().fg(Color::DarkGray).add_modifier(Modifier::ITALIC),
        )),
    )
}

// ── Main render ───────────────────────────────────────────────────

pub fn render(app: &mut App, frame: &mut Frame) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // header
            Constraint::Min(1),   // tree
            Constraint::Length(1), // footer
        ])
        .split(frame.area());

    render_header(app, frame, chunks[0]);
    render_tree(app, frame, chunks[1]);
    render_footer(app, frame, chunks[2]);

    if app.mode == AppMode::ConfirmDelete {
        render_confirm_dialog(app, frame);
    }
}

// ── Header ────────────────────────────────────────────────────────

const SPINNER_CHARS: &[char] = &['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

fn render_header(app: &App, frame: &mut Frame, area: Rect) {
    let total_size = app
        .root
        .as_ref()
        .map(|r| format_size(r.size))
        .unwrap_or_default();

    let pending = app.scanner.pending_count();
    let scan_indicator = if pending > 0 {
        let tick = (std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
            / 80) as usize;
        let spinner = SPINNER_CHARS[tick % SPINNER_CHARS.len()];
        format!("  {spinner} scanning ({pending})")
    } else {
        String::new()
    };

    let status = app.status_message.as_deref().unwrap_or("");

    let content = format!(
        " TreeSize — {}  [{total_size}]{scan_indicator}  {status}",
        app.target_path.display(),
    );

    let header = Paragraph::new(content).style(
        Style::default()
            .fg(Color::White)
            .bg(Color::DarkGray)
            .add_modifier(Modifier::BOLD),
    );
    frame.render_widget(header, area);
}

// ── Tree view ─────────────────────────────────────────────────────

fn render_tree(app: &mut App, frame: &mut Frame, area: Rect) {
    let Some(root) = &app.root else {
        let loading =
            Paragraph::new("  Scanning...").style(Style::default().fg(Color::DarkGray));
        frame.render_widget(loading, area);
        return;
    };

    if root.children.is_empty() {
        let msg = match root.scan_state {
            ScanState::Scanned => "  (empty directory)",
            _ => "  Scanning...",
        };
        let loading = Paragraph::new(msg).style(Style::default().fg(Color::DarkGray));
        frame.render_widget(loading, area);
        return;
    }

    let items: Vec<TreeItem<'static, String>> = root
        .children
        .iter()
        .map(|c| node_to_tree_item(c, root.size))
        .collect();

    let tree = Tree::new(&items)
        .expect("Tree items should be valid")
        .block(Block::default())
        .highlight_style(
            Style::default()
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▸ ");

    frame.render_stateful_widget(tree, area, &mut app.tree_state);
}

// ── Footer ────────────────────────────────────────────────────────

fn render_footer(app: &App, frame: &mut Frame, area: Rect) {
    let hints = match app.mode {
        AppMode::Scanning => " q/Ctrl+C: Quit",
        AppMode::Browsing => {
            " ↑↓/jk: Navigate  Enter/l: Expand  Bksp/h: Collapse  r: Rescan  d: Delete  q: Quit"
        }
        AppMode::ConfirmDelete => " y: Confirm  n/Esc: Cancel",
    };

    let footer =
        Paragraph::new(hints).style(Style::default().fg(Color::White).bg(Color::DarkGray));
    frame.render_widget(footer, area);
}

// ── Confirmation dialog ───────────────────────────────────────────

fn render_confirm_dialog(app: &App, frame: &mut Frame) {
    let Some((path, size, _is_dir)) = &app.delete_target else {
        return;
    };

    let area = frame.area();
    let dialog_width = (area.width * 50 / 100).max(40).min(area.width);
    let dialog_height = 5_u16.min(area.height);
    let dialog_area = centered_rect(dialog_width, dialog_height, area);

    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let size_str = format_size(*size);

    let text = format!("Delete \"{name}\" ({size_str})?\n\n[y] Yes   [n] No");

    let dialog = Paragraph::new(text)
        .style(Style::default().fg(Color::White))
        .block(
            Block::default()
                .title(" Confirm Delete ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Red)),
        );

    frame.render_widget(Clear, dialog_area);
    frame.render_widget(dialog, dialog_area);
}

fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let x = area.x + area.width.saturating_sub(width) / 2;
    let y = area.y + area.height.saturating_sub(height) / 2;
    Rect::new(x, y, width.min(area.width), height.min(area.height))
}
