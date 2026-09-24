//! 表示言語（日本語 / English）と翻訳文字列。
//!
//! 文字列は `locales/<area>.toml` に 1 キー 1 行で ja / en を並べて置き、
//! `t!("area.key")` で引く。言語はプロセス全体で 1 つ（[`set_language`]）。
//! 見つからないキーは英語、それも無ければキー自身を返す（落とさない）。

use std::collections::HashMap;
use std::fmt::Display;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU8, Ordering};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Language {
    #[default]
    Ja,
    En,
}

impl Language {
    pub const ALL: [Language; 2] = [Language::Ja, Language::En];

    /// 設定画面の選択肢に出す名前。どの言語表示中でもその言語自身の名前で出す。
    pub fn native_name(self) -> &'static str {
        match self {
            Language::Ja => "日本語",
            Language::En => "English",
        }
    }
}

/// area 名と中身。area 名がキーの接頭辞になる。
const SOURCES: &[(&str, &str)] = &[
    ("app", include_str!("../locales/app.toml")),
    ("settings", include_str!("../locales/settings.toml")),
    ("tasks", include_str!("../locales/tasks.toml")),
    ("reviews", include_str!("../locales/reviews.toml")),
    ("notifications", include_str!("../locales/notifications.toml")),
    ("core", include_str!("../locales/core.toml")),
];

#[derive(Deserialize)]
struct Entry {
    ja: String,
    en: String,
}

struct Catalog {
    ja: HashMap<String, &'static str>,
    en: HashMap<String, &'static str>,
}

fn catalog() -> &'static Catalog {
    static CATALOG: OnceLock<Catalog> = OnceLock::new();
    CATALOG.get_or_init(|| {
        let mut ja = HashMap::new();
        let mut en = HashMap::new();
        for (area, source) in SOURCES {
            let entries: HashMap<String, Entry> = toml::from_str(source)
                .unwrap_or_else(|error| panic!("locales/{area}.toml is invalid: {error}"));
            for (key, entry) in entries {
                let key = format!("{area}.{key}");
                // 一度だけ作ってプロセス終了まで使うので 'static にしてよい。
                ja.insert(key.clone(), &*Box::leak(entry.ja.into_boxed_str()));
                en.insert(key, &*Box::leak(entry.en.into_boxed_str()));
            }
        }
        Catalog { ja, en }
    })
}

static CURRENT: AtomicU8 = AtomicU8::new(0);

pub fn set_language(language: Language) {
    CURRENT.store(
        match language {
            Language::Ja => 0,
            Language::En => 1,
        },
        Ordering::Relaxed,
    );
}

pub fn language() -> Language {
    match CURRENT.load(Ordering::Relaxed) {
        1 => Language::En,
        _ => Language::Ja,
    }
}

/// 現在の言語で `key` を引く。`t!("key")` から呼ばれる。
pub fn tr(key: &'static str) -> &'static str {
    let catalog = catalog();
    let table = match language() {
        Language::Ja => &catalog.ja,
        Language::En => &catalog.en,
    };
    table
        .get(key)
        .or_else(|| catalog.en.get(key))
        .copied()
        .unwrap_or(key)
}

/// `{name}` を引数で置き換える。`t!("key", name = value)` から呼ばれる。
pub fn tr_fmt(key: &'static str, args: &[(&str, &dyn Display)]) -> String {
    let mut text = tr(key).to_string();
    for (name, value) in args {
        text = text.replace(&format!("{{{name}}}"), &value.to_string());
    }
    text
}

/// 翻訳文字列を引く。
///
/// ```ignore
/// t!("tasks.title")                 // -> &'static str
/// t!("tasks.overdue", days = 3)     // -> String
/// ```
#[macro_export]
macro_rules! t {
    ($key:literal) => {
        $crate::tr($key)
    };
    ($key:literal, $($name:ident = $value:expr),+ $(,)?) => {
        $crate::tr_fmt($key, &[$((stringify!($name), &$value as &dyn ::std::fmt::Display)),+])
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn every_locale_file_parses() {
        let catalog = catalog();
        assert_eq!(catalog.ja.len(), catalog.en.len());
    }

    #[test]
    fn formats_named_arguments() {
        assert_eq!(
            tr_fmt("missing.{n}", &[("n", &3)]),
            "missing.3",
            "unknown keys fall back to the key itself"
        );
    }

    /// ワークスペース内の `t!("…")` が全て両言語に存在すること。
    #[test]
    fn every_used_key_exists() {
        let catalog = catalog();
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let mut missing = vec![];
        let mut stack = vec![root];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).unwrap().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else if path.extension().is_some_and(|e| e == "rs")
                    && !path.ends_with("i18n/src/lib.rs")
                {
                    let text = std::fs::read_to_string(&path).unwrap();
                    for key in used_keys(&text) {
                        if !catalog.ja.contains_key(key) || !catalog.en.contains_key(key) {
                            missing.push(format!("{}: {key}", path.display()));
                        }
                    }
                }
            }
        }
        assert!(missing.is_empty(), "missing translations:\n{}", missing.join("\n"));
    }

    /// `t!("key"` の key を集める。`format!("` 等の `…t!(` は除く。
    fn used_keys(text: &str) -> Vec<&str> {
        let mut keys = vec![];
        let mut rest = text;
        while let Some(pos) = rest.find("t!(\"") {
            let before = rest[..pos].chars().next_back();
            let tail = &rest[pos + 4..];
            if !before.is_some_and(|c| c.is_alphanumeric() || c == '_')
                && let Some(end) = tail.find('"')
            {
                keys.push(&tail[..end]);
            }
            rest = tail;
        }
        keys
    }
}
