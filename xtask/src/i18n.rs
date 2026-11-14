//! `cargo xtask i18n`: the desktop app's translation catalogs (issue #29).
//!
//! Extracts every `@tr(…)` in `crates/jidr-ui/**/*.slint` and every
//! `tr!(…)` / `trn!(…)` in `crates/jidr-ui/src/**/*.rs`, writes the template
//! `crates/jidr-ui/i18n/jidr-ui.pot` and merges it into each
//! `i18n/<lang>/LC_MESSAGES/jidr-ui.po` (translations kept, new strings added
//! empty, strings no longer in the code dropped).
//!
//! `--check` (part of `cargo xtask ci`) changes nothing. It fails when the
//! catalogs are stale or when a translation is missing, fuzzy, drops or
//! invents a placeholder, loses a term that must stay as is (`adb`, `Magisk`,
//! …), or, in a right-to-left language, leaves a placeholder unisolated.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::workspace_root;

/// Slint's gettext domain is the crate name (`slint-build` uses `CARGO_PKG_NAME`).
const DOMAIN: &str = "jidr-ui";

/// Left untranslated wherever the English has them (case-sensitive).
const KEEP: &[&str] = &[
    "adb",
    "Magisk",
    "su -c id",
    "jidr ",
    "Claude",
    "GPX",
    "KML",
    "MCP",
    "APK",
    "API",
    "GPS",
    "SDK",
    "CLI",
    "JSON",
    "Play Integrity",
    "Google APIs",
    "OpenStreetMap",
    "Homebrew",
    "brew ",
    "logcat",
    "/sdcard",
];

/// Right-to-left languages: every placeholder sits inside FIRST STRONG
/// ISOLATE … POP DIRECTIONAL ISOLATE so a Latin device name, serial, path or
/// number keeps its own direction inside Arabic text (`<bdi>` in HTML).
const RTL: &[&str] = &["ar"];
const FSI: char = '\u{2068}';
const PDI: char = '\u{2069}';

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Key {
    ctx: String,
    id: String,
    plural: Option<String>,
}

#[derive(Debug, Clone)]
struct Msg {
    key: Key,
    refs: Vec<String>,
}

#[derive(Debug, Clone, Default)]
struct Translation {
    /// `#` translator comments, without the leading `# `.
    comments: Vec<String>,
    flags: Vec<String>,
    /// One string for a singular entry, `nplurals` strings for a plural one.
    strs: Vec<String>,
}

pub fn run(args: &[String]) -> Result<()> {
    let check = args.iter().any(|a| a == "--check");
    let root = workspace_root().join("crates/jidr-ui");
    let msgs = extract(&root)?;
    let dir = root.join("i18n");
    let pot = render_pot(&msgs);
    let mut stale = Vec::new();
    write_or_compare(&dir.join(format!("{DOMAIN}.pot")), &pot, check, &mut stale)?;

    let mut problems = Vec::new();
    for lang in languages(&dir)? {
        let path = dir
            .join(&lang)
            .join("LC_MESSAGES")
            .join(format!("{DOMAIN}.po"));
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        let (header, old) = parse_po(&text).with_context(|| path.display().to_string())?;
        let nplurals = nplurals(&header)
            .with_context(|| format!("{}: no `Plural-Forms: nplurals=…`", path.display()))?;
        let merged = render_po(&header, &msgs, &old, nplurals);
        write_or_compare(&path, &merged, check, &mut stale)?;
        for m in &msgs {
            let t = old.get(&m.key).cloned().unwrap_or_default();
            lint(&lang, m, &t, nplurals, &mut problems);
        }
    }
    if check {
        for p in &problems {
            println!("xtask: i18n: {p}");
        }
    }
    if !stale.is_empty() {
        bail!(
            "translation catalogs are out of date ({}); run `cargo xtask i18n`",
            stale.join(", ")
        );
    }
    if check && !problems.is_empty() {
        bail!("{} translation problem(s)", problems.len());
    }
    let missing = problems.len();
    println!(
        "xtask: i18n {}: {} strings{}",
        if check { "ok" } else { "updated" },
        msgs.len(),
        if check || missing == 0 {
            String::new()
        } else {
            format!(", {missing} translation problem(s); see `cargo xtask i18n --check`")
        }
    );
    Ok(())
}

