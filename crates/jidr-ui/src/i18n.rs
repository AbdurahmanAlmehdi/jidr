//! The app's languages (issue #29, D33 follow-up).
//!
//! `.slint` strings go through `@tr` and Slint's bundled translations; the
//! strings the Rust side builds (statuses, toasts, rows) go through [`tr!`]
//! and [`trn!`], which read the same catalogs (`i18n/<lang>/LC_MESSAGES/
//! jidr-ui.po`, compiled in by `build.rs`). `cargo xtask i18n` extracts both
//! kinds and keeps the catalogs in step.
//!
//! The language is view state (gui-prefs `language`, never the protocol):
//! empty follows the Mac's preferred languages, otherwise it is a code from
//! [`Lang::code`]. It applies live: Slint re-translates its strings and the
//! app re-renders the Rust ones.

use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    En,
    Ar,
    Es,
    ZhHans,
    PtBr,
}

impl Lang {
    /// In Settings' order, after "System".
    pub const ALL: [Lang; 5] = [Lang::En, Lang::Ar, Lang::Es, Lang::ZhHans, Lang::PtBr];

    /// The catalog folder under `i18n/` (a BCP 47 tag), and the gui-prefs value.
    pub fn code(self) -> &'static str {
        match self {
            Lang::En => "en",
            Lang::Ar => "ar",
            Lang::Es => "es",
            Lang::ZhHans => "zh-Hans",
            Lang::PtBr => "pt-BR",
        }
    }

    /// The language's name in itself, as the Settings menu lists it.
    pub fn autonym(self) -> &'static str {
        match self {
            Lang::En => "English",
            Lang::Ar => "العربية",
            Lang::Es => "Español",
            Lang::ZhHans => "简体中文",
            Lang::PtBr => "Português (Brasil)",
        }
    }

    pub fn from_code(code: &str) -> Option<Lang> {
        Lang::ALL.into_iter().find(|l| l.code() == code)
    }

    pub fn rtl(self) -> bool {
        self == Lang::Ar
    }

    /// The catalog's `Plural-Forms` (Slint evaluates it for `@tr`; a test
    /// keeps [`Lang::plural`] in step with it).
    pub fn plural_forms(self) -> &'static str {
        match self {
            Lang::En | Lang::Es => "nplurals=2; plural=(n != 1);",
            Lang::PtBr => "nplurals=2; plural=(n > 1);",
            Lang::ZhHans => "nplurals=1; plural=0;",
            Lang::Ar => {
                "nplurals=6; plural=(n==0 ? 0 : n==1 ? 1 : n==2 ? 2 : n%100>=3 && n%100<=10 ? 3 : n%100>=11 ? 4 : 5);"
            }
        }
    }

    /// The plural form index for `n`.
    pub fn plural(self, n: i64) -> usize {
        match self {
            Lang::En | Lang::Es => usize::from(n != 1),
            Lang::PtBr => usize::from(n > 1),
            Lang::ZhHans => 0,
            Lang::Ar => match n {
                0 => 0,
                1 => 1,
                2 => 2,
                _ if (3..=10).contains(&(n % 100)) => 3,
                _ if n % 100 >= 11 => 4,
                _ => 5,
            },
        }
    }

    /// The first of `preferred` (BCP 47 tags, most preferred first, as macOS
    /// lists them) that Jidr has; English when none matches. Traditional
    /// Chinese isn't Simplified, so it falls through like any other language
    /// Jidr lacks.
    pub fn from_preferred<'a>(preferred: impl IntoIterator<Item = &'a str>) -> Lang {
        for tag in preferred {
            let tag = tag.replace('_', "-");
            let lower = tag.to_ascii_lowercase();
            let mut parts = lower.split('-');
            let found = match parts.next().unwrap_or_default() {
                "en" => Some(Lang::En),
                "ar" => Some(Lang::Ar),
                "es" => Some(Lang::Es),
                "pt" => Some(Lang::PtBr),
                "zh" => {
                    let rest: Vec<&str> = parts.collect();
                    let traditional = rest.contains(&"hant")
                        || (!rest.contains(&"hans")
                            && rest.iter().any(|p| matches!(*p, "tw" | "hk" | "mo")));
                    (!traditional).then_some(Lang::ZhHans)
                }
                _ => None,
            };
            if let Some(l) = found {
                return l;
            }
        }
        Lang::En
    }

    /// The gui-prefs value: empty (System) follows the Mac.
    pub fn resolve(pref: &str) -> Lang {
        Lang::from_code(pref).unwrap_or_else(|| {
            let system: Vec<String> = sys_locale::get_locales().collect();
            Lang::from_preferred(system.iter().map(String::as_str))
        })
    }
}

