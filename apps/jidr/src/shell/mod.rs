//! `jidr shell`: an interactive shell over the same daemon connection and
//! command definitions as the one-shot CLI (issue #73). Each line is a `jidr`
//! command without the `jidr`, plus a few builtins: `use [device]` sets the
//! device filled in for commands that take one, `source <file>` runs a
//! script, `help` and `exit`. It adds no protocol surface.

mod complete;
mod line;

use std::io::IsTerminal;
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use clap::Parser;
use jidr_client::Client;
use jidr_core::{ErrorCode, JidrError, JidrPaths, JidrResult};
use rustyline::error::ReadlineError;
use rustyline::history::FileHistory;
use rustyline::{CompletionType, Config, Editor};
use serde_json::{Value, json};

use crate::cli::{Cli, Cmd, DaemonCmd};
use crate::output::Out;
use complete::{ShellHelper, Vocab};

/// File under Jidr's data directory that keeps the shell's history.
pub const HISTORY_FILE: &str = "shell_history";
/// How deeply `source` may nest.
const MAX_DEPTH: usize = 16;

/// What a line asks the shell to do next.
#[derive(Debug, PartialEq, Eq)]
pub enum Flow {
    Next,
    Exit(u8),
}

/// How a script ended.
#[derive(Debug, PartialEq, Eq)]
pub enum ScriptEnd {
    Done,
    /// `exit [code]`.
    Exit(u8),
    /// The line at `line` (1-based) failed with exit code `code`.
    Failed {
        line: usize,
        code: u8,
    },
}

impl ScriptEnd {
    pub fn code(&self) -> u8 {
        match *self {
            Self::Done => 0,
            Self::Exit(c) | Self::Failed { code: c, .. } => c,
        }
    }
}

/// Runs a script's lines in order with `exec`, skipping blank lines and `#`
/// comments; stops at `exit` or at the first line that fails (`Err(code)`).
pub fn run_lines(text: &str, mut exec: impl FnMut(&str) -> Result<Flow, u8>) -> ScriptEnd {
    for (i, raw) in text.lines().enumerate() {
        let l = raw.trim();
        if l.is_empty() || l.starts_with('#') {
            continue;
        }
        match exec(l) {
            Ok(Flow::Next) => {}
            Ok(Flow::Exit(c)) => return ScriptEnd::Exit(c),
            Err(code) => return ScriptEnd::Failed { line: i + 1, code },
        }
    }
    ScriptEnd::Done
}

/// `jidr shell [script]`: a script file, piped lines, or the interactive shell.
pub fn main(cli: &Cli, script: Option<&Path>, out: Out) -> u8 {
    let rt = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => return out.err(&JidrError::unexpected(format!("tokio runtime: {e}"))),
    };
    let paths = match JidrPaths::discover() {
        Ok(p) => p,
        Err(e) => return out.err(&e),
    };
    let mut s = Session {
        rt,
        paths,
        out,
        restart_stale: cli.restart_stale,
        root: line::tree(),
        client: None,
        device: None,
        vocab: Arc::new(Mutex::new(Vocab::default())),
        depth: 0,
    };
    match script {
        Some(p) if p != Path::new("-") => s.script_file(p),
        _ if script.is_some() || !std::io::stdin().is_terminal() => {
            let mut text = String::new();
            if let Err(e) = std::io::Read::read_to_string(&mut std::io::stdin(), &mut text) {
                return out.err(&JidrError::unexpected(format!("reading stdin: {e}")));
            }
            s.script("<stdin>", &text).code()
        }
        _ => s.repl(),
    }
}

struct Session {
    rt: tokio::runtime::Runtime,
    paths: JidrPaths,
    /// The shell's own `--json` / `--quiet`, applied to every line.
    out: Out,
    restart_stale: bool,
    root: clap::Command,
    /// Opened on first use and dropped when the daemon goes away.
    client: Option<Client>,
    /// `use <device>`.
    device: Option<String>,
    vocab: Arc<Mutex<Vocab>>,
    depth: usize,
}

/// Runs `f`, giving up on Ctrl-C (the shell stays up; jobs keep running in
/// the daemon, as when Ctrl-C ends a one-shot `jidr`).
async fn interruptible<T>(f: impl Future<Output = JidrResult<T>>) -> JidrResult<T> {
    tokio::select! {
        r = f => r,
        _ = tokio::signal::ctrl_c() => Err(jidr_core::cancel::cancelled()),
    }
}

