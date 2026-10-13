//! Command-line definition (hand-written clap; every command is a thin call
//! over the daemon protocol).

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};

#[derive(Debug, Parser)]
#[command(
    name = "jidr",
    version,
    about = "Android emulators with one-click root, for people, CI and AI agents",
    after_help = "Exit codes: 0 ok · 1 unexpected · 2 usage · 3 not found · 4 busy · 5 boot timeout · 6 sdk · 7 root failed · 8 daemon stale"
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
    /// Flash a zip onto /system (not available yet).
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
    /// The download cache (Magisk APKs and other pinned files, by SHA-256).
    Cache {
        #[command(subcommand)]
        cmd: CacheCmd,
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
    /// List every protocol command (the same list the GUI and MCP see).
    Commands,
    /// Call any protocol command directly: `jidr call device.list` or
    /// `jidr call device.start '{"device":"Phone"}'`.
    Call {
        method: String,
        /// Params as a JSON object.
        params: Option<String>,
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
        /// Root mode (default: adb where the image allows it).
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
    },
    /// Stop devices (saves their quick-boot state).
    Stop {
        #[command(flatten)]
        sel: Selection,
    },
    /// List devices.
    List,
    /// Settings, status and disk usage of a device.
    Details { device: String },
    /// Copy a stopped device.
    Clone { device: String, name: String },
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
    /// Check that root works.
    Verify { device: String },
    /// Root mode and health.
    Status { device: String },
}

#[derive(Debug, Subcommand)]
pub enum SnapshotCmd {
    List { device: String },
    Save { device: String, name: String },
    Load { device: String, name: String },
    Delete { device: String, name: String },
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
}
