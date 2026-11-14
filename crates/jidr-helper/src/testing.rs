//! A fake guest for offline tests (not used by the product): the real
//! request loop ([`Server`]) on a host TCP port with fake `ip`, `tc`,
//! `cmd package`, `stop`/`start` and files, and an `adb`
//! script whose `forward` points at it. Like a real device, nothing answers
//! until `jidr-helper start` has run through `adb shell`, and a "reboot"
//! stops the helper again (keeping what lives in `/data`).

use std::collections::HashMap;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use crate::identity::{CHMOD, CHOWN, CMD, PIDOF, START, STOP};
use crate::netem::{IP, Output, Run, TC};
use crate::rpc::{PROTOCOL, Pong};
use crate::server::{Next, Server};

/// What the fake guest's kernel and helper look like.
#[derive(Debug)]
pub struct GuestState {
    /// Root qdisc line per interface, as `tc qdisc show` prints it.
    pub qdiscs: HashMap<String, String>,
    /// `ip -4 route show table all`.
    pub routes: String,
    /// Whether the kernel has netem.
    pub netem: bool,
    /// The running helper's uid (0 when started as root).
    pub uid: u32,
    /// Files the helper wrote or reads, by path (kept across reboots).
    pub files: HashMap<String, Vec<u8>>,
    /// `cmd package list packages -U`.
    pub packages: String,
    /// system_server's pid; `None` while the framework is stopped.
    pub system_server: Option<u32>,
    /// The IMEI telephony reports.
    pub imei: String,
    /// Where the fake `adb` reads what the device reports.
    dir: Option<std::path::PathBuf>,
}

impl GuestState {
    /// Writes what the fake `adb` shows: the IMEI (as a `service call`
    /// parcel) and system_server's pid. (The global Android ID is the file
    /// `android_id`, which the fake `adb` reads and writes.)
    fn sync(&self) {
        let Some(dir) = &self.dir else { return };
        let _ = std::fs::write(dir.join("imei"), parcel(&self.imei));
        let pid = self
            .system_server
            .map(|p| p.to_string())
            .unwrap_or_default();
        let _ = std::fs::write(dir.join("system_server"), pid);
    }
}

/// `service call`'s dump of a parcel holding `text` (UTF-16).
fn parcel(text: &str) -> String {
    let dots: String = text.chars().flat_map(|c| ['.', c]).collect();
    format!("Result: Parcel(00000000 0000000f 00000000 '........{dots}')\n")
}

impl Default for GuestState {
    fn default() -> Self {
        Self {
            qdiscs: HashMap::from([
                ("wlan0".into(), "qdisc mq 0: root".into()),
                ("radio0".into(), "qdisc pfifo_fast 0: root refcnt 2".into()),
            ]),
            routes: "default via 10.0.2.2 dev wlan0 table wlan0 proto static\n\
                     default via 10.0.2.2 dev radio0 table 1022 proto static\n"
                .into(),
            netem: true,
            uid: 0,
            files: HashMap::new(),
            packages: "package:android uid:1000\n\
                       package:com.android.shell uid:2000\n\
                       package:com.example.app uid:10123\n"
                .into(),
            system_server: Some(500),
            imei: "867400022047199".into(),
            dir: None,
        }
    }
}

#[derive(Clone, Default)]
struct Shared(Arc<Mutex<GuestState>>);

