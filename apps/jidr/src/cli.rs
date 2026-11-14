//! Command-line definition (hand-written clap; every command is a thin call
//! over the daemon protocol).

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};

#[derive(Debug, Parser)]
#[command(
    name = "jidr",
    version,
    about = "Android emulators with one-click root, for people, CI and AI agents",
    after_help = "Exit codes: 0 ok · 1 unexpected · 2 usage · 3 not found · 4 busy · 5 boot timeout · 6 sdk · 7 root failed · 8 daemon stale · 9 resource exhausted"
)]
pub struct Cli {
    /// Machine-readable output: {"jidr_json": 1, "ok": …, "result"|"error": …}.
    #[arg(long, global = true)]
    pub json: bool,
    /// Only print results and errors (no progress).
    #[arg(long, short, global = true)]
    pub quiet: bool,
    /// Replace a running daemon of another version without asking.
    #[arg(long, global = true, env = "JIDR_RESTART_STALE")]
    pub restart_stale: bool,
    #[command(subcommand)]
    pub cmd: Cmd,
}

#[derive(Debug, Subcommand)]
pub enum Cmd {
    /// Run or control the Jidr daemon (started automatically when needed).
    Daemon {
        #[command(subcommand)]
        action: Option<DaemonCmd>,
    },
    /// Create, start, stop and manage devices.
    Device {
        #[command(subcommand)]
        cmd: DeviceCmd,
    },
    /// Install an APK on a running device.
    Install {
        device: String,
        apk: PathBuf,
        /// Fail if the app is already installed (default: replace it).
        #[arg(long)]
        no_replace: bool,
        /// Grant all runtime permissions.
        #[arg(long)]
        grant: bool,
        /// Allow a version downgrade.
        #[arg(long)]
        downgrade: bool,
    },
    /// Copy a file to a device (default destination /sdcard/Download/).
    Push {
        device: String,
        local: PathBuf,
        remote: Option<String>,
    },
    /// Copy a file from a device.
    Pull {
        device: String,
        remote: String,
        local: PathBuf,
    },
    /// Flash a zip (ADR-0004): a Magisk or Zygisk module installs and
    /// reboots a Full-root device; recovery (TWRP), Riru and OTA/factory
    /// image zips are refused (exit 2); anything else, or a module on a device
    /// without Full root, is copied to /sdcard/Download.
    Flash { device: String, zip: PathBuf },
    /// Show, follow or clear the device log.
    Logcat {
        device: String,
        /// Only the last N lines.
        #[arg(long, short = 'n')]
        lines: Option<u32>,
        /// Filter spec, e.g. `ActivityManager:I` (repeatable).
        #[arg(long = "filter")]
        filter: Vec<String>,
        /// Keep streaming new lines.
        #[arg(long, short)]
        follow: bool,
        /// Clear the log instead of showing it.
        #[arg(long, conflicts_with_all = ["follow", "lines"])]
        clear: bool,
    },
    /// List and install system images.
    Image {
        #[command(subcommand)]
        cmd: ImageCmd,
    },
    /// Hardware profiles.
    Profile {
        #[command(subcommand)]
        cmd: ProfileCmd,
    },
    /// The Android SDK Jidr uses.
    Sdk {
        #[command(subcommand)]
        cmd: SdkCmd,
    },
    /// Root a device: adb root or Magisk.
    Root {
        #[command(subcommand)]
        cmd: RootCmd,
    },
    /// Device snapshots, tagged with their root mode.
    Snapshot {
        #[command(subcommand)]
        cmd: SnapshotCmd,
    },
    /// CA certificates in a Full-root device's system store (Burp, mitmproxy, …).
    Cert {
        #[command(subcommand)]
        cmd: CertCmd,
    },
    /// The device's global HTTP proxy (Burp, mitmproxy, …).
    Proxy {
        #[command(subcommand)]
        cmd: ProxyCmd,
    },
    /// Network conditions set inside the guest by the rooted on-device helper.
    Network {
        #[command(subcommand)]
        cmd: NetworkCmd,
    },
    /// Play an audio file as a device's microphone (WAV, or raw 16-bit PCM).
    Audio {
        #[command(subcommand)]
        cmd: AudioCmd,
    },
    /// The download cache (Magisk APKs and other pinned files, by SHA-256).
    Cache {
        #[command(subcommand)]
        cmd: CacheCmd,
    },
    /// Host RAM, the device budget and what each running device uses (D42).
    Budget,
    /// Throwaway clones of a base for agents and CI: lease one, use its
    /// serial, release it (D42).
    Pool {
        #[command(subcommand)]
        cmd: PoolCmd,
    },
    /// Read-only bases that pool clones are made from.
    Base {
        #[command(subcommand)]
        cmd: BaseCmd,
    },
    /// Daemon-wide settings, shared with the desktop app.
    Settings {
        #[command(subcommand)]
        cmd: SettingsCmd,
    },
    /// Remove Jidr's daemon, MCP entries, caches and logs; `--purge` also
    /// deletes every Jidr device and its private SDK. Never touches Android
    /// Studio's SDK or devices.
    Uninstall {
        /// Also delete all Jidr data: devices, private SDK, settings.
        #[arg(long)]
        purge: bool,
        /// Don't ask for confirmation.
        #[arg(long, short)]
        yes: bool,
    },
    /// Long-running operations.
    Jobs {
        #[command(subcommand)]
        cmd: JobsCmd,
    },
    /// Serve MCP on stdio for AI agents (add to Claude: `claude mcp add jidr -- jidr mcp`).
    Mcp {
        /// Allow destructive commands (reset, delete…) on devices the agent didn't create (D5).
        #[arg(long)]
        allow_destructive: bool,
    },
    /// List every protocol command (the same list the GUI and MCP see).
    Commands,
    /// Call any protocol command directly: `jidr call device.list` or
    /// `jidr call device.start '{"device":"Phone"}'`.
    Call {
        method: String,
        /// Params as a JSON object.
        params: Option<String>,
    },
    /// Interactive shell: run `jidr` commands without the `jidr` prefix,
    /// with `use <device>`, tab completion and history. With a file (or
    /// piped input) it runs each line in order and stops at the first failure.
    Shell {
        /// A script of shell lines (`#` starts a comment).
        script: Option<PathBuf>,
    },
    /// Print shell completions.
    Completions { shell: clap_complete::Shell },
}

