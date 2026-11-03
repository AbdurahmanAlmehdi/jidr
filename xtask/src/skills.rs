//! Agent skills check (D43).
//!
//! Every `skills/<name>/SKILL.md` starts with YAML frontmatter: a `name` equal
//! to its folder and a `description` (at most 1024 characters) saying when to
//! use the skill. Every `jidr …` command in a fenced shell block must parse
//! against the real CLI, and every `jidr call <method>` must name a command in
//! the protocol reference, so a renamed subcommand, flag or method fails CI.

use std::collections::BTreeSet;

use anyhow::{Context, Result, bail};
use clap::Parser;
use clap::error::ErrorKind;

use crate::workspace_root;

/// The CLI's clap definition, compiled in as-is. Only `Cli::try_parse_from` is
/// used here; the parsed fields are read by the `jidr` binary itself.
#[allow(dead_code)]
#[path = "../../apps/jidr/src/cli.rs"]
mod cli;

const MAX_DESCRIPTION: usize = 1024;
const MAX_NAME: usize = 64;
const SHELL_FENCES: &[&str] = &["bash", "sh", "shell", "zsh"];

pub fn run() -> Result<()> {
    let root = workspace_root();
    let methods = protocol_methods(
        &std::fs::read_to_string(root.join("docs/control-protocol.md"))
            .context("reading docs/control-protocol.md")?,
    );
    let dir = root.join("skills");
    let mut entries: Vec<_> = std::fs::read_dir(&dir)
        .with_context(|| format!("reading {}", dir.display()))?
        .collect::<std::io::Result<_>>()?;
    entries.sort_by_key(|e| e.file_name());

    let mut problems = Vec::new();
    let mut checked = 0;
    for entry in entries {
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let folder = entry.file_name().to_string_lossy().into_owned();
        let path = entry.path().join("SKILL.md");
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) => {
                problems.push(format!("skills/{folder}/SKILL.md: {e}"));
                continue;
            }
        };
        problems.extend(
            check_skill(&folder, &text, &methods)
                .into_iter()
                .map(|p| format!("skills/{folder}/SKILL.md: {p}")),
        );
        checked += 1;
    }
    if checked == 0 {
        bail!("no skills found in {}", dir.display());
    }
    if !problems.is_empty() {
        for p in &problems {
            println!("xtask: {p}");
        }
        bail!("{} skills problem(s)", problems.len());
    }
    println!("xtask: skills-check ok ({checked} skills)");
    Ok(())
}

/// Every problem in one SKILL.md, with line numbers where they apply.
pub fn check_skill(folder: &str, text: &str, methods: &BTreeSet<String>) -> Vec<String> {
    let mut problems = Vec::new();
    match frontmatter(text) {
        Err(e) => problems.push(e),
        Ok(fm) => {
            match fm.name {
                None => problems.push("frontmatter has no `name`".into()),
                Some(name) if name != folder => {
                    problems.push(format!("name `{name}` must match the folder `{folder}`"))
                }
                Some(name) if !valid_name(&name) => problems.push(format!(
                    "name `{name}` must be 1–{MAX_NAME} lowercase letters, digits and single hyphens"
                )),
                Some(_) => {}
            }
            match fm.description {
                None => problems.push("frontmatter has no `description`".into()),
                Some(d) if d.trim().is_empty() => problems.push("description is empty".into()),
                Some(d) if d.chars().count() > MAX_DESCRIPTION => problems.push(format!(
                    "description is {} characters (max {MAX_DESCRIPTION})",
                    d.chars().count()
                )),
                Some(_) => {}
            }
        }
    }
    for (line, cmd) in jidr_commands(text) {
        if let Err(e) = check_command(&cmd, methods) {
            problems.push(format!("line {line}: `{}`: {e}", cmd.join(" ")));
        }
    }
    problems
}

#[derive(Debug, Default)]
struct Frontmatter {
    name: Option<String>,
    description: Option<String>,
}