static CURRENT: AtomicU8 = AtomicU8::new(0);

/// Mirrors a window for the current language and switches its type to the
/// Arabic roles (each window has its own `Theme` global).
pub fn theme<C: slint::ComponentHandle>(c: &C)
where
    for<'a> crate::Theme<'a>: slint::Global<'a, C>,
{
    let lang = current();
    let t = c.global::<crate::Theme>();
    t.set_rtl(lang.rtl());
    t.set_arabic(lang == Lang::Ar);
}

/// The language [`tr!`] translates into.
pub fn current() -> Lang {
    Lang::ALL
        .get(usize::from(CURRENT.load(Ordering::Relaxed)))
        .copied()
        .unwrap_or(Lang::En)
}

/// Switches the Rust strings and Slint's `@tr` to `lang`. Slint needs a
/// component to exist first; before that only the Rust side switches, so
/// call it again after creating the window.
pub fn set(lang: Lang) {
    let idx = Lang::ALL.iter().position(|l| *l == lang).unwrap_or(0);
    CURRENT.store(u8::try_from(idx).unwrap_or(0), Ordering::Relaxed);
    // English is Slint's source language: "" selects it.
    let code = if lang == Lang::En { "" } else { lang.code() };
    if let Err(e) = slint::select_bundled_translation(code) {
        tracing::debug!("select_bundled_translation({code}): {e}");
    }
}

mod catalog {
    // Generated by build.rs: `PLURAL_FORMS: &[(&str, &str)]` and
    // `CATALOG: &[(&str, &[(&str, &[&str])])]`, entries sorted by msgid.
    include!(concat!(env!("OUT_DIR"), "/i18n_catalog.rs"));
}

fn lookup(lang: Lang, id: &str) -> Option<&'static [&'static str]> {
    let (_, entries) = catalog::CATALOG
        .iter()
        .find(|(code, _)| *code == lang.code())?;
    let i = entries.binary_search_by(|(k, _)| (*k).cmp(id)).ok()?;
    entries.get(i).map(|(_, v)| *v)
}

/// `id` in the current language (English when it has no translation).
pub fn translate(id: &str) -> String {
    lookup(current(), id)
        .and_then(|v| v.first())
        .filter(|s| !s.is_empty())
        .map_or_else(|| id.to_string(), |s| (*s).to_string())
}

/// The plural form of `one` / `other` for `n` in the current language.
pub fn translate_n(one: &str, other: &str, n: i64) -> String {
    let lang = current();
    lookup(lang, one)
        .and_then(|v| v.get(lang.plural(n)))
        .filter(|s| !s.is_empty())
        .map_or_else(
            || if n == 1 { one } else { other }.to_string(),
            |s| (*s).to_string(),
        )
}

/// Fills a translated template the way Slint's `@tr` does: `{}` takes the
/// next argument, `{0}` a given one, `{n}` the count; `{{` and `}}` are
/// literal braces.
pub fn format(template: &str, n: Option<i64>, args: &[&dyn std::fmt::Display]) -> String {
    let mut out = String::with_capacity(template.len() + 16);
    let mut next = 0;
    let mut rest = template;
    while let Some(i) = rest.find(['{', '}']) {
        out.push_str(&rest[..i]);
        let tail = &rest[i..];
        if tail.starts_with("{{") || tail.starts_with("}}") {
            out.push_str(&tail[..1]);
            rest = &tail[2..];
            continue;
        }
        let Some(end) = tail.find('}').filter(|_| tail.starts_with('{')) else {
            out.push_str(&tail[..1]);
            rest = &tail[1..];
            continue;
        };
        let name = &tail[1..end];
        let value = match name {
            "" => {
                next += 1;
                args.get(next - 1).map(|a| a.to_string())
            }
            "n" => n.map(|n| n.to_string()),
            i => i
                .parse::<usize>()
                .ok()
                .and_then(|i| args.get(i))
                .map(|a| a.to_string()),
        };
        match value {
            Some(v) => out.push_str(&v),
            None => out.push_str(&tail[..=end]),
        }
        rest = &tail[end + 1..];
    }
    out.push_str(rest);
    out
}