#[derive(Debug, Subcommand)]
pub enum DaemonCmd {
    /// Run the daemon in the foreground (what auto-start runs).
    Run,
    /// Stop the running daemon. Running devices keep running.
    Stop,
    /// Daemon version, uptime and device counts.
    Status,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum RootModeArg {
    None,
    Adb,
    Magisk,
}

impl RootModeArg {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Adb => "adb",
            Self::Magisk => "magisk",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum OnOff {
    On,
    Off,
}

impl OnOff {
    pub fn enabled(self) -> bool {
        self == Self::On
    }
}

#[derive(Debug, Args)]
pub struct Hardware {
    /// RAM in MB.
    #[arg(long = "ram")]
    pub ram_mb: Option<u32>,
    /// CPU cores.
    #[arg(long)]
    pub cpus: Option<u32>,
    /// Screen width in pixels.
    #[arg(long)]
    pub width: Option<u32>,
    /// Screen height in pixels.
    #[arg(long)]
    pub height: Option<u32>,
    /// Screen density in dpi.
    #[arg(long)]
    pub density: Option<u32>,
}

#[derive(Debug, Args)]
pub struct Selection {
    /// Device names or ids.
    pub names: Vec<String>,
    /// Every device.
    #[arg(long, conflicts_with = "names")]
    pub all: bool,
}

#[derive(Debug, Subcommand)]
pub enum DeviceCmd {
    /// Create a device.
    Create {
        name: String,
        /// System image: `35`, `35:google_apis`, `35:google_apis_playstore` or a full package.
        #[arg(long, short)]
        image: String,
        /// Hardware profile id (`jidr profile list`).
        #[arg(long, short)]
        profile: Option<String>,
        #[command(flatten)]
        hw: Hardware,
        /// Data partition in MB.
        #[arg(long)]
        data_mb: Option<u32>,
        /// Root mode (default: `jidr settings`' default root mode, adb unless
        /// changed; none where the image doesn't allow adb root). Magisk runs
        /// the Full-root setup right after creating.
        #[arg(long, value_enum)]
        root: Option<RootModeArg>,
        /// Let AI agents run destructive commands on it.
        #[arg(long)]
        agent_owned: bool,
    },
    /// Start devices and wait until they've booted.
    Start {
        #[command(flatten)]
        sel: Selection,
        /// Ignore the quick-boot snapshot.
        #[arg(long)]
        cold_boot: bool,
        /// Return as soon as the boot job starts.
        #[arg(long, conflicts_with = "timeout")]
        no_wait: bool,
        /// Give up (and stop the boot) after this many seconds.
        #[arg(long)]
        timeout: Option<u64>,
        /// Start even past the host RAM budget.
        #[arg(long)]
        force: bool,
    },
    /// Stop devices (saves their quick-boot state).
    Stop {
        #[command(flatten)]
        sel: Selection,
    },
    /// Save devices' whole state and stop them; `device start` resumes them
    /// in seconds. A hibernated device uses no RAM.
    Hibernate {
        #[command(flatten)]
        sel: Selection,
    },
    /// List devices.
    List,
    /// Settings, status and disk usage of a device.
    Details { device: String },
    /// Copy a stopped device.
    Clone {
        device: String,
        name: String,
        /// Keep this snapshot: an instant copy-on-write clone whose first
        /// start resumes it (otherwise the copy cold-boots).
        #[arg(long)]
        keep_snapshot: Option<String>,
    },
    /// Factory reset: wipe all user data.
    Reset {
        device: String,
        /// Don't ask for confirmation.
        #[arg(long, short)]
        yes: bool,
    },
    /// Delete devices and their data.
    Delete {
        #[command(flatten)]
        sel: Selection,
        /// Don't ask for confirmation.
        #[arg(long, short)]
        yes: bool,
    },
    /// Rename a device or change its hardware (while stopped).
    Edit {
        device: String,
        /// New name.
        #[arg(long)]
        rename: Option<String>,
        #[command(flatten)]
        hw: Hardware,
        /// Let AI agents run destructive commands on it.
        #[arg(long)]
        agent_owned: Option<bool>,
        /// Hibernate it after this many idle minutes (0 = never).
        #[arg(long, conflicts_with = "auto_hibernate_default")]
        auto_hibernate: Option<u32>,
        /// Follow the global auto-hibernate setting again.
        #[arg(long)]
        auto_hibernate_default: bool,
    },
    /// gRPC endpoint, token and adb serial of a running device.
    ConnectInfo { device: String },
    /// Zip a device's settings, emulator and crash logs and logcat (for bug reports).
    Logs {
        device: String,
        /// Where to write the zip (default: Jidr's logs folder).
        #[arg(long)]
        zip: Option<PathBuf>,
    },
}

#[derive(Debug, Subcommand)]
pub enum PoolCmd {
    /// Lease a clone of a base and boot it from the base's snapshot; prints
    /// its serial.
    Get {
        base: String,
        /// Lease lifetime: `90s`, `30m`, `2h`.
        #[arg(long, default_value = "2h")]
        ttl: String,
        /// Wait this many seconds for room in the RAM budget.
        #[arg(long)]
        timeout: Option<u64>,
    },
    /// Give a clone back (by lease id or device name): it is stopped and deleted.
    Release { lease_or_device: String },
    /// Extend a lease.
    Renew {
        lease: String,
        /// New lifetime from now: `90s`, `30m`, `2h`.
        #[arg(long, default_value = "2h")]
        ttl: String,
    },
    /// Current leases.
    Ls,
}

#[derive(Debug, Subcommand)]
pub enum BaseCmd {
    /// Save a running device (any root mode) as a base: snapshot
    /// `jidr-base` with animations off. The device is stopped.
    Create { device: String, name: String },
    /// Bases and their leased clones.
    Ls,
    /// Delete a base (no clone of it may be leased).
    Rm {
        base: String,
        /// Don't ask for confirmation.
        #[arg(long, short)]
        yes: bool,
    },
}

/// `90s`, `30m`, `2h`, `1d` or plain seconds, in seconds.
pub fn parse_duration(s: &str) -> Result<u64, String> {
    let s = s.trim();
    let (num, mult) = match s.char_indices().last() {
        Some((i, 's')) => (&s[..i], 1),
        Some((i, 'm')) => (&s[..i], 60),
        Some((i, 'h')) => (&s[..i], 3600),
        Some((i, 'd')) => (&s[..i], 86_400),
        _ => (s, 1),
    };
    num.parse::<u64>()
        .ok()
        .filter(|n| *n > 0)
        .map(|n| n * mult)
        .ok_or_else(|| format!("not a duration: {s:?} (use e.g. 90s, 30m, 2h)"))
}

#[derive(Debug, Subcommand)]
pub enum SettingsCmd {
    /// Show the settings.
    Get,
    /// Change settings; only the options given change.
    Set {
        /// Root mode for `jidr device create` without `--root`.
        #[arg(long, value_enum)]
        default_root: Option<RootModeArg>,
        /// Save a quick-boot snapshot when a device stops (from its next start).
        #[arg(long)]
        quick_boot: Option<bool>,
        /// Share of host RAM running devices may use, 10–100 percent.
        #[arg(long, value_parser = clap::value_parser!(u8).range(10..=100))]
        budget_percent: Option<u8>,
        /// Hibernate running devices idle this many minutes (0 = off).
        #[arg(long)]
        auto_hibernate_minutes: Option<u32>,
    },
}

#[derive(Debug, Subcommand)]
pub enum CacheCmd {
    /// Cached downloads.
    List,
    /// Add a local file (e.g. a Magisk APK) for offline use.
    Import {
        path: PathBuf,
        /// Expected SHA-256 of the file.
        #[arg(long)]
        sha256: Option<String>,
    },
    /// Delete cached downloads.
    Prune {
        /// Only those older than this many days.
        #[arg(long)]
        older_than_days: Option<u64>,
    },
}

#[derive(Debug, Subcommand)]
pub enum ImageCmd {
    /// System images for this Mac, installed or available.
    List {
        /// Only installed images (works offline).
        #[arg(long)]
        installed: bool,
    },
    /// Download and install a system image (and the emulator if needed).
    Install {
        image: String,
        /// Accept the licenses shown (prints each license id and hash).
        #[arg(long)]
        accept_licenses: bool,
    },
}

#[derive(Debug, Subcommand)]
pub enum ProfileCmd {
    /// Built-in hardware profiles.
    List,
}

#[derive(Debug, Subcommand)]
pub enum SdkCmd {
    /// Where the SDKs are, emulator version, hypervisor, free disk.
    Status,
    /// Licenses still to accept for these packages, with their text.
    Licenses { packages: Vec<String> },
    /// Install SDK packages into Jidr's private SDK.
    Install {
        packages: Vec<String>,
        /// Accept the licenses shown (prints each license id and hash).
        #[arg(long)]
        accept_licenses: bool,
    },
}

#[derive(Debug, Subcommand)]
pub enum RootCmd {
    /// Switch root mode (cold boots the device).
    Set {
        device: String,
        #[arg(value_enum)]
        mode: RootModeArg,
        /// Use this Magisk APK instead of downloading the pinned release (offline, D30).
        #[arg(long)]
        magisk: Option<PathBuf>,
    },
    /// Writable /system on a shell-root (adb) device: `on` boots with
    /// `-writable-system` and runs `adb remount`; `off` goes back to the
    /// read-only stock system image. Cold boots a running device.
    WritableSystem {
        device: String,
        #[arg(value_enum)]
        state: OnOff,
    },
    /// Check that root works.
    Verify { device: String },
    /// Root mode and health.
    Status { device: String },
    /// Start the rooted on-device helper if needed and show its version, uid
    /// and SELinux context (ADR-0003).
    Helper { device: String },
    /// Grant every app root without a prompt on a Full-root device (D22, off by default).
    AutoGrant {
        device: String,
        /// `on` grants every app root; `off` restores Magisk's prompt.
        #[arg(value_enum)]
        state: OnOff,
    },
}

#[derive(Debug, Subcommand)]
pub enum SnapshotCmd {
    List { device: String },
    Save { device: String, name: String },
    Load { device: String, name: String },
    Delete { device: String, name: String },
}

#[derive(Debug, Subcommand)]
pub enum CertCmd {
    /// Trust a CA certificate (PEM or DER) as a system cert; restarts the device.
    Install { device: String, file: PathBuf },
    /// The CA certificates Jidr added and whether each is trusted.
    List { device: String },
    /// Check that every one of them is in the system store apps read.
    Verify { device: String },
    /// Remove one (by file name, subject hash or SHA-256) or `--all`; restarts the device.
    Remove {
        device: String,
        #[arg(required_unless_present = "all", conflicts_with = "all")]
        cert: Option<String>,
        #[arg(long)]
        all: bool,
    },
}

#[derive(Debug, Subcommand)]
pub enum ProxyCmd {
    /// Send the device's HTTP(S) traffic through `[host:]port`; the host
    /// defaults to 10.0.2.2, this Mac as the emulator sees it.
    Set { device: String, proxy: String },
    /// Remove the proxy.
    Clear { device: String },
    /// The current proxy.
    Show { device: String },
}

#[derive(Debug, Subcommand)]
pub enum NetworkCmd {
    /// Drop PERCENT % of the device's outgoing packets on Wi-Fi and mobile
    /// data (0 removes it); without PERCENT, show the current loss. Needs
    /// Shell or Full root; kept across cold boots.
    Loss {
        device: String,
        percent: Option<f64>,
    },
    /// Make DNS slow or fail on Wi-Fi and mobile data: `slow` delays each
    /// query (--delay-ms, default 3000), `fail` drops them all so lookups
    /// time out, `normal` undoes it; without MODE, show it. Needs Shell or
    /// Full root; kept across cold boots.
    Dns {
        device: String,
        mode: Option<DnsArg>,
        /// The delay of each DNS query in ms (1–30000); implies `slow`.
        #[arg(long, value_name = "MS")]
        delay_ms: Option<u32>,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum DnsArg {
    Normal,
    Slow,
    Fail,
}

impl DnsArg {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Slow => "slow",
            Self::Fail => "fail",
        }
    }
}

#[derive(Debug, Subcommand)]
pub enum AudioCmd {
    /// Play a WAV file (8–48 kHz, mono or stereo) as the microphone of a
    /// running device; waits until it ends.
    Inject {
        device: String,
        file: PathBuf,
        /// Start again at the end, until `jidr audio stop`.
        #[arg(long = "loop")]
        repeat: bool,
        /// Headerless 16-bit little-endian PCM: its sample rate in Hz.
        #[arg(long)]
        rate: Option<u32>,
        /// Headerless PCM: 1 (default) or 2 channels.
        #[arg(long, requires = "rate")]
        channels: Option<u16>,
        /// Print the job id and return; stop it with `jidr audio stop`.
        #[arg(long)]
        no_wait: bool,
    },
    /// Stop the audio playing into the microphone.
    Stop { device: String },
}

#[derive(Debug, Subcommand)]
pub enum JobsCmd {
    /// Running jobs.
    List {
        /// Include finished jobs.
        #[arg(long)]
        all: bool,
    },
    /// Wait for a job and print its result.
    Wait { job: String },
    /// Cancel a job.
    Cancel { job: String },
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    /// Regression: `-f` was claimed by both `--filter` and `--follow`; clap
    /// panics on that at runtime in debug builds. Checks every subcommand.
    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn root_writable_system_takes_on_or_off() {
        let cli = Cli::try_parse_from(["jidr", "root", "writable-system", "Phone", "on"]).unwrap();
        match cli.cmd {
            Cmd::Root {
                cmd: RootCmd::WritableSystem { device, state },
            } => {
                assert_eq!(device, "Phone");
                assert!(state.enabled());
            }
            other => panic!("{other:?}"),
        }
        assert!(
            Cli::try_parse_from(["jidr", "root", "writable-system", "Phone", "maybe"]).is_err()
        );
    }

