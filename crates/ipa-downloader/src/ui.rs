//! Full-screen redraw menus. Raw mode needs `\r\n` / MoveTo — bare `\n` staircases.

use std::io::{self, Write};
use std::sync::atomic::{AtomicU8, Ordering};

use crossterm::cursor::{Hide, MoveTo, Show};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::style::{Print, Stylize};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, size, Clear, ClearType, EnterAlternateScreen,
    LeaveAlternateScreen,
};
use ipatool::IpatoolError;

const LANG_EN: u8 = 0;
const LANG_RU: u8 = 1;
static UI_LANG: AtomicU8 = AtomicU8::new(LANG_EN);

/// Switch UI chrome language (hints, Search:, etc.).
pub fn set_lang(ru: bool) {
    UI_LANG.store(if ru { LANG_RU } else { LANG_EN }, Ordering::Relaxed);
}

fn is_ru() -> bool {
    UI_LANG.load(Ordering::Relaxed) == LANG_RU
}

fn tr(en: &'static str, ru: &'static str) -> &'static str {
    if is_ru() {
        ru
    } else {
        en
    }
}

pub struct TermGuard;

impl TermGuard {
    pub fn enter() -> Result<Self, IpatoolError> {
        enable_raw_mode().map_err(io_err)?;
        execute!(io::stdout(), EnterAlternateScreen, Hide).map_err(io_err)?;
        Ok(Self)
    }
}

impl Drop for TermGuard {
    fn drop(&mut self) {
        let _ = execute!(io::stdout(), Show, LeaveAlternateScreen);
        let _ = disable_raw_mode();
    }
}

fn io_err(e: impl ToString) -> IpatoolError {
    IpatoolError::Message(e.to_string())
}

fn paint(lines: &[String]) -> Result<(), IpatoolError> {
    let mut out = io::stdout();
    let (cols, rows) = size().unwrap_or((80, 24));
    let cols = cols as usize;
    let max_rows = rows as usize;

    execute!(out, Clear(ClearType::All), MoveTo(0, 0)).map_err(io_err)?;

    for (row, line) in lines.iter().take(max_rows).enumerate() {
        execute!(out, MoveTo(0, row as u16)).map_err(io_err)?;
        let visible = truncate_for_term(line, cols.saturating_sub(1));
        execute!(out, Print(&visible), Print("\x1b[0m"), Print("\r")).map_err(io_err)?;
    }
    out.flush().map_err(io_err)?;
    Ok(())
}

fn truncate_for_term(s: &str, max_cols: usize) -> String {
    if max_cols == 0 {
        return String::new();
    }
    let mut out = String::new();
    let mut cols = 0usize;
    let mut chars = s.chars().peekable();
    let mut truncated = false;
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            out.push(c);
            if chars.peek() == Some(&'[') {
                out.push(chars.next().unwrap());
                for c2 in chars.by_ref() {
                    out.push(c2);
                    if c2.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
            continue;
        }
        if cols >= max_cols {
            truncated = true;
            break;
        }
        out.push(c);
        cols += 1;
    }
    if truncated {
        out.push_str("\x1b[0m");
    }
    out
}

fn read_key() -> Result<KeyCode, IpatoolError> {
    Ok(read_key_event()?.code)
}

fn read_key_event() -> Result<KeyEvent, IpatoolError> {
    loop {
        let ev = event::read().map_err(io_err)?;
        if let Event::Key(k) = ev {
            if k.kind == KeyEventKind::Press {
                return Ok(k);
            }
        }
    }
}

