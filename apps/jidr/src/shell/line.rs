//! Shell lines: splitting into words, finding where they land in the `jidr`
//! clap tree, and filling in the current device (`use <device>`).

use clap::{Arg, Command};
use serde_json::{Value, json};

/// Splits a line into words like a POSIX shell, without expansion: `'…'` is
/// literal, `"…"` honours `\"` and `\\`, a backslash outside quotes escapes
/// the next character, and an unquoted `#` at the start of a word begins a
/// comment.
pub fn split(line: &str) -> Result<Vec<String>, String> {
    let mut words = Vec::new();
    let mut cur = String::new();
    // A word has started (so `''` gives an empty word).
    let mut in_word = false;
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        match c {
            c if c.is_whitespace() => {
                if in_word {
                    words.push(std::mem::take(&mut cur));
                    in_word = false;
                }
            }
            '#' if !in_word => break,
            '\'' => {
                in_word = true;
                loop {
                    match chars.next() {
                        Some('\'') => break,
                        Some(c) => cur.push(c),
                        None => return Err("unclosed ' quote".into()),
                    }
                }
            }
            '"' => {
                in_word = true;
                loop {
                    match chars.next() {
                        Some('"') => break,
                        Some('\\') => match chars.next() {
                            Some(c @ ('"' | '\\')) => cur.push(c),
                            Some(c) => {
                                cur.push('\\');
                                cur.push(c);
                            }
                            None => return Err("unclosed \" quote".into()),
                        },
                        Some(c) => cur.push(c),
                        None => return Err("unclosed \" quote".into()),
                    }
                }
            }
            '\\' => {
                in_word = true;
                match chars.next() {
                    Some(c) => cur.push(c),
                    None => return Err("a line can't end with \\".into()),
                }
            }
            c => {
                in_word = true;
                cur.push(c);
            }
        }
    }
    if in_word {
        words.push(cur);
    }
    Ok(words)
}

/// The `jidr` command tree with help flags and globals propagated, ready
/// for [`walk`].
pub fn tree() -> Command {
    use clap::CommandFactory;
    let mut c = crate::cli::Cli::command();
    c.build();
    c
}

/// Where a list of words lands in the command tree.
#[derive(Debug)]
pub struct Walk {
    /// The deepest subcommand the words name.
    pub leaf: Command,
    /// Index just past the last subcommand word.
    pub path_end: usize,
    /// Indices of the positional words given to `leaf`.
    pub positionals: Vec<usize>,
    /// Ids of the options and flags given (unknown ones by their name).
    pub seen: Vec<String>,
    /// The last word was an option still waiting for its value.
    pub pending: Option<Arg>,
}

fn find_long<'a>(node: &'a Command, root: &'a Command, name: &str) -> Option<&'a Arg> {
    let hit = |c: &'a Command| c.get_arguments().find(|a| a.get_long() == Some(name));
    hit(node).or_else(|| hit(root))
}

fn find_short<'a>(node: &'a Command, root: &'a Command, ch: char) -> Option<&'a Arg> {
    let hit = |c: &'a Command| c.get_arguments().find(|a| a.get_short() == Some(ch));
    hit(node).or_else(|| hit(root))
}

fn takes_value(arg: Option<&Arg>) -> bool {
    arg.is_some_and(|a| a.get_action().takes_values())
}