impl Shared {
    fn lock(&self) -> MutexGuard<'_, GuestState> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Run for Shared {
    fn run(&self, program: &str, args: &[&str]) -> std::io::Result<Output> {
        let mut s = self.lock();
        let ok = |stdout: String| Output {
            stdout,
            ..Output::default()
        };
        let out = match (program, args) {
            (CMD, ["package", "list", "packages", "-U", ..]) => ok(s.packages.clone()),
            (STOP, []) => {
                s.system_server = None;
                ok(String::new())
            }
            (START, []) => {
                s.system_server = Some(501);
                ok(String::new())
            }
            (PIDOF, ["system_server"]) => match s.system_server {
                Some(pid) => ok(format!("{pid}\n")),
                None => Output {
                    status: 1,
                    ..Output::default()
                },
            },
            (CHOWN | CHMOD, _) => ok(String::new()),
            (IP, _) => ok(s.routes.clone()),
            (TC, ["qdisc", "show", "dev", dev]) => {
                ok(s.qdiscs.get(*dev).cloned().unwrap_or_default())
            }
            (TC, ["qdisc", "replace", "dev", dev, "root", "netem", "loss", pct]) if s.netem => {
                let line = format!("qdisc netem 8001: root refcnt 2 limit 1000 loss {pct}");
                s.qdiscs.insert(dev.to_string(), line);
                ok(String::new())
            }
            (TC, ["qdisc", "replace", ..]) => Output {
                status: 2,
                stderr: "Unknown qdisc \"netem\", hence option \"loss\" is unparsable".into(),
                ..Output::default()
            },
            (TC, ["qdisc", "del", "dev", dev, "root"]) => {
                s.qdiscs.insert(dev.to_string(), "qdisc mq 0: root".into());
                ok(String::new())
            }
            _ => Output {
                status: 1,
                stderr: format!("fake guest: no {program} {}", args.join(" ")),
                ..Output::default()
            },
        };
        s.sync();
        Ok(out)
    }

    fn read_file(&self, path: &str) -> std::io::Result<Vec<u8>> {
        self.lock()
            .files
            .get(path)
            .cloned()
            .ok_or_else(|| std::io::Error::from(std::io::ErrorKind::NotFound))
    }

    fn write_file(&self, path: &str, contents: &[u8]) -> std::io::Result<()> {
        self.lock()
            .files
            .insert(path.to_string(), contents.to_vec());
        Ok(())
    }

    fn remove_file(&self, path: &str) -> std::io::Result<()> {
        self.lock().files.remove(path);
        Ok(())
    }
}

/// The `adb` the fake guest answers to. `forward` makes one forward per
/// serial to the guest's port; `push` keeps the file; `shell` knows the
/// helper's start command (which replaces an older build),
/// `getprop ro.product.cpu.abi` and `id`; `root` works.
const FAKE_ADB: &str = r#"#!/bin/sh
dir="$(dirname "$0")/guest"
serial=none
while [ "$1" = -s ]; do serial="$2"; shift 2; done
case "$1" in
  root) echo "restarting adbd as root";;
  unroot) echo "restarting adbd as non root";;
  forward)
    if [ "$2" = --list ]; then cat "$dir/forwards" 2>/dev/null; true
    else echo "$serial tcp:@PORT@ ${3}" >> "$dir/forwards"; echo @PORT@; fi;;
  push) cp "$2" "$dir/pushed"; echo 1 >> "$dir/pushes"; echo "1 file pushed";;
  shell)
    case "$2" in
      "getprop ro.product.cpu.abi") echo arm64-v8a;;
      "settings get secure android_id") cat "$dir/android_id";;
      "settings put secure android_id "*) echo "${2##* }" > "$dir/android_id";;
      "service call iphonesubinfo 1 s16 com.android.shell") cat "$dir/imei";;
      "pidof system_server") cat "$dir/system_server";;
      "getprop service.bootanim.exit") echo 1;;
      id*) echo "uid=0(root) gid=0(root) groups=0(root)";;
      *"/data/local/tmp/jidr-helper start"*)
        rm -f "$dir/older-build"
        touch "$dir/started"; echo 1 >> "$dir/starts"; echo '{"pid":1}';;
      *) echo ok;;
    esac;;
  *) echo ok;;
esac
"#;

/// A fake guest serving on a host port until the test ends.
pub struct FakeGuest {
    /// Its working files: `forwards`, `pushed`, `pushes`, `started`, `starts`.
    pub dir: PathBuf,
    /// A helper "build" for each arch, as `<helper_dir>/<arch>/jidr-helper`;
    /// point `JIDR_HELPER_DIR` here.
    pub helper_dir: PathBuf,
    pub port: u16,
    state: Shared,
}

