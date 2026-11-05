//! Tab completion: shell builtins, the CLI's subcommands and flags, device
//! names, protocol methods from `commands.list` and file paths.

use std::sync::{Arc, Mutex, PoisonError};

use clap::{Command, ValueHint};
use rustyline::Context;
use rustyline::completion::{Completer, FilenameCompleter, Pair};
use rustyline::highlight::Highlighter;
use rustyline::hint::Hinter;
use rustyline::validate::Validator;

use super::line::{is_device_arg, split, walk};

/// Words the shell handles itself.
pub const BUILTINS: &[&str] = &["use", "source", "help", "exit", "quit"];

/// What completion knows about the daemon, refreshed between lines.
#[derive(Debug, Default)]
pub struct Vocab {
    /// Device names.
    pub devices: Vec<String>,
    /// Device names and ids.
    pub known: Vec<String>,
    /// Protocol methods (`commands.list`).
    pub methods: Vec<String>,
    /// The methods whose params have a `device`.
    pub device_methods: Vec<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Candidates {
    Words(Vec<String>),
    Files,
}

fn names(cmd: &Command) -> Vec<String> {
    cmd.get_subcommands()
        .filter(|s| !s.is_hide_set())
        .map(|s| s.get_name().to_string())
        .collect()
}

fn flags(cmd: &Command, root: &Command) -> Vec<String> {
    cmd.get_arguments()
        .chain(root.get_arguments().filter(|a| a.is_global_set()))
        .filter(|a| !a.is_hide_set())
        .filter_map(|a| a.get_long().map(|l| format!("--{l}")))
        .collect()
}

/// Candidates for the word after `before` (the words already typed).
pub fn candidates(root: &Command, vocab: &Vocab, before: &[String], word: &str) -> Candidates {
    let before = match before.first() {
        Some(j) if j == "jidr" => &before[1..],
        _ => before,
    };
    let Some(first) = before.first() else {
        if word.starts_with('-') {
            return Candidates::Words(flags(root, root));
        }
        let mut all: Vec<String> = BUILTINS.iter().map(|b| b.to_string()).collect();
        all.extend(names(root));
        return Candidates::Words(all);
    };
    match first.as_str() {
        "use" if before.len() == 1 => return Candidates::Words(vocab.devices.clone()),
        "source" if before.len() == 1 => return Candidates::Files,
        "help" => return Candidates::Words(names(&walk(root, &before[1..]).leaf)),
        "use" | "source" | "exit" | "quit" => return Candidates::Words(Vec::new()),
        _ => {}
    }
    let w = walk(root, before);
    let value_of = |arg: &clap::Arg| {
        let values: Vec<String> = arg
            .get_possible_values()
            .iter()
            .filter(|v| !v.is_hide_set())
            .map(|v| v.get_name().to_string())
            .collect();
        if !values.is_empty() {
            Candidates::Words(values)
        } else if matches!(
            arg.get_value_hint(),
            ValueHint::AnyPath | ValueHint::FilePath | ValueHint::DirPath
        ) {
            Candidates::Files
        } else {
            Candidates::Words(Vec::new())
        }
    };
    if let Some(arg) = &w.pending {
        return value_of(arg);
    }
    if word.starts_with('-') {
        return Candidates::Words(flags(&w.leaf, root));
    }
    if w.positionals.is_empty() && w.leaf.has_subcommands() {
        return Candidates::Words(names(&w.leaf));
    }
    let n = w.positionals.len();
    let positionals: Vec<&clap::Arg> = w.leaf.get_positionals().collect();
    let arg = positionals.get(n).or_else(|| {
        positionals
            .last()
            .filter(|a| a.get_num_args().is_some_and(|r| r.max_values() > 1))
    });
    match arg {
        Some(a) if is_device_arg(a) => Candidates::Words(vocab.devices.clone()),
        Some(_) if w.leaf.get_name() == "call" && n == 0 => {
            Candidates::Words(vocab.methods.clone())
        }
        Some(a) => value_of(a),
        None => Candidates::Words(Vec::new()),
    }
}

/// The rustyline helper.
pub struct ShellHelper {
    pub vocab: Arc<Mutex<Vocab>>,
    pub root: Command,
    pub files: FilenameCompleter,
}

impl Completer for ShellHelper {
    type Candidate = Pair;