/// Follows `words` (without the leading `jidr`) down `root`.
pub fn walk(root: &Command, words: &[String]) -> Walk {
    let mut w = Walk {
        leaf: root.clone(),
        path_end: 0,
        positionals: Vec::new(),
        seen: Vec::new(),
        pending: None,
    };
    let mut only_positionals = false;
    let mut i = 0;
    while i < words.len() {
        let t = words[i].as_str();
        i += 1;
        if only_positionals {
            w.positionals.push(i - 1);
            continue;
        }
        if t == "--" {
            only_positionals = true;
            continue;
        }
        if let Some(long) = t.strip_prefix("--") {
            let (name, inline) = match long.split_once('=') {
                Some((n, _)) => (n, true),
                None => (long, false),
            };
            let arg = find_long(&w.leaf, root, name);
            w.seen
                .push(arg.map_or(name.to_string(), |a| a.get_id().to_string()));
            if takes_value(arg) && !inline {
                if i < words.len() {
                    i += 1;
                } else {
                    w.pending = arg.cloned();
                }
            }
            continue;
        }
        if let Some(shorts) = t.strip_prefix('-').filter(|s| !s.is_empty()) {
            for (k, ch) in shorts.char_indices() {
                let arg = find_short(&w.leaf, root, ch);
                w.seen
                    .push(arg.map_or(ch.to_string(), |a| a.get_id().to_string()));
                if takes_value(arg) {
                    if k + ch.len_utf8() == shorts.len() {
                        if i < words.len() {
                            i += 1;
                        } else {
                            w.pending = arg.cloned();
                        }
                    }
                    break;
                }
            }
            continue;
        }
        if w.positionals.is_empty()
            && let Some(sub) = w.leaf.find_subcommand(t).cloned()
        {
            w.leaf = sub;
            w.path_end = i;
            continue;
        }
        w.positionals.push(i - 1);
    }
    w
}

/// Positional ids that name a device (`device` everywhere, `names` in
/// `device start|stop|hibernate|delete`).
pub fn is_device_arg(arg: &Arg) -> bool {
    matches!(arg.get_id().as_str(), "device" | "names")
}

/// Puts `device` in as the device of a command that takes one first and
/// wasn't given one. A command was given one when its first positional word
/// is a known device name or id (`known`) or `device` itself; otherwise that
/// word is the command's next argument (`snapshot save before-login`,
/// `proxy set 8080`). Lines that pass `--all` or ask for help are left alone.
pub fn fill_device(
    root: &Command,
    words: Vec<String>,
    device: &str,
    known: &[String],
) -> Vec<String> {
    let w = walk(root, &words);
    if w.leaf.has_subcommands()
        || !w.leaf.get_positionals().next().is_some_and(is_device_arg)
        || w.seen.iter().any(|s| s == "all" || s == "help")
    {
        return words;
    }
    if let Some(&first) = w.positionals.first() {
        let given = &words[first];
        if given == device || known.iter().any(|k| k == given) {
            return words;
        }
    }
    let mut words = words;
    words.insert(w.path_end, device.to_string());
    words
}

