use serde::Serialize;
use std::{process::Command, sync::OnceLock, time::Duration};

// Add a language here only when both its native and web UI translations ship.
const SUPPORTED: &[&str] = &["en"];

#[derive(Serialize)]
pub struct Locale {
    pub preferred: Vec<String>,
    pub language: &'static str,
}

fn select(preferred: &[String]) -> &'static str {
    preferred
        .iter()
        .find_map(|tag| {
            let base = tag.split(['-', '_']).next().unwrap_or("");
            SUPPORTED
                .iter()
                .copied()
                .find(|s| s.eq_ignore_ascii_case(base))
        })
        .unwrap_or("en")
}

pub fn current() -> &'static Locale {
    static LOCALE: OnceLock<Locale> = OnceLock::new();
    LOCALE.get_or_init(|| {
        // Finder-launched apps have no reliable LANG. Read macOS UI preferences.
        let preferred: Vec<String> = crate::router::run(
            Command::new("/usr/bin/osascript").args([
                "-l", "JavaScript", "-e",
                "ObjC.import('Foundation'); JSON.stringify(ObjC.deepUnwrap($.NSLocale.preferredLanguages))",
            ]),
            Duration::from_secs(3),
        )
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
        Locale { language: select(&preferred), preferred }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn system_language_and_english_fallback() {
        for tags in [vec![], vec!["ru-RU"], vec!["ja-JP", "EN_us"], vec!["en-GB"]] {
            let tags = tags.into_iter().map(String::from).collect::<Vec<_>>();
            assert_eq!(select(&tags), "en");
        }
    }
}