/// The `name` and `description` of a `---` delimited frontmatter block. Only
/// single-line `key: value` entries are read (plain, single- or double-quoted).
fn frontmatter(text: &str) -> std::result::Result<Frontmatter, String> {
    let mut lines = text.lines();
    if lines.next().map(str::trim_end) != Some("---") {
        return Err("must start with `---` YAML frontmatter".into());
    }
    let mut fm = Frontmatter::default();
    for line in lines {
        if line.trim_end() == "---" {
            return Ok(fm);
        }
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let value = unquote(value.trim());
        match key.trim() {
            "name" => fm.name = Some(value),
            "description" => {
                if value == ">" || value == "|" || value.starts_with(['>', '|']) {
                    return Err("write `description` on one line".into());
                }
                fm.description = Some(value);
            }
            _ => {}
        }
    }
    Err("frontmatter has no closing `---`".into())
}

fn unquote(v: &str) -> String {
    if v.len() >= 2 && v.starts_with('"') && v.ends_with('"') {
        v[1..v.len() - 1]
            .replace("\\\"", "\"")
            .replace("\\\\", "\\")
    } else if v.len() >= 2 && v.starts_with('\'') && v.ends_with('\'') {
        v[1..v.len() - 1].replace("''", "'")
    } else {
        v.to_string()
    }
}

/// The skills spec's name rule: lowercase alphanumerics and hyphens, no
/// leading, trailing or double hyphen.
fn valid_name(name: &str) -> bool {
    (1..=MAX_NAME).contains(&name.len())
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && !name.starts_with('-')
        && !name.ends_with('-')
        && !name.contains("--")
}

/// Every command line in a fenced shell block that runs `jidr`, as words
/// (without the `jidr`), with its 1-based line number. Shell operators split a
/// line into commands; comments and redirections are dropped.
fn jidr_commands(text: &str) -> Vec<(usize, Vec<String>)> {
    let mut out = Vec::new();
    let mut in_shell = false;
    let mut in_fence = false;
    let mut pending: Option<(usize, String)> = None;
    for (i, line) in text.lines().enumerate() {
        let trimmed = line.trim_start();
        if let Some(lang) = trimmed.strip_prefix("```") {
            if in_fence {
                in_fence = false;
                in_shell = false;
            } else {
                in_fence = true;
                in_shell = SHELL_FENCES.contains(&lang.trim());
            }
            continue;
        }
        if !in_shell {
            continue;
        }
        // Join `\` continuations onto the line they started on.
        let (start, mut joined) = pending.take().unwrap_or((i + 1, String::new()));
        if let Some(head) = line.trim_end().strip_suffix('\\') {
            joined.push_str(head);
            joined.push(' ');
            pending = Some((start, joined));
            continue;
        }
        joined.push_str(line);
        for cmd in commands(&joined) {
            if cmd.first().map(String::as_str) == Some("jidr") {
                out.push((start, cmd[1..].to_vec()));
            }
        }
    }
    out
}