/// `jidr call`'s params with `"device"` added when they don't name one.
/// Params that aren't a JSON object are left for the daemon to reject.
pub fn call_params(params: Option<String>, device: &str) -> Option<String> {
    let mut v: Value = match &params {
        None => json!({}),
        Some(p) => match serde_json::from_str(p) {
            Ok(v) => v,
            Err(_) => return params,
        },
    };
    match v.as_object_mut() {
        Some(obj) if !obj.contains_key("device") => {
            obj.insert("device".into(), json!(device));
            Some(v.to_string())
        }
        _ => params,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(s: &[&str]) -> Vec<String> {
        s.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn splits_like_a_shell() {
        assert_eq!(split("device list").unwrap(), w(&["device", "list"]));
        assert_eq!(
            split("  proxy  set Lab   8080 ").unwrap(),
            w(&["proxy", "set", "Lab", "8080"])
        );
        assert_eq!(
            split(r#"call device.start '{"device":"My Phone"}'"#).unwrap(),
            w(&["call", "device.start", r#"{"device":"My Phone"}"#])
        );
        assert_eq!(
            split(r#"push "My Phone" a\ b.txt"#).unwrap(),
            w(&["push", "My Phone", "a b.txt"])
        );
        assert_eq!(split(r#"x "a\"b\\c\n""#).unwrap(), w(&["x", r#"a"b\c\n"#]));
        assert_eq!(split("x '' y").unwrap(), w(&["x", "", "y"]));
        assert_eq!(
            split("device list # all of them").unwrap(),
            w(&["device", "list"])
        );
        assert_eq!(split("   # only a comment").unwrap(), Vec::<String>::new());
        assert_eq!(
            split("logcat P --filter a#b").unwrap(),
            w(&["logcat", "P", "--filter", "a#b"])
        );
        assert_eq!(split("").unwrap(), Vec::<String>::new());
        for bad in ["call x '{", r#"call x "{"#, "x \\"] {
            assert!(split(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn walks_the_cli_tree() {
        let root = tree();
        let x = walk(
            &root,
            &w(&["--json", "device", "start", "--timeout", "30", "Phone"]),
        );
        assert_eq!(x.leaf.get_name(), "start");
        assert_eq!(x.path_end, 3);
        assert_eq!(x.positionals, vec![5]);
        assert_eq!(x.seen, w(&["json", "timeout"]));
        let x = walk(&root, &w(&["logcat", "P", "-n"]));
        assert_eq!(
            x.pending.map(|a| a.get_id().to_string()),
            Some("lines".into())
        );
        let x = walk(&root, &w(&["logcat", "-n5", "P"]));
        assert_eq!(x.positionals, vec![2]);
        let x = walk(&root, &w(&["call", "--", "device.list"]));
        assert_eq!(x.positionals, vec![2]);
    }

    fn fill(line: &[&str], known: &[&str]) -> Vec<String> {
        fill_device(&tree(), w(line), "Phone", &w(known))
    }

    #[test]
    fn fills_in_the_current_device() {
        let known = ["Phone", "Tablet", "0b6f…id"];
        // Commands that take a device and weren't given one.
        assert_eq!(
            fill(&["device", "start"], &known),
            w(&["device", "start", "Phone"])
        );
        assert_eq!(
            fill(&["device", "start", "--timeout", "300"], &known),
            w(&["device", "start", "Phone", "--timeout", "300"])
        );
        assert_eq!(
            fill(&["proxy", "set", "8080"], &known),
            w(&["proxy", "set", "Phone", "8080"])
        );
        assert_eq!(
            fill(&["proxy", "show"], &known),
            w(&["proxy", "show", "Phone"])
        );
        assert_eq!(
            fill(&["snapshot", "save", "before-login"], &known),
            w(&["snapshot", "save", "Phone", "before-login"])
        );
        assert_eq!(
            fill(&["logcat", "-n", "20"], &known),
            w(&["logcat", "Phone", "-n", "20"])
        );
        assert_eq!(
            fill(&["--json", "install", "app.apk", "--grant"], &known),
            w(&["--json", "install", "Phone", "app.apk", "--grant"])
        );
        assert_eq!(
            fill(&["push", "a.txt", "/sdcard/a.txt"], &known),
            w(&["push", "Phone", "a.txt", "/sdcard/a.txt"])
        );
        assert_eq!(
            fill(&["device", "clone", "Copy"], &known),
            w(&["device", "clone", "Phone", "Copy"])
        );
        // An explicit device (by name or id) wins.
        for line in [
            &["proxy", "set", "Tablet", "8080"][..],
            &["device", "start", "Tablet", "Phone"],
            &["snapshot", "save", "Phone", "s1"],
            &["device", "details", "0b6f…id"],
        ] {
            assert_eq!(fill(line, &known), w(line), "{line:?}");
        }
        // Commands that don't take a device, `--all`, help and incomplete lines.
        for line in [
            &["device", "list"][..],
            &["device", "create", "New", "--image", "35"],
            &["device", "stop", "--all"],
            &["image", "list"],
            &["proxy", "set", "--help"],
            &["proxy"],
            &["pool", "release", "lease-1"],
            &["call", "device.list"],
        ] {
            assert_eq!(fill(line, &known), w(line), "{line:?}");
        }
        // With an unknown device list, only the current device counts as given.
        assert_eq!(
            fill(&["proxy", "show", "Phone"], &[]),
            w(&["proxy", "show", "Phone"])
        );
    }

    #[test]
    fn call_params_get_the_device() {
        assert_eq!(call_params(None, "P").as_deref(), Some(r#"{"device":"P"}"#));
        assert_eq!(
            call_params(Some(r#"{"angle":90}"#.into()), "P").as_deref(),
            Some(r#"{"angle":90,"device":"P"}"#)
        );
        for kept in [r#"{"device":"Q"}"#, "[1]", "{bad"] {
            assert_eq!(call_params(Some(kept.into()), "P").as_deref(), Some(kept));
        }
    }
}