fn write_or_compare(path: &Path, text: &str, check: bool, stale: &mut Vec<String>) -> Result<()> {
    let current = std::fs::read_to_string(path).unwrap_or_default();
    if current == text {
        return Ok(());
    }
    if check {
        stale.push(
            path.strip_prefix(workspace_root())
                .unwrap_or(path)
                .display()
                .to_string(),
        );
        return Ok(());
    }
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    std::fs::write(path, text).with_context(|| format!("writing {}", path.display()))
}

/// The language folders under `i18n/`, sorted.
fn languages(dir: &Path) -> Result<Vec<String>> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))? {
        let e = e?;
        if e.path()
            .join("LC_MESSAGES")
            .join(format!("{DOMAIN}.po"))
            .exists()
        {
            out.push(e.file_name().to_string_lossy().into_owned());
        }
    }
    out.sort();
    Ok(out)
}

// ---------------------------------------------------------------- extraction

/// Every message, in source order (files sorted by path), merged by key.
fn extract(root: &Path) -> Result<Vec<Msg>> {
    let mut files = Vec::new();
    for sub in ["ui", "src"] {
        walk(&root.join(sub), &mut files)?;
    }
    files.sort();
    let mut order: Vec<Key> = Vec::new();
    let mut by_key: BTreeMap<Key, Vec<String>> = BTreeMap::new();
    for f in files {
        let rel = f
            .strip_prefix(root)
            .unwrap_or(&f)
            .to_string_lossy()
            .replace('\\', "/");
        let text = std::fs::read_to_string(&f)?;
        let found = if rel.ends_with(".slint") {
            scan_slint(&text)
        } else {
            scan_rust(&text)
        }
        .with_context(|| rel.clone())?;
        for key in found {
            let refs = by_key.entry(key.clone()).or_insert_with(|| {
                order.push(key.clone());
                Vec::new()
            });
            if !refs.contains(&rel) {
                refs.push(rel.clone());
            }
        }
    }
    Ok(order
        .into_iter()
        .map(|key| {
            let refs = by_key.remove(&key).unwrap_or_default();
            Msg { key, refs }
        })
        .collect())
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    for e in std::fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))? {
        let p = e?.path();
        if p.is_dir() {
            walk(&p, out)?;
        } else if p.extension().is_some_and(|x| x == "slint" || x == "rs") {
            out.push(p);
        }
    }
    Ok(())
}

struct Cursor<'a> {
    s: &'a [u8],
    i: usize,
}

impl Cursor<'_> {
    fn skip_ws(&mut self) {
        loop {
            while self.i < self.s.len() && self.s[self.i].is_ascii_whitespace() {
                self.i += 1;
            }
            if self.s[self.i..].starts_with(b"//") {
                while self.i < self.s.len() && self.s[self.i] != b'\n' {
                    self.i += 1;
                }
            } else {
                return;
            }
        }
    }

    fn eat(&mut self, tok: &str) -> bool {
        self.skip_ws();
        if self.s[self.i..].starts_with(tok.as_bytes()) {
            self.i += tok.len();
            true
        } else {
            false
        }
    }

    /// A `"…"` literal with Slint/Rust escapes, unescaped. Adjacent Rust
    /// literals aren't concatenated (neither language does that).
    fn string(&mut self) -> Result<String> {
        self.skip_ws();
        if self.s.get(self.i) != Some(&b'"') {
            bail!("expected a string literal at byte {}", self.i);
        }
        self.i += 1;
        let mut out = Vec::new();
        while let Some(&c) = self.s.get(self.i) {
            self.i += 1;
            match c {
                b'"' => return String::from_utf8(out).context("string literal isn't UTF-8"),
                b'\\' => {
                    let Some(&e) = self.s.get(self.i) else { break };
                    self.i += 1;
                    match e {
                        b'n' => out.push(b'\n'),
                        b't' => out.push(b'\t'),
                        b'"' | b'\\' | b'{' | b'\'' => out.push(e),
                        b'\n' => {
                            // Rust line continuation: skip the next line's indentation.
                            while self.s.get(self.i).is_some_and(u8::is_ascii_whitespace) {
                                self.i += 1;
                            }
                        }
                        b'u' => {
                            let rest = &self.s[self.i..];
                            let end = rest
                                .iter()
                                .position(|&b| b == b'}')
                                .context("bad \\u escape")?;
                            let hex = std::str::from_utf8(&rest[1..end])?;
                            let ch = u32::from_str_radix(hex, 16)
                                .ok()
                                .and_then(char::from_u32)
                                .context("bad \\u escape")?;
                            let mut b = [0; 4];
                            out.extend_from_slice(ch.encode_utf8(&mut b).as_bytes());
                            self.i += end + 1;
                        }
                        other => bail!("unsupported escape \\{}", other as char),
                    }
                }
                _ => out.push(c),
            }
        }
        bail!("unterminated string literal")
    }
}