    fn complete(
        &self,
        line: &str,
        pos: usize,
        _: &Context<'_>,
    ) -> rustyline::Result<(usize, Vec<Pair>)> {
        let head = line.get(..pos).unwrap_or(line);
        let start = head
            .char_indices()
            .rev()
            .find(|(_, c)| c.is_whitespace())
            .map_or(0, |(i, c)| i + c.len_utf8());
        let (typed, word) = head.split_at(start);
        let before =
            split(typed).unwrap_or_else(|_| typed.split_whitespace().map(String::from).collect());
        let found = {
            let vocab = self.vocab.lock().unwrap_or_else(PoisonError::into_inner);
            candidates(&self.root, &vocab, &before, word)
        };
        match found {
            Candidates::Files => self.files.complete_path(line, pos),
            Candidates::Words(mut list) => {
                list.retain(|c| c.starts_with(word));
                list.sort();
                list.dedup();
                Ok((
                    start,
                    list.into_iter()
                        .map(|c| Pair {
                            display: c.clone(),
                            replacement: if c.contains(char::is_whitespace) {
                                format!("'{c}' ")
                            } else {
                                format!("{c} ")
                            },
                        })
                        .collect(),
                ))
            }
        }
    }
}

impl Hinter for ShellHelper {
    type Hint = String;
}
impl Highlighter for ShellHelper {}
impl Validator for ShellHelper {}
impl rustyline::Helper for ShellHelper {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::line::tree;

    fn vocab() -> Vocab {
        Vocab {
            devices: vec!["Phone".into(), "Tablet".into()],
            known: Vec::new(),
            methods: vec!["device.list".into(), "gps.set".into()],
            device_methods: Vec::new(),
        }
    }

    fn words(line: &str, word: &str) -> Vec<String> {
        let before: Vec<String> = line.split_whitespace().map(String::from).collect();
        match candidates(&tree(), &vocab(), &before, word) {
            Candidates::Words(mut w) => {
                w.retain(|c| c.starts_with(word));
                w
            }
            Candidates::Files => vec!["<files>".into()],
        }
    }

    #[test]
    fn completes_commands_devices_and_methods() {
        let first = words("", "");
        for w in ["use", "source", "device", "proxy", "call", "exit"] {
            assert!(first.contains(&w.to_string()), "{w} in {first:?}");
        }
        assert_eq!(words("", "pro"), vec!["profile", "proxy"]);
        assert!(words("device", "").contains(&"start".to_string()));
        assert_eq!(words("proxy set", ""), vec!["Phone", "Tablet"]);
        assert_eq!(words("device start Phone", ""), vec!["Phone", "Tablet"]);
        assert_eq!(words("use", "T"), vec!["Tablet"]);
        assert_eq!(words("call", "gp"), vec!["gps.set"]);
        assert_eq!(words("root set Phone", ""), vec!["none", "adb", "magisk"]);
        assert_eq!(
            words("device create X --root", ""),
            vec!["none", "adb", "magisk"]
        );
        assert_eq!(words("install Phone", ""), vec!["<files>"]);
        assert_eq!(words("source", ""), vec!["<files>"]);
        assert!(words("device start", "--").contains(&"--cold-boot".to_string()));
        assert!(words("device start", "--").contains(&"--json".to_string()));
        assert_eq!(words("help device", "sta"), vec!["start"]);
        assert_eq!(words("jidr proxy", "s"), vec!["set", "show"]);
        assert!(words("device list", "").is_empty());
    }
}
