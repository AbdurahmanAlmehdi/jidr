//! The emulator's telnet console (`127.0.0.1:<console port>`), for what gRPC
//! doesn't cover: network speed/delay, gsm, power ac/health.

use std::path::{Path, PathBuf};
use std::time::Duration;

use jidr_core::{ErrorCode, JidrError, JidrResult, ResultExt};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::time::timeout;

const IO_TIMEOUT: Duration = Duration::from_secs(10);

/// The token file the emulator creates for console auth.
pub fn default_token_path() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".emulator_console_auth_token"))
}

#[derive(Debug)]
pub struct Console {
    reader: BufReader<OwnedReadHalf>,
    writer: OwnedWriteHalf,
}

impl Console {
    /// Connects and authenticates with the token in `token_file` (an empty
    /// or missing file means the console has auth disabled).
    pub async fn connect(port: u16, token_file: Option<&Path>) -> JidrResult<Self> {
        let stream = timeout(IO_TIMEOUT, TcpStream::connect(("127.0.0.1", port)))
            .await
            .map_err(|_| JidrError::sdk(format!("emulator console on {port} timed out")))?
            .ctx(ErrorCode::Sdk, format!("emulator console on {port}"))?;
        let (r, w) = stream.into_split();
        let mut console = Self {
            reader: BufReader::new(r),
            writer: w,
        };
        let banner = console.read_reply().await?;
        if banner.iter().any(|l| l.contains("Authentication required")) {
            let token = token_file
                .and_then(|p| std::fs::read_to_string(p).ok())
                .map(|t| t.trim().to_string())
                .filter(|t| !t.is_empty())
                .ok_or_else(|| {
                    JidrError::sdk("emulator console needs auth but no token file was found")
                })?;
            console.command(&format!("auth {token}")).await?;
        }
        Ok(console)
    }

    /// Reads lines until `OK` (returns the lines before it) or `KO: msg`.
    async fn read_reply(&mut self) -> JidrResult<Vec<String>> {
        let mut lines = Vec::new();
        loop {
            let mut line = String::new();
            let n = timeout(IO_TIMEOUT, self.reader.read_line(&mut line))
                .await
                .map_err(|_| JidrError::sdk("emulator console reply timed out"))?
                .ctx(ErrorCode::Sdk, "reading emulator console")?;
            if n == 0 {
                return Err(JidrError::sdk("emulator console closed the connection"));
            }
            let line = line.trim_end_matches(['\r', '\n']).to_string();
            if line == "OK" {
                return Ok(lines);
            }
            if let Some(msg) = line.strip_prefix("KO:") {
                return Err(JidrError::usage(format!(
                    "emulator console: {}",
                    msg.trim()
                )));
            }
            lines.push(line);
        }
    }

    /// Sends one command and returns its output lines (without `OK`).
    pub async fn command(&mut self, cmd: &str) -> JidrResult<Vec<String>> {
        if cmd.contains(['\r', '\n']) {
            return Err(JidrError::usage("console commands are a single line"));
        }
        timeout(
            IO_TIMEOUT,
            self.writer.write_all(format!("{cmd}\r\n").as_bytes()),
        )
        .await
        .map_err(|_| JidrError::sdk("emulator console write timed out"))?
        .ctx(ErrorCode::Sdk, "writing emulator console")?;
        self.read_reply().await
    }

    pub async fn avd_name(&mut self) -> JidrResult<String> {
        Ok(self
            .command("avd name")
            .await?
            .join("\n")
            .trim()
            .to_string())
    }

    /// `gsm`, `hscsd`, `gprs`, `edge`, `umts`, `hsdpa`, `lte`, `evdo`, `full`,
    /// or `<up>:<down>` kbps.
    pub async fn network_speed(&mut self, speed: &str) -> JidrResult<()> {
        check_word(speed)?;
        self.command(&format!("network speed {speed}"))
            .await
            .map(|_| ())
    }

    /// `gprs`, `edge`, `umts`, `none`, or `<min>:<max>` ms.
    pub async fn network_delay(&mut self, delay: &str) -> JidrResult<()> {
        check_word(delay)?;
        self.command(&format!("network delay {delay}"))
            .await
            .map(|_| ())
    }

    /// `gsm signal <rssi 0..31> [<ber 0..7>]`.
    pub async fn gsm_signal(&mut self, rssi: u8, ber: Option<u8>) -> JidrResult<()> {
        if rssi > 31 || ber.is_some_and(|b| b > 7) {
            return Err(JidrError::usage("gsm signal: rssi 0..31, ber 0..7"));
        }
        let cmd = match ber {
            Some(b) => format!("gsm signal {rssi} {b}"),
            None => format!("gsm signal {rssi}"),
        };
        self.command(&cmd).await.map(|_| ())
    }

    /// `gsm data|voice <unregistered|home|roaming|searching|denied|off|on>`.
    pub async fn gsm_state(&mut self, which: GsmLine, state: &str) -> JidrResult<()> {
        const STATES: &[&str] = &[
            "unregistered",
            "home",
            "roaming",
            "searching",
            "denied",
            "off",
            "on",
        ];
        if !STATES.contains(&state) {
            return Err(JidrError::usage(format!(
                "gsm state must be one of {}",
                STATES.join(", ")
            )));
        }
        let line = match which {
            GsmLine::Data => "data",
            GsmLine::Voice => "voice",
        };
        self.command(&format!("gsm {line} {state}"))
            .await
            .map(|_| ())
    }