/// `@tr(["ctx" =>] "text" [| "plural" % n] [, args…])`.
fn scan_slint(text: &str) -> Result<Vec<Key>> {
    let mut out = Vec::new();
    for (pos, _) in text.match_indices("@tr(") {
        let line_start = text[..pos].rfind('\n').map_or(0, |i| i + 1);
        if text[line_start..pos].contains("//") {
            continue;
        }
        let mut c = Cursor {
            s: text.as_bytes(),
            i: pos + 4,
        };
        let first = c.string()?;
        let (ctx, id) = if c.eat("=>") {
            (first, c.string()?)
        } else {
            (String::new(), first)
        };
        let plural = if c.eat("|") { Some(c.string()?) } else { None };
        out.push(Key { ctx, id, plural });
    }
    Ok(out)
}

/// `tr!("text"[, args…])` and `trn!("one", "other", n[, args…])`.
fn scan_rust(text: &str) -> Result<Vec<Key>> {
    let mut out = Vec::new();
    let bytes = text.as_bytes();
    for (pos, m) in text
        .match_indices("tr!(")
        .chain(text.match_indices("trn!("))
    {
        // `tr!` must be a whole identifier (not `str!`, `attr!`).
        if pos > 0 && (bytes[pos - 1].is_ascii_alphanumeric() || bytes[pos - 1] == b'_') {
            continue;
        }
        let line_start = text[..pos].rfind('\n').map_or(0, |i| i + 1);
        if text[line_start..pos].contains("//") {
            continue;
        }
        let mut c = Cursor {
            s: bytes,
            i: pos + m.len(),
        };
        // `macro_rules!` definitions and doc examples use `$…`, not a literal.
        c.skip_ws();
        if c.s.get(c.i) != Some(&b'"') {
            continue;
        }
        let id = c.string()?;
        let plural = if m == "trn!(" {
            if !c.eat(",") {
                bail!("trn! needs a plural string");
            }
            Some(c.string()?)
        } else {
            None
        };
        out.push(Key {
            ctx: String::new(),
            id,
            plural,
        });
    }
    Ok(out)
}

// ------------------------------------------------------------------- PO files

fn escape(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => o.push_str("\\\\"),
            '"' => o.push_str("\\\""),
            '\n' => o.push_str("\\n"),
            '\t' => o.push_str("\\t"),
            c => o.push(c),
        }
    }
    o
}

/// `keyword "…"`, split after each `\n` like gettext does.
fn field(out: &mut String, keyword: &str, value: &str) {
    if value.contains('\n') && value.trim_end_matches('\n').contains('\n') {
        let _ = writeln!(out, "{keyword} \"\"");
        for part in value.split_inclusive('\n') {
            let _ = writeln!(out, "\"{}\"", escape(part));
        }
    } else {
        let _ = writeln!(out, "{keyword} \"{}\"", escape(value));
    }
}