impl Session {
    fn vocab(&self) -> MutexGuard<'_, Vocab> {
        self.vocab.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn connect(&mut self, out: Out) -> JidrResult<&Client> {
        if self.client.is_none() {
            let c = self
                .rt
                .block_on(crate::connect(&self.paths, self.restart_stale, out))?;
            self.client = Some(c);
        }
        self.client
            .as_ref()
            .ok_or_else(|| JidrError::unexpected("no daemon connection"))
    }

    /// One call for the shell's own bookkeeping; a dead connection is dropped
    /// so the next command reconnects.
    fn quiet_call(&mut self, method: &str) -> Option<Value> {
        let c = self.client.as_ref()?;
        let r = self.rt.block_on(async {
            tokio::time::timeout(Duration::from_secs(5), c.call(method, json!({}))).await
        });
        match r {
            Ok(Ok(v)) => Some(v),
            Ok(Err(e)) if e.code == ErrorCode::Usage => None,
            _ => {
                self.client = None;
                None
            }
        }
    }

    /// Re-reads the device names (for completion and `use`).
    fn refresh_devices(&mut self) {
        let Some(list) = self.quiet_call("device.list") else {
            return;
        };
        let devices = list.as_array().cloned().unwrap_or_default();
        let mut v = self.vocab();
        v.devices = devices
            .iter()
            .filter_map(|d| d["name"].as_str().map(String::from))
            .collect();
        v.known = devices
            .iter()
            .flat_map(|d| [d["name"].as_str(), d["id"].as_str()])
            .flatten()
            .map(String::from)
            .collect();
    }

    /// Reads `commands.list` once.
    fn refresh_methods(&mut self) {
        if !self.vocab().methods.is_empty() {
            return;
        }
        let Some(list) = self.quiet_call("commands.list") else {
            return;
        };
        let specs = list.as_array().cloned().unwrap_or_default();
        let mut v = self.vocab();
        v.methods = specs
            .iter()
            .filter_map(|c| c["id"].as_str().map(String::from))
            .collect();
        v.device_methods = specs
            .iter()
            .filter(|c| c["params"]["properties"].get("device").is_some())
            .filter_map(|c| c["id"].as_str().map(String::from))
            .collect();
    }

    fn prompt(&self) -> String {
        match &self.device {
            Some(d) => format!("jidr ({d})> "),
            None => "jidr> ".into(),
        }
    }

    fn repl(&mut self) -> u8 {
        let out = self.out;
        if let Err(e) = self.connect(out) {
            out.err(&e);
        }
        self.refresh_devices();
        self.refresh_methods();
        let config = Config::builder()
            .auto_add_history(true)
            .completion_type(CompletionType::List)
            .max_history_size(1000)
            .map(|b| b.build())
            .unwrap_or_default();
        let mut ed: Editor<ShellHelper, FileHistory> = match Editor::with_config(config) {
            Ok(ed) => ed,
            Err(e) => return out.err(&JidrError::unexpected(format!("terminal: {e}"))),
        };
        ed.set_helper(Some(ShellHelper {
            vocab: self.vocab.clone(),
            root: self.root.clone(),
            files: rustyline::completion::FilenameCompleter::new(),
        }));
        let history = self.paths.data.join(HISTORY_FILE);
        let _ = std::fs::create_dir_all(&self.paths.data);
        let _ = ed.load_history(&history);
        out.info(&format!(
            "Jidr shell {}. `help` lists commands, `use <device>` picks a device, `exit` or Ctrl-D quits.",
            env!("CARGO_PKG_VERSION")
        ));
        let code = loop {
            match ed.readline(&self.prompt()) {
                Ok(l) => {
                    if let Ok(Flow::Exit(c)) = self.line(&l) {
                        break c;
                    }
                }
                Err(ReadlineError::Interrupted) => {}
                Err(ReadlineError::Eof) => break 0,
                Err(e) => break out.err(&JidrError::unexpected(format!("terminal: {e}"))),
            }
            if self.client.is_none() {
                let _ = self.connect(out);
            }
            self.refresh_devices();
            self.refresh_methods();
        };
        let _ = ed.save_history(&history);
        code
    }

    fn script_file(&mut self, path: &Path) -> u8 {
        match std::fs::read_to_string(path) {
            Ok(text) => self.script(&path.display().to_string(), &text).code(),
            Err(e) => self.out.err(&read_error(path, &e)),
        }
    }

    /// Runs a script; on failure says which line stopped it.
    fn script(&mut self, name: &str, text: &str) -> ScriptEnd {
        if self.depth >= MAX_DEPTH {
            let code = self.out.err(&JidrError::usage(format!(
                "`source` nests deeper than {MAX_DEPTH} files"
            )));
            return ScriptEnd::Failed { line: 0, code };
        }
        self.depth += 1;
        let end = run_lines(text, |l| self.line(l));
        self.depth -= 1;
        if let ScriptEnd::Failed { line, code } = end
            && line > 0
        {
            eprintln!("{name}:{line}: stopped here (exit code {code})");
        }
        end
    }