    pub async fn power_ac(&mut self, on: bool) -> JidrResult<()> {
        self.command(if on { "power ac on" } else { "power ac off" })
            .await
            .map(|_| ())
    }

    /// `unknown|good|overheat|dead|overvoltage|failure`.
    pub async fn power_health(&mut self, health: &str) -> JidrResult<()> {
        check_word(health)?;
        self.command(&format!("power health {health}"))
            .await
            .map(|_| ())
    }

    /// Asks the emulator to exit (no graceful snapshot save guarantee).
    pub async fn kill(mut self) -> JidrResult<()> {
        // The emulator may close the socket before answering.
        let _ = self.command("kill").await;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GsmLine {
    Data,
    Voice,
}

fn check_word(arg: &str) -> JidrResult<()> {
    if !arg.is_empty()
        && arg
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == ':' || c == '.')
    {
        Ok(())
    } else {
        Err(JidrError::usage(format!(
            "invalid console argument {arg:?}"
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;

    /// A fake console: auth required, records commands, answers like the emulator.
    async fn fake_console(token: &'static str) -> (u16, tokio::task::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = tokio::spawn(async move {
            let (sock, _) = listener.accept().await.unwrap();
            let (r, mut w) = sock.into_split();
            let mut r = BufReader::new(r);
            w.write_all(b"Android Console: Authentication required\r\nAndroid Console: type 'auth <auth_token>' to authenticate\r\nOK\r\n")
                .await
                .unwrap();
            let mut seen = Vec::new();
            let mut authed = false;
            loop {
                let mut line = String::new();
                if r.read_line(&mut line).await.unwrap() == 0 {
                    return seen;
                }
                let cmd = line.trim().to_string();
                seen.push(cmd.clone());
                let reply: String = if let Some(t) = cmd.strip_prefix("auth ") {
                    authed = t == token;
                    if authed {
                        "Android Console: type 'help'\r\nOK\r\n".into()
                    } else {
                        "KO: authentication token does not match\r\n".into()
                    }
                } else if !authed {
                    "KO: unknown command, try 'help'\r\n".into()
                } else if cmd == "avd name" {
                    "Phone_6_1\r\nOK\r\n".into()
                } else if cmd.starts_with("network speed bogus") {
                    "KO: invalid speed\r\n".into()
                } else {
                    "OK\r\n".into()
                };
                w.write_all(reply.as_bytes()).await.unwrap();
            }
        });
        (port, handle)
    }

    #[tokio::test]
    async fn authenticates_and_runs_commands() {
        let dir = tempfile::tempdir().unwrap();
        let token = dir.path().join("token");
        std::fs::write(&token, "s3cret\n").unwrap();
        let (port, server) = fake_console("s3cret").await;
        let mut c = Console::connect(port, Some(&token)).await.unwrap();
        assert_eq!(c.avd_name().await.unwrap(), "Phone_6_1");
        c.network_speed("edge").await.unwrap();
        c.network_delay("100:200").await.unwrap();
        c.gsm_signal(20, Some(3)).await.unwrap();
        c.gsm_state(GsmLine::Data, "roaming").await.unwrap();
        c.power_ac(false).await.unwrap();
        c.power_health("overheat").await.unwrap();
        let err = c.network_speed("bogus").await.unwrap_err();
        assert_eq!(err.code, ErrorCode::Usage);
        assert!(err.message.contains("invalid speed"));
        drop(c);
        let seen = server.await.unwrap();
        assert_eq!(seen[0], "auth s3cret");
        assert!(seen.contains(&"gsm data roaming".to_string()));
        assert!(seen.contains(&"power ac off".to_string()));
    }

    #[tokio::test]
    async fn wrong_token_and_bad_arguments() {
        let dir = tempfile::tempdir().unwrap();
        let token = dir.path().join("token");
        std::fs::write(&token, "nope").unwrap();
        let (port, _server) = fake_console("s3cret").await;
        assert!(Console::connect(port, Some(&token)).await.is_err());

        let (port, _server) = fake_console("t").await;
        std::fs::write(&token, "t").unwrap();
        let mut c = Console::connect(port, Some(&token)).await.unwrap();
        assert_eq!(
            c.network_speed("edge; kill").await.unwrap_err().code,
            ErrorCode::Usage
        );
        assert_eq!(
            c.gsm_signal(40, None).await.unwrap_err().code,
            ErrorCode::Usage
        );
        assert_eq!(
            c.gsm_state(GsmLine::Voice, "maybe").await.unwrap_err().code,
            ErrorCode::Usage
        );
        assert_eq!(c.command("a\nb").await.unwrap_err().code, ErrorCode::Usage);
    }

    #[tokio::test]
    async fn missing_token_file_is_a_clear_error() {
        let (port, _server) = fake_console("t").await;
        let err = Console::connect(port, Some(Path::new("/nonexistent/token")))
            .await
            .unwrap_err();
        assert!(err.message.contains("token"), "{err}");
    }
}