/// `s` as an AppleScript string literal (for `osascript` prompts).
pub fn applescript_string(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

/// A Rust-side UI string: `tr!("Hibernated {}", name)`. The literal is the
/// msgid `cargo xtask i18n` extracts, so it must be a literal.
macro_rules! tr {
    ($s:literal) => {
        $crate::i18n::translate($s)
    };
    ($s:literal, $($a:expr),+ $(,)?) => {
        $crate::i18n::format(&$crate::i18n::translate($s), None, &[$(&$a as &dyn std::fmt::Display),+])
    };
}

/// A plural: `trn!("{n} snapshot", "{n} snapshots", count)`; extra
/// arguments fill `{}` like [`tr!`].
macro_rules! trn {
    ($one:literal, $other:literal, $n:expr $(, $a:expr)* $(,)?) => {{
        let n = i64::try_from($n).unwrap_or(i64::MAX);
        $crate::i18n::format(&$crate::i18n::translate_n($one, $other, n), Some(n), &[$(&$a as &dyn std::fmt::Display),*])
    }};
}

pub(crate) use {tr, trn};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preferred_languages_pick_the_first_jidr_has() {
        assert_eq!(Lang::from_preferred(["fr-FR", "ar-LY", "en"]), Lang::Ar);
        assert_eq!(Lang::from_preferred(["en-GB", "es-ES"]), Lang::En);
        assert_eq!(Lang::from_preferred(["es-419"]), Lang::Es);
        assert_eq!(Lang::from_preferred(["pt-PT"]), Lang::PtBr);
        assert_eq!(Lang::from_preferred(["zh-Hans-CN"]), Lang::ZhHans);
        assert_eq!(Lang::from_preferred(["zh_CN"]), Lang::ZhHans);
        assert_eq!(Lang::from_preferred(["zh-Hant-TW", "ja"]), Lang::En);
        assert_eq!(Lang::from_preferred(["zh-TW", "es"]), Lang::Es);
        assert_eq!(Lang::from_preferred(["de", "fr"]), Lang::En);
        assert_eq!(Lang::from_preferred([]), Lang::En);
        assert_eq!(Lang::resolve("pt-BR"), Lang::PtBr);
    }

    #[test]
    fn arabic_has_six_plural_forms() {
        let forms: Vec<usize> = [0, 1, 2, 3, 10, 11, 99, 100, 102, 103, 111]
            .iter()
            .map(|n| Lang::Ar.plural(*n))
            .collect();
        assert_eq!(forms, [0, 1, 2, 3, 3, 4, 4, 5, 5, 3, 4]);
        assert_eq!(Lang::PtBr.plural(0), 0);
        assert_eq!(Lang::Es.plural(0), 1);
    }

    /// The catalogs declare the plural rules `Lang::plural` implements.
    #[test]
    fn catalog_plural_forms_match() {
        for (code, forms) in catalog::PLURAL_FORMS {
            let lang = Lang::from_code(code).unwrap();
            assert_eq!(forms.trim(), lang.plural_forms(), "{code}");
        }
        assert_eq!(catalog::PLURAL_FORMS.len(), Lang::ALL.len() - 1);
    }

    #[test]
    fn format_follows_slint() {
        assert_eq!(format("{} of {}", None, &[&"1", &"2"]), "1 of 2");
        assert_eq!(format("{1} before {0}", None, &[&"a", &"b"]), "b before a");
        assert_eq!(
            format("{n} devices on {}", Some(3), &[&"Mac"]),
            "3 devices on Mac"
        );
        assert_eq!(format("{{literal}} {}", None, &[&"x"]), "{literal} x");
        assert_eq!(format("missing {}", None, &[]), "missing {}");
    }

    #[test]
    fn english_falls_through_and_other_languages_translate() {
        set(Lang::En);
        assert_eq!(tr!("Stopped"), "Stopped");
        assert_eq!(trn!("{n} snapshot", "{n} snapshots", 3), "3 snapshots");
        set(Lang::Es);
        assert_ne!(tr!("Stopped"), "Stopped");
        set(Lang::En);
    }
}