    /// Runs one line. `Err` carries the exit code of a failure, already reported.
    fn line(&mut self, text: &str) -> Result<Flow, u8> {
        let out = self.out;
        let mut words = line::split(text).map_err(|m| out.err(&JidrError::usage(m)))?;
        if words.first().is_some_and(|w| w == "jidr") {
            words.remove(0);
        }
        let Some(first) = words.first() else {
            return Ok(Flow::Next);
        };
        match first.as_str() {
            "exit" | "quit" => match words.get(1).map(|c| c.parse::<u8>()) {
                None => Ok(Flow::Exit(0)),
                Some(Ok(c)) if words.len() == 2 => Ok(Flow::Exit(c)),
                _ => Err(out.err(&JidrError::usage("usage: exit [code 0-255]"))),
            },
            "help" | "?" if words.len() == 1 => {
                self.help();
                Ok(Flow::Next)
            }
            "help" | "?" => {
                let mut cmd = words[1..].to_vec();
                cmd.push("--help".into());
                self.command(cmd)
            }
            "use" => match &words[1..] {
                [] => {
                    self.device = None;
                    out.ok(&json!({ "device": null }), |_| "No current device.".into());
                    Ok(Flow::Next)
                }
                [d] => {
                    let d = d.clone();
                    self.use_device(&d).map_err(|e| out.err(&e))?;
                    Ok(Flow::Next)
                }
                _ => Err(out.err(&JidrError::usage("usage: use [device]"))),
            },
            "source" | "." => match &words[1..] {
                [p] => {
                    let p = std::path::PathBuf::from(p);
                    let text =
                        std::fs::read_to_string(&p).map_err(|e| out.err(&read_error(&p, &e)))?;
                    match self.script(&p.display().to_string(), &text) {
                        ScriptEnd::Done => Ok(Flow::Next),
                        ScriptEnd::Exit(c) => Ok(Flow::Exit(c)),
                        ScriptEnd::Failed { code, .. } => Err(code),
                    }
                }
                _ => Err(out.err(&JidrError::usage("usage: source <file>"))),
            },
            _ => self.command(words),
        }
    }

    fn use_device(&mut self, name: &str) -> JidrResult<()> {
        let out = self.out;
        self.connect(out)?;
        let Some(c) = &self.client else {
            return Err(JidrError::unexpected("no daemon connection"));
        };
        let list = self
            .rt
            .block_on(interruptible(c.call("device.list", json!({}))))?;
        let found = list.as_array().and_then(|a| {
            a.iter()
                .find(|d| d["name"].as_str() == Some(name) || d["id"].as_str() == Some(name))
        });
        let Some(d) = found.and_then(|d| d["name"].as_str()) else {
            return Err(JidrError::not_found(format!(
                "no device named {name:?} (`device list` shows them)"
            )));
        };
        self.device = Some(d.to_string());
        out.ok(&json!({ "device": d }), |_| format!("Using {d}."));
        Ok(())
    }

    fn help(&self) {
        let mut text = String::from(
            "Shell commands:\n  use [device]     Fill in this device for commands that take one; `use` alone clears it\n  source <file>    Run a script (one command per line, `#` comments); stops at the first failure\n  help [command]   This help, or a command's help\n  exit [code]      Leave the shell (or Ctrl-D)\n\nJidr commands (without `jidr`; `--json` and `-q` work per line):\n",
        );
        for s in self.root.get_subcommands().filter(|s| !s.is_hide_set()) {
            if matches!(s.get_name(), "shell" | "mcp" | "completions" | "help") {
                continue;
            }
            let about = s.get_about().map(|a| a.to_string()).unwrap_or_default();
            let about = about.lines().next().unwrap_or_default();
            text.push_str(&format!("  {:<16} {about}\n", s.get_name()));
        }
        text.push_str("\nTab completes commands, flags, device names and protocol methods.");
        self.out
            .ok(&json!({ "builtins": complete::BUILTINS }), |_| text);
    }