/// Single-choice list. Full clear each key — never paints over scrollback.
pub fn select(header: &[String], prompt: &str, items: &[&str]) -> Result<usize, IpatoolError> {
    if items.is_empty() {
        return Err(IpatoolError::Message("empty select".into()));
    }
    let mut idx = 0usize;
    let view = visible_rows().saturating_sub(header.len() + 6).max(5);
    loop {
        let mut lines = header.to_vec();
        lines.push(String::new());
        lines.push(prompt.to_string());
        lines.push(String::new());

        let start = if items.len() <= view {
            0
        } else {
            idx.saturating_sub(view / 2)
                .min(items.len().saturating_sub(view))
        };
        let end = (start + view).min(items.len());
        if start > 0 {
            lines.push("  …".dim().to_string());
        }
        for (i, item) in items.iter().enumerate().take(end).skip(start) {
            let mark = if i == idx { "❯ " } else { "  " };
            let row = format!("{mark}{item}");
            lines.push(if i == idx {
                row.cyan().bold().to_string()
            } else {
                row
            });
        }
        if end < items.len() {
            lines.push("  …".dim().to_string());
        }
        lines.push(String::new());
        lines.push(
            tr(
                "↑↓ move · Enter confirm · Esc cancel",
                "↑↓ выбор · Enter подтвердить · Esc отмена",
            )
            .dim()
            .to_string(),
        );
        paint(&lines)?;

        match read_key()? {
            KeyCode::Up | KeyCode::Char('k') => {
                idx = if idx == 0 { items.len() - 1 } else { idx - 1 };
            }
            KeyCode::Down | KeyCode::Char('j') => {
                idx = (idx + 1) % items.len();
            }
            KeyCode::Home => idx = 0,
            KeyCode::End => idx = items.len() - 1,
            KeyCode::Enter => return Ok(idx),
            KeyCode::Esc | KeyCode::Char('q') => {
                return Err(IpatoolError::Message("cancelled".into()));
            }
            _ => {}
        }
    }
}

/// Multi-select with live search. Returns indices into the original `items` slice.
pub fn multi_select(
    header: &[String],
    prompt: &str,
    items: &[String],
) -> Result<Vec<usize>, IpatoolError> {
    if items.is_empty() {
        return Ok(Vec::new());
    }
    let mut query = String::new();
    let mut on = vec![false; items.len()];
    let mut idx = 0usize;

    loop {
        let q = query.to_lowercase();
        let visible: Vec<usize> = if q.is_empty() {
            (0..items.len()).collect()
        } else {
            items
                .iter()
                .enumerate()
                .filter(|(_, s)| s.to_lowercase().contains(&q))
                .map(|(i, _)| i)
                .collect()
        };
        if idx >= visible.len() {
            idx = visible.len().saturating_sub(1);
        }

        let selected_n = on.iter().filter(|&&v| v).count();
        let view = visible_rows().saturating_sub(header.len() + 9).max(5);

        let mut lines = header.to_vec();
        lines.push(String::new());
        lines.push(prompt.to_string());
        lines.push(format!("{}{query}▌", tr("Search: ", "Поиск: ").cyan()));
        lines.push(
            format!(
                "{} {}/{} · {} {selected_n}",
                tr("shown", "показано"),
                visible.len(),
                items.len(),
                tr("selected", "выбрано"),
            )
            .dim()
            .to_string(),
        );
        lines.push(String::new());

        if visible.is_empty() {
            lines.push(tr("  (no matches)", "  (нет совпадений)").dim().to_string());
        } else {
            let start = if visible.len() <= view {
                0
            } else {
                idx.saturating_sub(view / 2)
                    .min(visible.len().saturating_sub(view))
            };
            let end = (start + view).min(visible.len());
            if start > 0 {
                lines.push("  …".dim().to_string());
            }
            for (vi, &oi) in visible.iter().enumerate().take(end).skip(start) {
                let boxc = if on[oi] { "[x]" } else { "[ ]" };
                let mark = if vi == idx { "❯ " } else { "  " };
                let row = format!("{mark}{boxc} {}", items[oi]);
                lines.push(if vi == idx {
                    row.cyan().bold().to_string()
                } else {
                    row
                });
            }
            if end < visible.len() {
                lines.push("  …".dim().to_string());
            }
        }

        lines.push(String::new());
        lines.push(
            tr(
                "type search · ↑↓ · Space · * all/none · Enter · Esc",
                "печать = поиск · ↑↓ · Пробел · * всё/снять · Enter · Esc",
            )
            .dim()
            .to_string(),
        );
        paint(&lines)?;

        let key = read_key_event()?;
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);

        match key.code {
            KeyCode::Up => {
                if !visible.is_empty() {
                    idx = if idx == 0 { visible.len() - 1 } else { idx - 1 };
                }
            }
            KeyCode::Down => {
                if !visible.is_empty() {
                    idx = (idx + 1) % visible.len();
                }
            }
            KeyCode::PageUp => {
                idx = idx.saturating_sub(view);
            }
            KeyCode::PageDown => {
                if !visible.is_empty() {
                    idx = (idx + view).min(visible.len() - 1);
                }
            }
            KeyCode::Home => idx = 0,
            KeyCode::End => {
                if !visible.is_empty() {
                    idx = visible.len() - 1;
                }
            }
            KeyCode::Char(' ') => {
                if let Some(&oi) = visible.get(idx) {
                    on[oi] = !on[oi];
                }
            }
            KeyCode::Char('*') | KeyCode::Char('+') => {
                let all_on = !visible.is_empty() && visible.iter().all(|&oi| on[oi]);
                for &oi in &visible {
                    on[oi] = !all_on;
                }
            }
            KeyCode::Char('a') | KeyCode::Char('A') if ctrl => {
                let all_on = !visible.is_empty() && visible.iter().all(|&oi| on[oi]);
                for &oi in &visible {
                    on[oi] = !all_on;
                }
            }
            KeyCode::Char(c) if !c.is_control() && !ctrl => {
                query.push(c);
                idx = 0;
            }
            KeyCode::Backspace => {
                query.pop();
                idx = 0;
            }
            KeyCode::Enter => {
                return Ok(on
                    .iter()
                    .enumerate()
                    .filter_map(|(i, v)| v.then_some(i))
                    .collect());
            }
            KeyCode::Esc => {
                if query.is_empty() {
                    return Ok(Vec::new());
                }
                query.clear();
                idx = 0;
            }
            _ => {}
        }
    }
}