fn entry(out: &mut String, m: &Msg, t: Option<&Translation>, nplurals: usize) {
    out.push('\n');
    if let Some(t) = t {
        for c in &t.comments {
            let _ = writeln!(out, "# {c}");
        }
    }
    let _ = writeln!(out, "#: {}", m.refs.join(" "));
    if let Some(t) = t.filter(|t| !t.flags.is_empty()) {
        let _ = writeln!(out, "#, {}", t.flags.join(", "));
    }
    if !m.key.ctx.is_empty() {
        field(out, "msgctxt", &m.key.ctx);
    }
    field(out, "msgid", &m.key.id);
    match &m.key.plural {
        Some(p) => {
            field(out, "msgid_plural", p);
            for i in 0..nplurals {
                let s = t.and_then(|t| t.strs.get(i)).map_or("", String::as_str);
                field(out, &format!("msgstr[{i}]"), s);
            }
        }
        None => {
            let s = t.and_then(|t| t.strs.first()).map_or("", String::as_str);
            field(out, "msgstr", s);
        }
    }
}

fn render_pot(msgs: &[Msg]) -> String {
    let mut s = String::from(
        "# The Jidr desktop app's strings. Generated by `cargo xtask i18n`; do not edit.\n\
         msgid \"\"\n\
         msgstr \"\"\n\
         \"Project-Id-Version: jidr-ui\\n\"\n\
         \"MIME-Version: 1.0\\n\"\n\
         \"Content-Type: text/plain; charset=UTF-8\\n\"\n\
         \"Content-Transfer-Encoding: 8bit\\n\"\n\
         \"Plural-Forms: nplurals=2; plural=(n != 1);\\n\"\n",
    );
    for m in msgs {
        entry(&mut s, m, None, 2);
    }
    s
}

fn render_po(
    header: &str,
    msgs: &[Msg],
    old: &BTreeMap<Key, Translation>,
    nplurals: usize,
) -> String {
    let mut s = header.trim_end().to_string();
    s.push('\n');
    for m in msgs {
        entry(&mut s, m, old.get(&m.key), nplurals);
    }
    s
}

fn nplurals(header: &str) -> Option<usize> {
    let rest = &header[header.find("nplurals=")? + "nplurals=".len()..];
    rest.split(|c: char| !c.is_ascii_digit())
        .next()?
        .parse()
        .ok()
}

fn unquote(line: &str) -> Result<String> {
    let inner = line
        .trim()
        .strip_prefix('"')
        .and_then(|l| l.strip_suffix('"'))
        .with_context(|| format!("expected a quoted string: {line}"))?;
    let mut o = String::new();
    let mut it = inner.chars();
    while let Some(c) = it.next() {
        if c != '\\' {
            o.push(c);
            continue;
        }
        match it.next() {
            Some('n') => o.push('\n'),
            Some('t') => o.push('\t'),
            Some('"') => o.push('"'),
            Some('\\') => o.push('\\'),
            other => bail!("unsupported PO escape \\{other:?}"),
        }
    }
    Ok(o)
}

