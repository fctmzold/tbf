use std::cell::Cell;
use std::io;

use anyhow::Result;
use crossterm::event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode};
use crossterm::event::{MouseButton, MouseEventKind};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
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
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let outcome = run_loop(&mut terminal);

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    outcome
}

fn run_loop(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> Result<Option<String>> {
    const COMMANDS: [&str; 7] = [
        "Exact - Check specific timestamp",
        "Bruteforce - Search timestamp range",
        "Clipforce - Scan for clips",
        "Link - Extract from TwitchTracker",
        "Live - Find currently live VOD",
        "Fix - Fix unmuted playlist",
        "Quit",
    ];

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

            let items: Vec<ListItem> = COMMANDS
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
            if offset + visible_height > COMMANDS.len() && COMMANDS.len() >= visible_height {
                offset = COMMANDS.len() - visible_height;
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
            Event::Key(key) => match key.code {
                KeyCode::Char('q') => return Ok(None),
                KeyCode::Enter => {
                    if selected == COMMANDS.len() - 1 {
                        return Ok(None);
                    }
                    let key = COMMANDS[selected]
                        .split(" -")
                        .next()
                        .unwrap_or("")
                        .to_lowercase();
                    return Ok(Some(key));
                }
                KeyCode::Up => selected = selected.saturating_sub(1),
                KeyCode::Down if selected + 1 < COMMANDS.len() => selected += 1,
                KeyCode::Down => {}
                _ => {}
            },
            Event::Mouse(mouse) => {
                let list_area = list_area_cell.get();
                let inside = mouse.column >= list_area.x
                    && mouse.column < list_area.x + list_area.width
                    && mouse.row >= list_area.y
                    && mouse.row < list_area.y + list_area.height;
                if inside {
                    match mouse.kind {
                        MouseEventKind::Down(MouseButton::Left) => {
                            // Subtract borders (1) plus current scroll offset.
                            let relative = (mouse.row - list_area.y).saturating_sub(1) as usize;
                            let clicked = relative + offset;
                            if clicked < COMMANDS.len() {
                                selected = clicked;
                            }
                        }
                        MouseEventKind::ScrollUp => {
                            selected = selected.saturating_sub(1);
                        }
                        MouseEventKind::ScrollDown if selected + 1 < COMMANDS.len() => {
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