impl FakeGuest {
    /// Writes the fake `adb` over `adb` (keeping its working files next to
    /// it, in `guest/`) and a fake helper build under `root`, and starts
    /// serving.
    pub fn start(adb: &Path, root: &Path) -> std::io::Result<FakeGuest> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let port = listener.local_addr()?.port();
        let dir = adb
            .parent()
            .ok_or_else(|| std::io::Error::other("adb has no folder"))?
            .join("guest");
        std::fs::create_dir_all(&dir)?;
        std::fs::write(adb, FAKE_ADB.replace("@PORT@", &port.to_string()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(adb, std::fs::Permissions::from_mode(0o755))?;
        }
        let helper_dir = root.join("helper");
        for arch in ["aarch64", "x86_64"] {
            std::fs::create_dir_all(helper_dir.join(arch))?;
            std::fs::write(
                helper_dir.join(arch).join("jidr-helper"),
                format!("\x7fELF fake jidr-helper for {arch}"),
            )?;
        }
        let state = Shared::default();
        std::fs::write(dir.join("android_id"), "1f3842600e7f0f09\n")?;
        state.lock().dir = Some(dir.clone());
        state.lock().sync();
        let (st, started) = (state.clone(), dir.join("started"));
        let (pushed, older) = (dir.join("pushed"), dir.join("older-build"));
        std::thread::spawn(move || {
            for conn in listener.incoming() {
                let Ok(stream) = conn else { continue };
                // adb accepts the forward and closes it when nothing
                // listens on the device.
                if !started.exists() {
                    continue;
                }
                let pong = {
                    let s = st.lock();
                    Pong {
                        version: crate::VERSION.into(),
                        protocol: PROTOCOL,
                        sha256: std::fs::read_to_string(&older)
                            .unwrap_or_else(|_| crate::sha256_file(&pushed).unwrap_or_default()),
                        pid: 4242,
                        uid: s.uid,
                        selinux: Some("u:r:su:s0".into()),
                        arch: "aarch64".into(),
                    }
                };
                let server = Server::new(pong, st.clone());
                let started = started.clone();
                std::thread::spawn(move || {
                    if let Ok(Next::Exit) = server.serve(&stream) {
                        let _ = std::fs::remove_file(&started);
                    }
                });
            }
        });
        Ok(FakeGuest {
            dir,
            helper_dir,
            port,
            state,
        })
    }

    /// The guest's state, to change or check it.
    pub fn state(&self) -> MutexGuard<'_, GuestState> {
        self.state.lock()
    }

    /// A reboot: the helper is gone, and with it the kernel's shaping;
    /// `/data` (the Android ID, files) stays.
    pub fn reboot(&self) {
        let _ = std::fs::remove_file(self.dir.join("started"));
        let mut s = self.state();
        let fresh = GuestState {
            files: std::mem::take(&mut s.files),
            packages: std::mem::take(&mut s.packages),
            dir: s.dir.take(),
            ..GuestState::default()
        };
        *s = fresh;
        s.sync();
    }

    /// The SSAID table the helper wrote, as text.
    pub fn ssaid(&self) -> Option<String> {
        self.state()
            .files
            .get(crate::identity::SSAID_FILE)
            .map(|b| String::from_utf8_lossy(b).into_owned())
    }

    /// An older helper build is running, until `jidr-helper start`
    /// replaces it.
    pub fn run_older_build(&self) -> std::io::Result<()> {
        std::fs::write(self.dir.join("older-build"), "an older build")
    }

    /// How many times `jidr-helper start` ran, and the binary was pushed.
    pub fn starts(&self) -> usize {
        count(&self.dir.join("starts"))
    }

    pub fn pushes(&self) -> usize {
        count(&self.dir.join("pushes"))
    }
}

fn count(path: &Path) -> usize {
    std::fs::read_to_string(path)
        .map(|s| s.lines().count())
        .unwrap_or(0)
}
