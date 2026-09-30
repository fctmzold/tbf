use std::cell::Cell;
use std::io;
use std::time::Duration;

use anyhow::Result;
use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, KeyModifiers,
};
use crossterm::event::{MouseButton, MouseEventKind};
use crossterm::execute;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};
use ratatui::Terminal;

/// Menu labels shown in the picker; the last entry always quits.
const COMMAND_LABELS: [&str; 8] = [
    "Exact - Check specific timestamp",
    "Bruteforce - Search timestamp range",
    "Clipforce - Scan for clips",
    "Link - Extract from TwitchTracker",
    "Live - Find currently live VOD",
    "Vods - List channel VODs with links",
    "Fix - Fix unmuted playlist",
    "Quit",
];

/// Command keys returned by the picker, aligned with `COMMAND_LABELS`.
const COMMAND_KEYS: [&str; 7] = [
    "exact",
    "bruteforce",
    "clipforce",
    "link",
    "live",
    "vods",
    "fix",
];

/// Map a menu index to its command key without parsing display text.
fn command_key(selected: usize) -> String {
    COMMAND_KEYS[selected].to_string()
}

/// Row index inside a bordered widget, or `None` on its frame and outside.
fn inner_row(mouse_row: u16, area: Rect) -> Option<usize> {
    let row = mouse_row.checked_sub(area.y)? as usize;
    if row == 0 || row + 1 >= area.height as usize {
        return None;
    }
    Some(row - 1)
}

/// Whether a click column falls inside a bordered widget excluding its frame.
fn inner_hit(column: u16, area: Rect) -> bool {
    column > area.x && column + 1 < area.x + area.width
}

/// Run the interactive command picker.
///
/// # Returns
///
/// Lowercase command key (`exact`, `bruteforce`, `clipforce`, `link`, `live`,
/// `vods`, `fix`) or `None` when the user quits.
///
/// # Errors
///
/// Returns an error when terminal setup, drawing, or event reading fails.
pub fn run() -> Result<Option<String>> {
    // `init` installs a panic hook, so a panic cannot leave the terminal
    // in raw mode or on the alternate screen.
    let mut terminal = ratatui::init();
    execute!(terminal.backend_mut(), EnableMouseCapture)?;

    let outcome = run_loop(&mut terminal);

    let _ = execute!(terminal.backend_mut(), DisableMouseCapture);
    ratatui::restore();
    outcome
}

fn run_loop(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> Result<Option<String>> {
    let mut selected = 0_usize;
    let mut offset = 0_usize;
    // `Cell` shares the drawn list area with the mouse handler.
    // Rust novices: `Cell` gives interior mutability for `Copy` types
    // without needing a mutable borrow.
    let list_area_cell = Cell::new(Rect::default());

    loop {
        terminal.draw(|frame| {
            let area = frame.area();
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .margin(1)
                .constraints([Constraint::Length(3), Constraint::Min(0)])
                .split(area);

            let title = Paragraph::new(Line::from("Twitch Broadcast Finder"))
                .style(
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                )
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title("Interactive Mode"),
                );
            frame.render_widget(title, chunks[0]);

            let items: Vec<ListItem> = COMMAND_LABELS
                .iter()
                .enumerate()
                .map(|(index, command)| {
                    let style = if index == selected {
                        Style::default()
                            .fg(Color::Black)
                            .bg(Color::Cyan)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default()
                    };
                    ListItem::new(*command).style(style)
                })
                .collect();

            let list_area = chunks[1];
            list_area_cell.set(list_area);
            let visible_height = list_area.height.saturating_sub(2).max(1) as usize;

            if selected < offset {
                offset = selected;
            } else if selected >= offset + visible_height {
                offset = selected - visible_height + 1;
            }
            if offset + visible_height > COMMAND_LABELS.len()
                && COMMAND_LABELS.len() >= visible_height
            {
                offset = COMMAND_LABELS.len() - visible_height;
            }

            let end = (offset + visible_height).min(items.len());
            let visible_items = items[offset..end].to_vec();
            let list = List::new(visible_items).block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Commands (Use arrows or mouse)"),
            );
            frame.render_widget(list, list_area);
        })?;

        if !event::poll(Duration::from_millis(150))? {
            continue;
        }
        match event::read()? {
            Event::Key(key) if key.kind == KeyEventKind::Press => match (key.code, key.modifiers) {
                (KeyCode::Char('c'), KeyModifiers::CONTROL)
                | (KeyCode::Esc | KeyCode::Char('q'), _) => return Ok(None),
                (KeyCode::Enter, _) => {
                    if selected == COMMAND_LABELS.len() - 1 {
                        return Ok(None);
                    }
                    return Ok(Some(command_key(selected)));
                }
                (KeyCode::Up, _) => selected = selected.saturating_sub(1),
                (KeyCode::Down, _) if selected + 1 < COMMAND_LABELS.len() => {
                    selected += 1;
                }
                _ => {}
            },
            Event::Key(_) => {}
            Event::Mouse(mouse) => {
                let list_area = list_area_cell.get();
                if !inner_hit(mouse.column, list_area) {
                    continue;
                }
                let Some(relative) = inner_row(mouse.row, list_area) else {
                    continue;
                };
                let clicked = relative + offset;
                if clicked >= COMMAND_LABELS.len() {
                    continue;
                }
                match mouse.kind {
                    MouseEventKind::Down(MouseButton::Left) => {
                        if clicked == selected {
                            if selected == COMMAND_LABELS.len() - 1 {
                                return Ok(None);
                            }
                            return Ok(Some(command_key(selected)));
                        }
                        selected = clicked;
                    }
                    MouseEventKind::ScrollUp => {
                        selected = selected.saturating_sub(1);
                    }
                    MouseEventKind::ScrollDown if selected + 1 < COMMAND_LABELS.len() => {
                        selected += 1;
                    }
                    MouseEventKind::ScrollDown => {}
                    _ => {}
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menu_keys_align_with_labels() {
        assert_eq!(COMMAND_LABELS.len(), COMMAND_KEYS.len() + 1);
        assert_eq!(COMMAND_LABELS[COMMAND_LABELS.len() - 1], "Quit");
        assert_eq!(command_key(0), "exact");
        assert_eq!(command_key(5), "vods");
    }

    #[test]
    fn inner_row_rejects_frame() {
        let area = Rect::new(0, 10, 40, 8);
        assert_eq!(inner_row(10, area), None);
        assert_eq!(inner_row(17, area), None);
        assert_eq!(inner_row(9, area), None);
        assert_eq!(inner_row(11, area), Some(0));
    }

    #[test]
    fn inner_hit_rejects_frame() {
        let area = Rect::new(5, 0, 40, 8);
        assert!(!inner_hit(5, area));
        assert!(!inner_hit(44, area));
        assert!(inner_hit(6, area));
    }
}
