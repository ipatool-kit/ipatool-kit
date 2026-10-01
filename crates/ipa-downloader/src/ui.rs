//! Full-screen redraw menus. Raw mode needs `\r\n` / MoveTo — bare `\n` staircases.

use std::io::{self, Write};
use crossterm::cursor::{Hide, MoveTo, Show};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::style::{Print, Stylize};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, size, Clear, ClearType, EnterAlternateScreen,
    LeaveAlternateScreen,
};
use ipatool::IpatoolError;

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
pub fn select<S: AsRef<str>>(
    header: &[String],
    prompt: impl AsRef<str>,
    items: &[S],
) -> Result<usize, IpatoolError> {
    let prompt = prompt.as_ref();
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
            let row = format!("{mark}{}", item.as_ref());
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
        lines.push(crate::i18n::t("ui.hint_select").dim().to_string());
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
    prompt: impl AsRef<str>,
    items: &[String],
) -> Result<Vec<usize>, IpatoolError> {
    let prompt = prompt.as_ref();
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
        lines.push(format!("{}{query}▌", crate::i18n::t("ui.search").cyan()));
        lines.push(
            format!(
                "{} {}/{} · {} {selected_n}",
                crate::i18n::t("ui.shown"),
                visible.len(),
                items.len(),
                crate::i18n::t("ui.selected"),
            )
            .dim()
            .to_string(),
        );
        lines.push(String::new());

        if visible.is_empty() {
            lines.push(crate::i18n::t("ui.no_matches").dim().to_string());
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
            crate::i18n::t("ui.hint_multi")
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

pub fn input_line(header: &[String], prompt: impl AsRef<str>) -> Result<String, IpatoolError> {
    let prompt = prompt.as_ref();
    let mut buf = String::new();
    loop {
        let mut lines = header.to_vec();
        lines.push(String::new());
        lines.push(format!("{prompt}{buf}▌"));
        lines.push(String::new());
        lines.push(
            crate::i18n::t("ui.hint_input")
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

pub fn input_password(header: &[String], prompt: impl AsRef<str>) -> Result<String, IpatoolError> {
    let prompt = prompt.as_ref();
    let mut buf = String::new();
    loop {
        let stars: String = "*".repeat(buf.chars().count());
        let mut lines = header.to_vec();
        lines.push(String::new());
        lines.push(format!("{prompt}{stars}▌"));
        lines.push(String::new());
        lines.push(
            crate::i18n::t("ui.hint_input")
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

pub fn message(header: &[String], body: impl AsRef<str>) -> Result<(), IpatoolError> {
    show_message(header, body.as_ref(), false)
}

/// Like [`message`], but also writes the full text to `~/.ipatool/last-error.txt`
/// and shows that path (for long Apple/HTTP failures).
pub fn error_message(header: &[String], body: impl AsRef<str>) -> Result<(), IpatoolError> {
    show_message(header, body.as_ref(), true)
}

fn show_message(header: &[String], body: &str, save_error: bool) -> Result<(), IpatoolError> {
    let saved = if save_error {
        persist_last_error(body)
    } else {
        None
    };

    let (cols, rows) = size().unwrap_or((80, 24));
    let cols = (cols as usize).saturating_sub(1).max(20);
    let max_rows = rows as usize;

    let mut content = Vec::new();
    if let Some(ref path) = saved {
        content.push(format!("full error saved: {}", path.display()));
        content.push(String::new());
    }
    for part in body.lines() {
        content.extend(wrap_plain(part, cols));
    }

    // Reserve header + blank + footer ("Enter…") + optional overflow note.
    let header_rows = header.len() + 1;
    let footer_rows = 2;
    let budget = max_rows.saturating_sub(header_rows + footer_rows).max(1);
    let overflow = content.len().saturating_sub(budget);
    let shown = if overflow > 0 {
        let mut s = content[..budget.saturating_sub(1)].to_vec();
        s.push(format!("… ({overflow} more lines — open the file above)"));
        s
    } else {
        content
    };

    let mut lines = header.to_vec();
    lines.push(String::new());
    lines.extend(shown);
    lines.push(String::new());
    lines.push(crate::i18n::t("ui.enter_continue").dim().to_string());
    paint(&lines)?;
    loop {
        match read_key()? {
            KeyCode::Enter | KeyCode::Esc | KeyCode::Char('q') => return Ok(()),
            _ => {}
        }
    }
}

fn persist_last_error(body: &str) -> Option<std::path::PathBuf> {
    let dir = ipatool::session::ipatool_dir()?;
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("last-error.txt");
    std::fs::write(&path, body).ok()?;
    Some(path)
}

fn wrap_plain(line: &str, width: usize) -> Vec<String> {
    if line.is_empty() {
        return vec![String::new()];
    }
    if width == 0 {
        return vec![line.to_string()];
    }
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut cols = 0usize;
    for c in line.chars() {
        if cols >= width {
            out.push(std::mem::take(&mut cur));
            cols = 0;
        }
        cur.push(c);
        cols += 1;
    }
    if !cur.is_empty() || out.is_empty() {
        out.push(cur);
    }
    out
}

/// Paint a status screen without waiting for a key (live progress).
pub fn status(header: &[String], body: impl AsRef<str>) -> Result<(), IpatoolError> {
    let body = body.as_ref();
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