pub fn input_line(header: &[String], prompt: &str) -> Result<String, IpatoolError> {
    let mut buf = String::new();
    loop {
        let mut lines = header.to_vec();
        lines.push(String::new());
        lines.push(format!("{prompt}{buf}▌"));
        lines.push(String::new());
        lines.push(
            tr(
                "Enter confirm · Esc cancel",
                "Enter подтвердить · Esc отмена",
            )
            .dim()
            .to_string(),
        );
        paint(&lines)?;

        match read_key()? {
            KeyCode::Enter => return Ok(buf),
            KeyCode::Esc => return Ok(String::new()),
            KeyCode::Backspace => {
                buf.pop();
            }
            KeyCode::Char(c) => buf.push(c),
            _ => {}
        }
    }
}

pub fn input_password(header: &[String], prompt: &str) -> Result<String, IpatoolError> {
    let mut buf = String::new();
    loop {
        let stars: String = "*".repeat(buf.chars().count());
        let mut lines = header.to_vec();
        lines.push(String::new());
        lines.push(format!("{prompt}{stars}▌"));
        lines.push(String::new());
        lines.push(
            tr(
                "Enter confirm · Esc cancel",
                "Enter подтвердить · Esc отмена",
            )
            .dim()
            .to_string(),
        );
        paint(&lines)?;

        match read_key()? {
            KeyCode::Enter => return Ok(buf),
            KeyCode::Esc => return Ok(String::new()),
            KeyCode::Backspace => {
                buf.pop();
            }
            KeyCode::Char(c) => buf.push(c),
            _ => {}
        }
    }
}

pub fn message(header: &[String], body: &str) -> Result<(), IpatoolError> {
    let mut lines = header.to_vec();
    lines.push(String::new());
    for part in body.lines() {
        lines.push(part.to_string());
    }
    lines.push(String::new());
    lines.push(
        tr("Enter to continue", "Enter — продолжить")
            .dim()
            .to_string(),
    );
    paint(&lines)?;
    loop {
        match read_key()? {
            KeyCode::Enter | KeyCode::Esc | KeyCode::Char('q') => return Ok(()),
            _ => {}
        }
    }
}

/// Paint a status screen without waiting for a key (live progress).
pub fn status(header: &[String], body: &str) -> Result<(), IpatoolError> {
    let mut lines = header.to_vec();
    lines.push(String::new());
    for part in body.lines() {
        lines.push(part.to_string());
    }
    lines.push(String::new());
    lines.push("…".dim().to_string());
    paint(&lines)
}

fn visible_rows() -> usize {
    size().map(|(_, r)| r as usize).unwrap_or(24)
}