/// Splits a shell line into simple commands of unquoted words. Handles single
/// and double quotes, backslash escapes, `#` comments, `|`, `||`, `&&`, `;`,
/// `&`, `$(`/`)` and drops redirections with their targets. Leading
/// `VAR=value` assignments are skipped.
fn commands(line: &str) -> Vec<Vec<String>> {
    let mut cmds = Vec::new();
    let mut words: Vec<String> = Vec::new();
    let mut word = String::new();
    let mut in_word = false;
    let mut skip_next = false;
    let mut chars = line.chars().peekable();

    while let Some(c) = chars.next() {
        match c {
            '\'' => {
                in_word = true;
                for q in chars.by_ref() {
                    if q == '\'' {
                        break;
                    }
                    word.push(q);
                }
            }
            '"' => {
                in_word = true;
                while let Some(q) = chars.next() {
                    match q {
                        '"' => break,
                        '\\' => {
                            if let Some(n) = chars.next() {
                                word.push(n);
                            }
                        }
                        _ => word.push(q),
                    }
                }
            }
            '\\' => {
                in_word = true;
                if let Some(n) = chars.next() {
                    word.push(n);
                }
            }
            '#' if !in_word => break,
            ' ' | '\t' => end_word(&mut word, &mut in_word, &mut words, &mut skip_next),
            '|' | '&' | ';' | '(' | ')' => {
                end_word(&mut word, &mut in_word, &mut words, &mut skip_next);
                if !words.is_empty() {
                    cmds.push(std::mem::take(&mut words));
                }
            }
            '$' if chars.peek() == Some(&'(') => {
                end_word(&mut word, &mut in_word, &mut words, &mut skip_next);
                if !words.is_empty() {
                    cmds.push(std::mem::take(&mut words));
                }
            }
            '>' | '<' => {
                // `2>`, `>>`, `2>&1`: the digit before belongs to the redirection.
                if word.chars().all(|d| d.is_ascii_digit()) {
                    word.clear();
                    in_word = false;
                } else {
                    end_word(&mut word, &mut in_word, &mut words, &mut skip_next);
                }
                while matches!(chars.peek(), Some('>' | '&')) {
                    chars.next();
                }
                if chars.peek().is_some_and(|n| n.is_ascii_digit()) {
                    chars.next();
                } else {
                    skip_next = true;
                }
            }
            _ => {
                in_word = true;
                word.push(c);
            }
        }
    }
    end_word(&mut word, &mut in_word, &mut words, &mut skip_next);
    if !words.is_empty() {
        cmds.push(words);
    }
    cmds
}

/// Ends the current word: pushes it, unless it's a redirection target or a
/// leading `VAR=value` assignment.
fn end_word(word: &mut String, in_word: &mut bool, words: &mut Vec<String>, skip: &mut bool) {
    if *in_word {
        if *skip {
            *skip = false;
        } else if !(words.is_empty() && is_assignment(word)) {
            words.push(word.clone());
        }
        word.clear();
        *in_word = false;
    }
}

fn is_assignment(word: &str) -> bool {
    word.split_once('=').is_some_and(|(k, _)| {
        !k.is_empty()
            && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            && !k.starts_with(|c: char| c.is_ascii_digit())
    })
}

/// Parses `jidr <args>` with the real clap definition; `jidr call <method>`
/// must also name a protocol command.
fn check_command(args: &[String], methods: &BTreeSet<String>) -> std::result::Result<(), String> {
    let argv = std::iter::once("jidr").chain(args.iter().map(String::as_str));
    match cli::Cli::try_parse_from(argv) {
        Ok(parsed) => match parsed.cmd {
            cli::Cmd::Call { method, .. } if !methods.contains(&method) => {
                Err(format!("`{method}` is not a protocol command"))
            }
            _ => Ok(()),
        },
        Err(e) if matches!(e.kind(), ErrorKind::DisplayHelp | ErrorKind::DisplayVersion) => Ok(()),
        Err(e) => Err(e
            .render()
            .to_string()
            .lines()
            .next()
            .unwrap_or("does not parse")
            .trim_start_matches("error: ")
            .to_string()),
    }
}

