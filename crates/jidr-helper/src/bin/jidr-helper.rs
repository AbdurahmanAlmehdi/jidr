//! `jidr-helper`: the rooted on-device helper (ADR-0003). Runs on the
//! Android guest only; the daemon pushes and starts it.
//!
//! - `jidr-helper start`: starts `serve` in the background as its own
//!   session and waits until it answers (what the daemon runs as root);
//! - `jidr-helper serve`: listens on the abstract socket `@jidr-helper`;
//! - `jidr-helper ping`: asks the running helper who it is;
//! - `jidr-helper --version`.

use std::process::ExitCode;

const USAGE: &str = "usage: jidr-helper start | serve | ping | --version";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("--version" | "version") => {
            println!(
                "jidr-helper {} (protocol {})",
                jidr_helper::VERSION,
                jidr_helper::rpc::PROTOCOL
            );
            ExitCode::SUCCESS
        }
        Some("serve") => guest::serve(),
        Some("start") => guest::start(),
        Some("ping") => guest::ping(),
        _ => {
            eprintln!("{USAGE}");
            ExitCode::from(2)
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "android"))]
mod guest {
    use std::io::{BufRead, BufReader, Write};
    #[cfg(target_os = "android")]
    use std::os::android::net::SocketAddrExt;
    #[cfg(target_os = "linux")]
    use std::os::linux::net::SocketAddrExt;
    use std::os::unix::net::{SocketAddr, UnixListener, UnixStream};
    use std::os::unix::process::CommandExt;
    use std::process::{Command, ExitCode, Stdio};
    use std::sync::Arc;
    use std::time::Duration;

    use jidr_helper::netem::System;
    use jidr_helper::rpc::{ALLOWED_UIDS, PROTOCOL, Pong, Response, SOCKET};
    use jidr_helper::server::{Next, Server};

    /// `serve` exits with this when another helper holds the socket.
    const ALREADY_RUNNING: i32 = 3;

    fn addr() -> std::io::Result<SocketAddr> {
        SocketAddr::from_abstract_name(SOCKET)
    }

    fn pong() -> std::io::Result<Pong> {
        let exe = std::fs::canonicalize("/proc/self/exe")?;
        let selinux = std::fs::read_to_string("/proc/self/attr/current")
            .ok()
            .map(|s| s.trim_end_matches(['\0', '\n']).to_string())
            .filter(|s| !s.is_empty());
        Ok(Pong {
            version: jidr_helper::VERSION.to_string(),
            protocol: PROTOCOL,
            sha256: jidr_helper::sha256_file(&exe)?,
            pid: std::process::id(),
            uid: rustix::process::getuid().as_raw(),
            selinux,
            arch: std::env::consts::ARCH.to_string(),
        })
    }

    pub fn serve() -> ExitCode {
        // Its own session: no controlling terminal, so the end of the adb
        // shell that started it doesn't hang it up.
        let _ = rustix::process::setsid();
        let pong = match pong() {
            Ok(p) => p,
            Err(e) => {
                eprintln!("jidr-helper: reading its own executable: {e}");
                return ExitCode::FAILURE;
            }
        };
        let listener = match addr().and_then(|a| UnixListener::bind_addr(&a)) {
            Ok(l) => l,
            Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => {
                eprintln!("jidr-helper: already running");
                return ExitCode::from(ALREADY_RUNNING as u8);
            }
            Err(e) => {
                eprintln!("jidr-helper: binding @{SOCKET}: {e}");
                return ExitCode::FAILURE;
            }
        };
        let server = Arc::new(Server::new(pong, System));
        for conn in listener.incoming() {
            let Ok(stream) = conn else { continue };
            // Only root and shell (adbd, which carries `adb forward`); an
            // app's connection is closed unanswered.
            let allowed = rustix::net::sockopt::socket_peercred(&stream)
                .is_ok_and(|c| ALLOWED_UIDS.contains(&c.uid.as_raw()));
            if !allowed {
                continue;
            }
            let server = Arc::clone(&server);
            std::thread::spawn(move || {
                if let Ok(Next::Exit) = server.serve(&stream) {
                    std::process::exit(0);
                }
            });
        }
        ExitCode::SUCCESS
    }

