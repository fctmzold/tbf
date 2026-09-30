use std::cell::Cell;
use std::io;

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

/// Run the interactive command picker.
///
/// # Returns
///
/// Lowercase command key (`exact`, `bruteforce`, `clipforce`, `link`, `live`,
/// `fix`) or `None` when the user quits.
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

/// Menu labels shown in the picker; the last entry always quits.
const COMMAND_LABELS: [&str; 7] = [
    "Exact - Check specific timestamp",
    "Bruteforce - Search timestamp range",
    "Clipforce - Scan for clips",
    "Link - Extract from TwitchTracker",
    "Live - Find currently live VOD",
    "Fix - Fix unmuted playlist",
    "Quit",
];

/// Command keys returned by the picker, aligned with `COMMAND_LABELS`.
const COMMAND_KEYS: [&str; 6] = ["exact", "bruteforce", "clipforce", "link", "live", "fix"];

/// Map a menu index to its command key without parsing display text.
fn command_key(selected: usize) -> String {
    COMMAND_KEYS[selected].to_string()
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
            let visible_height = list_area.height.saturating_sub(2) as usize;
            let visible_height = visible_height.max(1);

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
                (KeyCode::Down, _) if selected + 1 < COMMAND_LABELS.len() => selected += 1,
                _ => {}
            },
            Event::Key(_) => {}
            Event::Mouse(mouse) => {
                let list_area = list_area_cell.get();
                let row = mouse.row.saturating_sub(list_area.y) as usize;
                let height = list_area.height as usize;
                let inside_rows = row > 0 && row + 1 < height;
                let inside_columns =
                    mouse.column > list_area.x && mouse.column + 1 < list_area.x + list_area.width;
                if inside_rows && inside_columns {
                    match mouse.kind {
                        MouseEventKind::Down(MouseButton::Left) => {
                            let clicked = row - 1 + offset;
                            if clicked < COMMAND_LABELS.len() {
                                if clicked == selected {
                                    if selected == COMMAND_LABELS.len() - 1 {
                                        return Ok(None);
                                    }
                                    return Ok(Some(command_key(selected)));
                                }
                                selected = clicked;
                            }
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
            }
            _ => {}
        }
    }
}
