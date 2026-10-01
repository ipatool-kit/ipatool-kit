//! Locale catalogs (`locales/en.toml`, `locales/ru.toml`) + lookup helpers.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::OnceLock;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Lang {
    En = 0,
    Ru = 1,
}

static LANG: AtomicU8 = AtomicU8::new(Lang::En as u8);
static TABLES: OnceLock<[HashMap<String, String>; 2]> = OnceLock::new();

fn tables() -> &'static [HashMap<String, String>; 2] {
    TABLES.get_or_init(|| {
        [
            parse_locale(include_str!("../locales/en.toml")),
            parse_locale(include_str!("../locales/ru.toml")),
        ]
    })
}

fn parse_locale(src: &str) -> HashMap<String, String> {
    let value: toml::Value = toml::from_str(src).expect("locale toml");
    let mut out = HashMap::new();
    flatten("", &value, &mut out);
    out
}

fn flatten(prefix: &str, value: &toml::Value, out: &mut HashMap<String, String>) {
    match value {
        toml::Value::Table(table) => {
            for (k, v) in table {
                let key = if prefix.is_empty() {
                    k.clone()
                } else {
                    format!("{prefix}.{k}")
                };
                flatten(&key, v, out);
            }
        }
        toml::Value::String(s) => {
            out.insert(prefix.to_string(), s.clone());
        }
        toml::Value::Integer(i) => {
            out.insert(prefix.to_string(), i.to_string());
        }
        _ => {}
    }
}

/// Load saved / env language and pin catalogs in memory.
pub fn init() {
    let _ = tables();
    set_lang(load_lang());
}

pub fn set_lang(lang: Lang) {
    LANG.store(lang as u8, Ordering::Relaxed);
}

#[allow(dead_code)]
pub fn lang() -> Lang {
    match LANG.load(Ordering::Relaxed) {
        x if x == Lang::Ru as u8 => Lang::Ru,
        _ => Lang::En,
    }
}

/// Look up `key` in the active locale (fallback: English, then the key itself).
///
/// Progress helpers may pass `key\\x1farg` for a single `{n}` / free-form argument
/// (see [`localize_progress`]).
pub fn t(key: &str) -> String {
    if let Some((k, arg)) = key.split_once('\x1f') {
        return tr(k, &[("n", arg), ("arg", arg)]);
    }
    lookup(key)
}

pub fn tr(key: &str, args: &[(&str, &str)]) -> String {
    let mut s = lookup(key);
    for (name, value) in args {
        s = s.replace(&format!("{{{name}}}"), value);
    }
    s
}

fn lookup(key: &str) -> String {
    let idx = LANG.load(Ordering::Relaxed) as usize;
    let tables = tables();
    tables
        .get(idx)
        .and_then(|m| m.get(key))
        .or_else(|| tables[0].get(key))
        .cloned()
        .unwrap_or_else(|| key.to_string())
}

pub fn main_menu_items() -> Vec<String> {
    let n: usize = lookup("main_menu.__len").parse().unwrap_or(0);
    (0..n).map(|i| t(&format!("main_menu.{i}"))).collect()
}

pub fn load_lang() -> Lang {
    if let Some(root) = crate::paths::data_root() {
        let p = root.join("lang");
        if let Ok(s) = std::fs::read_to_string(p) {
            let s = s.trim().to_ascii_lowercase();
            if s.starts_with("ru") || s == "russian" || s == "русский" {
                return Lang::Ru;
            }
            if s.starts_with("en") || s == "english" {
                return Lang::En;
            }
        }
    }
    let loc = std::env::var("LANG")
        .or_else(|_| std::env::var("LC_ALL"))
        .unwrap_or_default()
        .to_ascii_lowercase();
    if loc.starts_with("ru") {
        Lang::Ru
    } else {
        Lang::En
    }
}

pub fn save_lang(lang: Lang) {
    if let Some(root) = crate::paths::data_root() {
        let tag = match lang {
            Lang::Ru => "ru\n",
            Lang::En => "en\n",
        };
        let _ = std::fs::write(root.join("lang"), tag);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalogs_load_main_menu_and_login() {
        let tables = tables();
        let n: usize = tables[0]
            .get("main_menu.__len")
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        assert_eq!(n, 18);
        assert!(tables[0]
            .get("main_menu.0")
            .is_some_and(|s| s.contains("Search")));
        assert_eq!(
            tables[0].get("login.apple").map(String::as_str),
            Some("Log in to Apple account")
        );
        assert_eq!(
            tables[1].get("login.apple").map(String::as_str),
            Some("Войти в аккаунт Apple")
        );
    }

    #[test]
    fn tr_replaces_placeholders() {
        set_lang(Lang::En);
        let s = tr("status.elapsed", &[("msg", "Hi"), ("secs", "3")]);
        assert!(s.contains("Hi"));
        assert!(s.contains("3"));
    }
}