/// The header block (comments + the `msgid ""` entry, verbatim) and the
/// translations by key. Handles what `render_po` writes plus `#~` lines.
fn parse_po(text: &str) -> Result<(String, BTreeMap<Key, Translation>)> {
    let mut blocks = text.split("\n\n");
    let header = blocks.next().unwrap_or_default().to_string();
    let mut map = BTreeMap::new();
    for block in blocks {
        let mut t = Translation::default();
        let mut key = Key {
            ctx: String::new(),
            id: String::new(),
            plural: None,
        };
        let mut strs: BTreeMap<usize, String> = BTreeMap::new();
        // Which field a continuation line extends.
        enum F {
            None,
            Ctx,
            Id,
            Plural,
            Str(usize),
        }
        let mut cur = F::None;
        let mut any = false;
        for line in block.lines() {
            let line = line.trim_end();
            if line.is_empty() || line.starts_with("#~") || line.starts_with("#:") {
                continue;
            }
            if let Some(f) = line.strip_prefix("#,") {
                t.flags = f.split(',').map(|x| x.trim().to_string()).collect();
                continue;
            }
            if let Some(c) = line.strip_prefix('#') {
                t.comments
                    .push(c.strip_prefix(' ').unwrap_or(c).to_string());
                continue;
            }
            if line.starts_with('"') {
                let more = unquote(line)?;
                match cur {
                    F::Ctx => key.ctx.push_str(&more),
                    F::Id => key.id.push_str(&more),
                    F::Plural => {
                        if let Some(p) = key.plural.as_mut() {
                            p.push_str(&more);
                        }
                    }
                    F::Str(i) => strs.entry(i).or_default().push_str(&more),
                    F::None => bail!("stray string line: {line}"),
                }
                continue;
            }
            let (kw, rest) = line
                .split_once(' ')
                .with_context(|| format!("bad PO line: {line}"))?;
            let v = unquote(rest)?;
            any = true;
            cur = match kw {
                "msgctxt" => {
                    key.ctx = v;
                    F::Ctx
                }
                "msgid" => {
                    key.id = v;
                    F::Id
                }
                "msgid_plural" => {
                    key.plural = Some(v);
                    F::Plural
                }
                "msgstr" => {
                    strs.insert(0, v);
                    F::Str(0)
                }
                k => {
                    let i = k
                        .strip_prefix("msgstr[")
                        .and_then(|k| k.strip_suffix(']'))
                        .and_then(|n| n.parse().ok())
                        .with_context(|| format!("bad PO keyword: {k}"))?;
                    strs.insert(i, v);
                    F::Str(i)
                }
            };
        }
        if any {
            t.strs = strs.into_values().collect();
            map.insert(key, t);
        }
    }
    Ok((header, map))
}

// ---------------------------------------------------------------------- lint

/// `{}`, `{0}`, `{n}` … in order of appearance (`{{` / `}}` are escapes).
fn placeholders(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut it = s.char_indices().peekable();
    while let Some((i, c)) = it.next() {
        if c == '{' {
            if it.peek().is_some_and(|&(_, n)| n == '{') {
                it.next();
                continue;
            }
            if let Some(end) = s[i..].find('}') {
                out.push(s[i..i + end + 1].to_string());
            }
        }
    }
    out
}

fn sorted(mut v: Vec<String>) -> Vec<String> {
    v.sort();
    v
}