/// Method names from the `#### \`method\`` headings of the protocol reference.
fn protocol_methods(doc: &str) -> BTreeSet<String> {
    doc.lines()
        .filter_map(|l| l.strip_prefix("#### `")?.strip_suffix('`'))
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn methods() -> BTreeSet<String> {
        ["device.list", "media.screenshot"]
            .into_iter()
            .map(String::from)
            .collect()
    }

    fn skill(frontmatter: &str, body: &str) -> String {
        format!("---\n{frontmatter}\n---\n\n# Title\n\n{body}")
    }

    const OK_FM: &str = "name: jidr\ndescription: Use when the user wants an Android emulator.";

    #[test]
    fn a_valid_skill_passes() {
        let body = "```bash\njidr device list\njidr --json call media.screenshot '{\"device\":\"Phone\"}'\n```\n";
        assert_eq!(
            check_skill("jidr", &skill(OK_FM, body), &methods()),
            Vec::<String>::new()
        );
    }

    #[test]
    fn name_must_match_the_folder() {
        let p = check_skill(
            "jidr",
            &skill("name: jidr-basics\ndescription: x", ""),
            &methods(),
        );
        assert_eq!(p, ["name `jidr-basics` must match the folder `jidr`"]);
    }

    #[test]
    fn name_must_follow_the_spec() {
        let p = check_skill(
            "Jidr_X",
            &skill("name: Jidr_X\ndescription: x", ""),
            &methods(),
        );
        assert!(p[0].contains("lowercase letters"), "{p:?}");
    }

    #[test]
    fn description_is_required() {
        let p = check_skill("jidr", &skill("name: jidr", ""), &methods());
        assert_eq!(p, ["frontmatter has no `description`"]);
    }

    #[test]
    fn description_is_at_most_1024_chars() {
        let fm = format!("name: jidr\ndescription: {}", "a".repeat(1025));
        let p = check_skill("jidr", &skill(&fm, ""), &methods());
        assert_eq!(p, ["description is 1025 characters (max 1024)"]);
    }

    #[test]
    fn frontmatter_is_required() {
        let p = check_skill("jidr", "# Jidr\n", &methods());
        assert_eq!(p, ["must start with `---` YAML frontmatter"]);
    }

    #[test]
    fn quoted_values_are_unquoted() {
        let fm = frontmatter("---\nname: \"jidr\"\ndescription: 'it''s fine'\n---\n").unwrap();
        assert_eq!(fm.name.as_deref(), Some("jidr"));
        assert_eq!(fm.description.as_deref(), Some("it's fine"));
    }

    #[test]
    fn an_unknown_subcommand_fails() {
        let body = "```bash\njidr device lst\n```\n";
        let p = check_skill("jidr", &skill(OK_FM, body), &methods());
        assert_eq!(p.len(), 1, "{p:?}");
        assert!(p[0].starts_with("line 9: `device lst`:"), "{p:?}");
    }

    #[test]
    fn an_unknown_flag_fails() {
        let body = "```sh\njidr device start Phone --wait\n```\n";
        assert_eq!(
            check_skill("jidr", &skill(OK_FM, body), &methods()).len(),
            1
        );
    }

    #[test]
    fn an_unknown_protocol_method_fails() {
        let body = "```bash\njidr call media.screenshoot '{}'\n```\n";
        let p = check_skill("jidr", &skill(OK_FM, body), &methods());
        assert!(
            p[0].contains("`media.screenshoot` is not a protocol command"),
            "{p:?}"
        );
    }

    #[test]
    fn non_shell_fences_are_ignored() {
        let body = "```json\njidr nonsense\n```\n\njidr nonsense outside a fence\n";
        assert!(check_skill("jidr", &skill(OK_FM, body), &methods()).is_empty());
    }

    #[test]
    fn commands_split_on_operators_and_drop_comments_and_redirections() {
        let cmds = commands(
            r#"SERIAL=x jidr device list 2>/dev/null | grep "a b" && echo $(jidr profile list) # jidr bogus"#,
        );
        assert_eq!(
            cmds,
            [
                vec!["jidr", "device", "list"],
                vec!["grep", "a b"],
                vec!["echo"],
                vec!["jidr", "profile", "list"],
            ]
        );
    }

    #[test]
    fn continuation_lines_are_joined() {
        let text = "```bash\njidr device create Phone \\\n  --image 35\n```\n";
        assert_eq!(
            jidr_commands(text),
            [(
                2,
                vec![
                    "device".to_string(),
                    "create".into(),
                    "Phone".into(),
                    "--image".into(),
                    "35".into()
                ]
            )]
        );
    }

    #[test]
    fn protocol_methods_come_from_level_4_headings() {
        let m = protocol_methods("### `device.*`\n\n#### `device.list`\n\ntext `x.y`\n");
        assert_eq!(m.into_iter().collect::<Vec<_>>(), ["device.list"]);
    }

    #[test]
    fn the_repo_skills_pass() {
        run().unwrap();
    }
}