    /// Parses and runs a `jidr` command line.
    fn command(&mut self, mut words: Vec<String>) -> Result<Flow, u8> {
        if let Some(d) = self.device.clone() {
            self.refresh_devices();
            let known = self.vocab().known.clone();
            words = line::fill_device(&self.root, words, &d, &known);
        }
        let mut argv = vec!["jidr".to_string()];
        if self.out.json {
            argv.push("--json".into());
        }
        if self.out.quiet {
            argv.push("--quiet".into());
        }
        argv.extend(words);
        let cli = match Cli::try_parse_from(&argv) {
            Ok(cli) => cli,
            Err(e) => return self.clap_error(&e, argv.iter().any(|a| a == "--json")),
        };
        let out = Out {
            json: cli.json,
            quiet: cli.quiet,
        };
        let mut cmd = cli.cmd;
        match &mut cmd {
            Cmd::Shell { .. }
            | Cmd::Mcp { .. }
            | Cmd::Daemon {
                action: None | Some(DaemonCmd::Run),
            } => {
                return Err(out.err(&JidrError::usage(
                    "`shell`, `mcp` and `daemon run` can't run inside the shell",
                )));
            }
            Cmd::Completions { shell } => {
                use clap::CommandFactory;
                clap_complete::generate(
                    *shell,
                    &mut Cli::command(),
                    "jidr",
                    &mut std::io::stdout(),
                );
                return Ok(Flow::Next);
            }
            Cmd::Call { method, params } => {
                if let Some(d) = self.device.clone() {
                    self.refresh_methods();
                    if self.vocab().device_methods.contains(method) {
                        *params = line::call_params(params.take(), &d);
                    }
                }
            }
            _ => {}
        }
        let drops_daemon = matches!(
            cmd,
            Cmd::Daemon {
                action: Some(DaemonCmd::Stop)
            } | Cmd::Uninstall { .. }
        );
        let r = self.run(cmd, out);
        if drops_daemon {
            self.client = None;
        }
        r.map(|()| Flow::Next).map_err(|e| out.err(&e))
    }

    fn run(&mut self, cmd: Cmd, out: Out) -> JidrResult<()> {
        if self
            .rt
            .block_on(interruptible(crate::before_connect(&self.paths, &cmd, out)))?
        {
            return Ok(());
        }
        self.connect(out)?;
        let Some(c) = &self.client else {
            return Err(JidrError::unexpected("no daemon connection"));
        };
        self.rt
            .block_on(interruptible(crate::dispatch(c, cmd, out)))
    }

    /// Prints clap's message (help is not a failure).
    fn clap_error(&self, e: &clap::Error, json: bool) -> Result<Flow, u8> {
        if e.exit_code() == 0 {
            let _ = e.print();
            return Ok(Flow::Next);
        }
        if json {
            let msg = e.render().to_string();
            let msg = msg.trim().trim_start_matches("error: ");
            return Err(Out { json, quiet: false }.err(&JidrError::usage(msg)));
        }
        let _ = e.print();
        Err(ErrorCode::Usage.exit_code())
    }
}

fn read_error(path: &Path, e: &std::io::Error) -> JidrError {
    let msg = format!("can't read {}: {e}", path.display());
    if e.kind() == std::io::ErrorKind::NotFound {
        JidrError::not_found(msg)
    } else {
        JidrError::unexpected(msg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs `script` against a fake executor that fails on lines starting
    /// with `fail` (exit 3); returns the lines it ran.
    fn run(script: &str) -> (ScriptEnd, Vec<String>) {
        let mut ran = Vec::new();
        let end = run_lines(script, |l| {
            ran.push(l.to_string());
            if l.starts_with("fail") {
                Err(3)
            } else if l == "exit 4" {
                Ok(Flow::Exit(4))
            } else {
                Ok(Flow::Next)
            }
        });
        (end, ran)
    }

    #[test]
    fn scripts_skip_comments_and_blank_lines() {
        let (end, ran) = run(
            "#!/usr/bin/env -S jidr shell\n\n# set up\n  device list  \n\n   # indented comment\nuse Phone\n",
        );
        assert_eq!(end, ScriptEnd::Done);
        assert_eq!(end.code(), 0);
        assert_eq!(ran, vec!["device list", "use Phone"]);
    }

    #[test]
    fn scripts_stop_at_the_first_failure_with_its_line() {
        let (end, ran) = run("device list\n# comment\n\nfail here\ndevice stop Phone\n");
        assert_eq!(end, ScriptEnd::Failed { line: 4, code: 3 });
        assert_eq!(end.code(), 3);
        assert_eq!(ran, vec!["device list", "fail here"]);
    }

    #[test]
    fn scripts_end_at_exit() {
        let (end, ran) = run("a\nexit 4\nb\n");
        assert_eq!(end, ScriptEnd::Exit(4));
        assert_eq!(ran, vec!["a", "exit 4"]);
        assert_eq!(run("").0, ScriptEnd::Done);
    }
}