fn lint(lang: &str, m: &Msg, t: &Translation, nplurals: usize, problems: &mut Vec<String>) {
    let label = format!("{lang}: {:?}", m.key.id);
    if t.flags.iter().any(|f| f == "fuzzy") {
        problems.push(format!("{label} is marked fuzzy"));
    }
    let want = if m.key.plural.is_some() { nplurals } else { 1 };
    if t.strs.len() < want || t.strs.iter().take(want).any(String::is_empty) {
        problems.push(format!("{label} isn't translated"));
        return;
    }
    let english = m.key.plural.as_deref().unwrap_or(&m.key.id);
    let expected = sorted(placeholders(english));
    for s in t.strs.iter().take(want) {
        let mut got = sorted(placeholders(s));
        // A plural form may spell the count out ("جهازان", "un solo…").
        if m.key.plural.is_some() && !got.iter().any(|p| p == "{n}") {
            got.push("{n}".into());
            got.sort();
        }
        let mut exp = expected.clone();
        if m.key.plural.is_some() && !exp.iter().any(|p| p == "{n}") {
            exp.push("{n}".into());
            exp.sort();
        }
        if got != exp {
            problems.push(format!(
                "{label}: placeholders {got:?} don't match the English {exp:?} in {s:?}"
            ));
        }
        for k in KEEP {
            if m.key.id.contains(k) && !s.contains(k) {
                problems.push(format!("{label}: keep {k:?} as is in {s:?}"));
            }
        }
        if RTL.contains(&lang) {
            for (i, _) in s.match_indices('{') {
                let before = s[..i].chars().next_back();
                if before == Some('{') || s[i + 1..].starts_with('{') {
                    continue;
                }
                let after = s[i..].find('}').and_then(|e| s[i + e + 1..].chars().next());
                if before != Some(FSI) || after != Some(PDI) {
                    problems.push(format!(
                        "{label}: wrap each placeholder in U+2068 … U+2069 in {s:?}"
                    ));
                    break;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slint_forms() {
        let k = scan_slint(
            r#"text: @tr("Edit {}", name); t: @tr("ctx" => "Root"); // @tr("skip")
            p: @tr("{n} device" | "{n} devices" % count); q: @tr("say \"hi\"\n");"#,
        )
        .unwrap();
        assert_eq!(k.len(), 4);
        assert_eq!(k[0].id, "Edit {}");
        assert_eq!(k[1].ctx, "ctx");
        assert_eq!(k[2].plural.as_deref(), Some("{n} devices"));
        assert_eq!(k[3].id, "say \"hi\"\n");
    }

    #[test]
    fn rust_forms() {
        let k = scan_rust(
            r#"let a = tr!("Hibernated {}", name); let b = trn!("{n} snapshot", "{n} snapshots", n);
            macro_rules! tr { ($s:literal) => {} } let c = str!("no"); // tr!("comment")"#,
        )
        .unwrap();
        assert_eq!(k.len(), 2);
        assert_eq!(k[0].id, "Hibernated {}");
        assert_eq!(k[1].plural.as_deref(), Some("{n} snapshots"));
    }

    #[test]
    fn po_round_trip() {
        let msgs = vec![
            Msg {
                key: Key {
                    ctx: String::new(),
                    id: "Two\nlines \"q\"\n".into(),
                    plural: None,
                },
                refs: vec!["ui/a.slint".into()],
            },
            Msg {
                key: Key {
                    ctx: "c".into(),
                    id: "{n} device".into(),
                    plural: Some("{n} devices".into()),
                },
                refs: vec!["src/b.rs".into()],
            },
        ];
        let mut old = BTreeMap::new();
        old.insert(
            msgs[1].key.clone(),
            Translation {
                comments: vec!["unsure".into()],
                flags: vec![],
                strs: vec!["a".into(), "b".into(), "c".into()],
            },
        );
        let header = "msgid \"\"\nmsgstr \"\"\n\"Plural-Forms: nplurals=3; plural=0;\\n\"";
        let text = render_po(header, &msgs, &old, 3);
        let (h, back) = parse_po(&text).unwrap();
        assert_eq!(nplurals(&h), Some(3));
        assert_eq!(back[&msgs[1].key].strs, vec!["a", "b", "c"]);
        assert_eq!(back[&msgs[1].key].comments, vec!["unsure"]);
        assert!(back.contains_key(&msgs[0].key));
        assert_eq!(render_po(&h, &msgs, &back, 3), text);
    }

    #[test]
    fn lint_catches_placeholders_terms_and_isolation() {
        let m = Msg {
            key: Key {
                ctx: String::new(),
                id: "Run {} with adb".into(),
                plural: None,
            },
            refs: vec![],
        };
        let t = |s: &str| Translation {
            strs: vec![s.into()],
            ..Default::default()
        };
        let mut p = Vec::new();
        lint("es", &m, &t("Ejecuta {} con adb"), 2, &mut p);
        assert!(p.is_empty(), "{p:?}");
        lint("es", &m, &t("Ejecuta con adb"), 2, &mut p);
        lint("es", &m, &t("Ejecuta {} con ADB"), 2, &mut p);
        lint("ar", &m, &t("شغّل {} عبر adb"), 6, &mut p);
        assert_eq!(p.len(), 3, "{p:?}");
        p.clear();
        lint("ar", &m, &t("شغّل \u{2068}{}\u{2069} عبر adb"), 6, &mut p);
        assert!(p.is_empty(), "{p:?}");
    }
}