    #[test]
    fn audio_inject_flags() {
        let cli = Cli::try_parse_from([
            "jidr",
            "audio",
            "inject",
            "Phone",
            "a.pcm",
            "--rate",
            "16000",
            "--channels",
            "2",
            "--loop",
        ])
        .unwrap();
        let Cmd::Audio {
            cmd:
                AudioCmd::Inject {
                    rate,
                    channels,
                    repeat,
                    no_wait,
                    ..
                },
        } = cli.cmd
        else {
            panic!("not audio inject");
        };
        assert_eq!(
            (rate, channels, repeat, no_wait),
            (Some(16_000), Some(2), true, false)
        );
        // --channels describes headerless PCM, so it needs --rate.
        assert!(
            Cli::try_parse_from(["jidr", "audio", "inject", "P", "a.wav", "--channels", "2"])
                .is_err()
        );
        assert!(Cli::try_parse_from(["jidr", "audio", "stop", "P"]).is_ok());
    }

    #[test]
    fn durations() {
        assert_eq!(parse_duration("2h"), Ok(7200));
        assert_eq!(parse_duration("30m"), Ok(1800));
        assert_eq!(parse_duration("90s"), Ok(90));
        assert_eq!(parse_duration("45"), Ok(45));
        assert_eq!(parse_duration("1d"), Ok(86_400));
        for bad in ["", "h", "0m", "-1h", "2 hours"] {
            assert!(parse_duration(bad).is_err(), "{bad:?}");
        }
    }

    /// Regression: docs/root.md shows `jidr root auto-grant <device> on|off`,
    /// which used to fail to parse.
    #[test]
    fn root_auto_grant_parses() {
        for (arg, want) in [("on", true), ("off", false)] {
            let cli = Cli::try_parse_from(["jidr", "root", "auto-grant", "Phone", arg]).unwrap();
            match cli.cmd {
                Cmd::Root {
                    cmd: RootCmd::AutoGrant { device, state },
                } => {
                    assert_eq!(device, "Phone");
                    assert_eq!(state.enabled(), want);
                }
                other => panic!("{other:?}"),
            }
        }
        for bad in [vec!["Phone"], vec!["Phone", "maybe"], vec!["Phone", "true"]] {
            let args = ["jidr", "root", "auto-grant"].into_iter().chain(bad);
            assert!(Cli::try_parse_from(args).is_err());
        }
    }
}
