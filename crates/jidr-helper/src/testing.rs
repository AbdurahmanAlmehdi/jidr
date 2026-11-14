//! A fake guest for offline tests (not used by the product): the real
//! request loop ([`Server`]) on a host TCP port with fake `ip` and `tc`, and
//! an `adb` script whose `forward` points at it. Like a real device, nothing
//! answers until `jidr-helper start` has run through `adb shell`, and a
//! "reboot" stops the helper again.

use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use crate::fake::Kernel;
use crate::netem::{Output, Run};
use crate::rpc::{PROTOCOL, Pong};
use crate::server::{Next, Server};

/// What the fake guest's kernel and helper look like.
#[derive(Debug, Default)]
pub struct GuestState {
    /// Routes, qdiscs and filters, as `ip` and `tc` see them.
    pub kernel: Kernel,
    /// The running helper's uid (0 when started as root).
    pub uid: u32,
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
        Ok(self.lock().kernel.run(program, args))
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

    /// A reboot: the helper is gone and the kernel's shaping with it.
    pub fn reboot(&self) {
        let _ = std::fs::remove_file(self.dir.join("started"));
        *self.state() = GuestState::default();
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