    fn ping_once() -> std::io::Result<String> {
        let mut s = UnixStream::connect_addr(&addr()?)?;
        s.set_read_timeout(Some(Duration::from_secs(2)))?;
        s.write_all(b"{\"id\":1,\"method\":\"ping\"}\n")?;
        let mut line = String::new();
        BufReader::new(&s).read_line(&mut line)?;
        let resp: Response = serde_json::from_str(&line)?;
        match (resp.result, resp.error) {
            (Some(v), _) => Ok(v.to_string()),
            (_, Some(e)) => Err(std::io::Error::other(e.to_string())),
            _ => Err(std::io::Error::other("empty response")),
        }
    }

    pub fn ping() -> ExitCode {
        match ping_once() {
            Ok(pong) => {
                println!("{pong}");
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("jidr-helper: not answering: {e}");
                ExitCode::FAILURE
            }
        }
    }

    /// Our own `serve` processes other than this one (a hung helper still
    /// holding the socket), from `/proc/<pid>/cmdline`.
    fn serve_pids() -> Vec<i32> {
        let me = std::process::id() as i32;
        let Ok(dir) = std::fs::read_dir("/proc") else {
            return Vec::new();
        };
        dir.filter_map(|e| e.ok()?.file_name().to_str()?.parse::<i32>().ok())
            .filter(|pid| *pid != me)
            .filter(|pid| {
                let Ok(cmdline) = std::fs::read(format!("/proc/{pid}/cmdline")) else {
                    return false;
                };
                let mut args = cmdline.split(|b| *b == 0);
                let exe = args.next().unwrap_or_default();
                exe.ends_with(b"jidr-helper") && args.next() == Some(b"serve".as_slice())
            })
            .collect()
    }

    /// Makes room for this build: asks an older helper to shut down, and
    /// kills one that holds the socket without answering.
    fn replace_running(mine: &str) -> Option<String> {
        match ping_once() {
            Ok(pong) if pong.contains(mine) => return Some(pong),
            Ok(_) => {
                if let Ok(mut s) = addr().and_then(|a| UnixStream::connect_addr(&a)) {
                    let _ = s.write_all(b"{\"id\":1,\"method\":\"shutdown\"}\n");
                    let mut line = String::new();
                    let _ = s.set_read_timeout(Some(Duration::from_secs(2)));
                    let _ = BufReader::new(&s).read_line(&mut line);
                }
            }
            Err(_) => {}
        }
        for _ in 0..20 {
            if addr().and_then(|a| UnixStream::connect_addr(&a)).is_err() {
                return None;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        for pid in serve_pids() {
            if let Some(pid) = rustix::process::Pid::from_raw(pid) {
                let _ = rustix::process::kill_process(pid, rustix::process::Signal::KILL);
            }
        }
        None
    }

    /// Idempotent: a helper of this build that already answers is kept, so
    /// two daemons calls starting it at once both succeed.
    pub fn start() -> ExitCode {
        let mine = match pong() {
            Ok(p) => p.sha256,
            Err(e) => {
                eprintln!("jidr-helper: reading its own executable: {e}");
                return ExitCode::FAILURE;
            }
        };
        if let Some(pong) = replace_running(&mine) {
            println!("{pong}");
            return ExitCode::SUCCESS;
        }
        let child = std::env::current_exe().and_then(|exe| {
            Command::new(exe)
                .arg("serve")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .process_group(0)
                .spawn()
        });
        let mut child = match child {
            Ok(c) => c,
            Err(e) => {
                eprintln!("jidr-helper: starting serve: {e}");
                return ExitCode::FAILURE;
            }
        };
        for _ in 0..50 {
            if let Ok(pong) = ping_once() {
                println!("{pong}");
                return ExitCode::SUCCESS;
            }
            match child.try_wait() {
                // Another start won the socket: wait for that one to answer.
                Ok(Some(s)) if s.code() == Some(ALREADY_RUNNING) => {}
                Ok(Some(s)) => {
                    eprintln!("jidr-helper: serve exited: {s}");
                    return ExitCode::FAILURE;
                }
                _ => {}
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        eprintln!("jidr-helper: serve didn't answer in 5 s");
        ExitCode::FAILURE
    }
}

#[cfg(not(any(target_os = "linux", target_os = "android")))]
mod guest {
    use std::process::ExitCode;

    fn guest_only() -> ExitCode {
        eprintln!("jidr-helper runs on the Android guest; build it with `cargo xtask helper`");
        ExitCode::from(2)
    }

    pub fn serve() -> ExitCode {
        guest_only()
    }

    pub fn start() -> ExitCode {
        guest_only()
    }

    pub fn ping() -> ExitCode {
        guest_only()
    }
}
