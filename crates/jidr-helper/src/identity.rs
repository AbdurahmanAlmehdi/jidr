//! The Android ID, as apps see it (#19).
//!
//! Since Android 8, an app that reads `Settings.Secure.ANDROID_ID` doesn't
//! get the global `android_id` setting: `SettingsProvider` answers callers
//! with an app uid from a per-app table, the SSAID, kept in
//! `/data/system/users/0/settings_ssaid.xml` (one entry per app uid, plus
//! the `userkey` new entries are derived from). Only system and shell
//! callers read the global value.
//!
//! Setting the Android ID therefore changes both. The global value is an
//! ordinary setting the daemon writes once the framework is back (a
//! `settings put` just before `stop` can be lost: `SettingsProvider`
//! writes its files a moment later). The SSAID is the helper's part. There
//! is no API that writes the SSAID table, and `SettingsProvider`
//! keeps it in memory and rewrites the file from there, so the helper stops
//! the Android framework (`stop`: zygote, system_server and every app),
//! rewrites the file, and starts it again (`start`), which reloads it. The
//! file is plain XML on Android 8–11 and binary XML (ABX) from Android 12;
//! `SettingsProvider` reads both, so the helper reads the old one with
//! Android's own `abx2xml` when needed and writes plain XML.
//!
//! An app installed after the change gets its own derived ID again, until
//! the next change. Uninstalling an app drops its entry.
//!
//! Sources: AOSP `SettingsProvider.java` (`isNewSsaidSetting`,
//! `getSsaidSettingLocked`, `generateSsaidLocked`) and `SettingsState.java`
//! (the file's tags and attributes, `Xml.resolvePullParser`); see
//! `docs/clean-room.md`.

use crate::netem::{Output, Run};
use crate::rpc::{AndroidIdSet, AndroidIdSetResult, Error};

pub const CMD: &str = "/system/bin/cmd";
pub const STOP: &str = "/system/bin/stop";
pub const START: &str = "/system/bin/start";
pub const PIDOF: &str = "/system/bin/pidof";
pub const ABX2XML: &str = "/system/bin/abx2xml";
pub const CHOWN: &str = "/system/bin/chown";
pub const CHMOD: &str = "/system/bin/chmod";

/// The SSAID table of user 0.
pub const SSAID_FILE: &str = "/data/system/users/0/settings_ssaid.xml";

/// The key every new SSAID is derived from; kept as it is.
const USER_KEY: &str = "userkey";

/// App uids of user 0 (`Process.FIRST_APPLICATION_UID`..`LAST_APPLICATION_UID`):
/// only these callers read the SSAID.
const APP_UIDS: std::ops::RangeInclusive<u32> = 10_000..=19_999;

/// 16 hex digits, as Android generates it; returned in lower case.
pub fn check_android_id(id: &str) -> Result<String, Error> {
    if id.len() == 16 && id.chars().all(|c| c.is_ascii_hexdigit()) {
        Ok(id.to_ascii_lowercase())
    } else {
        Err(Error::usage(format!(
            "android_id: {id:?} isn't 16 hexadecimal digits"
        )))
    }
}

/// The apps of `cmd package list packages -U`: `package:<name> uid:<uid>`,
/// one per app uid (apps sharing a uid share an SSAID entry).
pub fn parse_packages(list: &str) -> Vec<(String, u32)> {
    let mut apps: Vec<(String, u32)> = Vec::new();
    for line in list.lines() {
        let mut words = line.split_whitespace();
        let (Some(pkg), Some(uid)) = (
            words.next().and_then(|w| w.strip_prefix("package:")),
            words
                .find_map(|w| w.strip_prefix("uid:"))
                .and_then(|u| u.split(',').next())
                .and_then(|u| u.parse::<u32>().ok()),
        ) else {
            continue;
        };
        if APP_UIDS.contains(&uid) && !apps.iter().any(|(_, u)| *u == uid) {
            apps.push((pkg.to_string(), uid));
        }
    }
    apps.sort_by_key(|(_, uid)| *uid);
    apps
}

/// One `<setting …/>` element's attributes, in order.
type Attrs = Vec<(String, String)>;

/// What the helper keeps from the old table: the `<settings version>` and
/// the `userkey` entry.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Table {
    pub version: Option<String>,
    pub user_key: Option<Attrs>,
}

fn unescape(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&#10;", "\n")
        .replace("&amp;", "&")
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// The `name="value"` pairs of a start tag's body.
fn attrs(body: &str) -> Attrs {
    let mut out = Vec::new();
    let mut rest = body;
    while let Some(eq) = rest.find('=') {
        let name = rest[..eq].trim().to_string();
        let after = rest[eq + 1..].trim_start();
        let Some(quote) = after.chars().next().filter(|c| *c == '"' || *c == '\'') else {
            break;
        };
        let Some(end) = after[1..].find(quote) else {
            break;
        };
        out.push((name, unescape(&after[1..1 + end])));
        rest = &after[end + 2..];
    }
    out
}

fn attr<'a>(attrs: &'a Attrs, name: &str) -> Option<&'a str> {
    attrs
        .iter()
        .find(|(n, _)| n == name)
        .map(|(_, v)| v.as_str())
}

/// Reads the parts of a plain-XML SSAID table the helper keeps.
pub fn parse_table(xml: &str) -> Table {
    let mut table = Table::default();
    let mut rest = xml;
    while let Some(start) = rest.find('<') {
        rest = &rest[start + 1..];
        let end = rest.find('>').unwrap_or(rest.len());
        let tag = rest[..end].trim_end_matches('/');
        if let Some(body) = tag.strip_prefix("settings ") {
            table.version = attr(&attrs(body), "version").map(str::to_owned);
        } else if let Some(body) = tag.strip_prefix("setting ") {
            let a = attrs(body);
            if attr(&a, "name") == Some(USER_KEY) {
                table.user_key = Some(a);
            }
        }
        rest = &rest[end..];
    }
    table
}

/// The new table: the old `userkey` (if any; `SettingsProvider` makes one
/// when it needs it), and `android_id` for every app.
pub fn render_table(table: &Table, apps: &[(String, u32)], android_id: &str) -> String {
    let mut out = String::from("<?xml version='1.0' encoding='utf-8' standalone='yes' ?>\n");
    out.push_str(&format!(
        "<settings version=\"{}\">\n",
        escape(table.version.as_deref().unwrap_or("-1"))
    ));
    let mut next_id = 0u64;
    if let Some(key) = &table.user_key {
        out.push_str("  <setting");
        for (name, value) in key {
            out.push_str(&format!(" {name}=\"{}\"", escape(value)));
        }
        out.push_str(" />\n");
        next_id = attr(key, "id")
            .and_then(|id| id.parse::<u64>().ok())
            .map_or(0, |id| id + 1);
    }
    for (i, (pkg, uid)) in apps.iter().enumerate() {
        out.push_str(&format!(
            "  <setting id=\"{}\" name=\"{uid}\" value=\"{android_id}\" package=\"{}\" />\n",
            next_id + i as u64,
            escape(pkg)
        ));
    }
    out.push_str("</settings>\n");
    out
}

fn run(r: &dyn Run, program: &str, args: &[&str]) -> Result<Output, Error> {
    let out = r.run(program, args).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            Error::unsupported(format!("{program} isn't on this image"))
        } else {
            Error::failed(format!("running {program}: {e}"))
        }
    })?;
    if out.status == 0 {
        Ok(out)
    } else {
        Err(Error::failed(format!(
            "{program} {}: {}",
            args.join(" "),
            format!("{}{}", out.stderr, out.stdout).trim()
        )))
    }
}

/// Reads the SSAID table as plain XML (converting ABX with `abx2xml`);
/// `None` when there is none yet (`SettingsProvider` creates it when an app
/// first reads its ID).
fn read_table(r: &dyn Run) -> Result<Option<Table>, Error> {
    let bytes = match r.read_file(SSAID_FILE) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(Error::failed(format!("reading {SSAID_FILE}: {e}"))),
    };
    let xml = if bytes.starts_with(b"ABX") {
        run(r, ABX2XML, &[SSAID_FILE, "-"])?.stdout
    } else {
        String::from_utf8_lossy(&bytes).into_owned()
    };
    Ok(Some(parse_table(&xml)))
}

/// Waits until system_server is gone after `stop` (up to 30 s).
fn wait_framework_stopped(r: &dyn Run) -> Result<(), Error> {
    for _ in 0..150 {
        let gone = r
            .run(PIDOF, &["system_server"])
            .map(|o| o.status != 0 || o.stdout.trim().is_empty())
            .unwrap_or(false);
        if gone {
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
    Err(Error::failed("system_server didn't stop within 30 s"))
}

/// While the framework is stopped: rewrites the table.
fn rewrite(r: &dyn Run, apps: &[(String, u32)], android_id: &str) -> Result<(), Error> {
    wait_framework_stopped(r)?;
    let table = read_table(r)?;
    // AtomicFile would restore a leftover backup over the new file.
    for leftover in [".bak", ".new"] {
        let _ = r.remove_file(&format!("{SSAID_FILE}{leftover}"));
    }
    // In place, so an existing file keeps its owner, mode and SELinux label;
    // a new one gets the folder's label (`system_data_file`) and is handed
    // to system_server's user like the other settings files.
    let xml = render_table(&table.clone().unwrap_or_default(), apps, android_id);
    r.write_file(SSAID_FILE, xml.as_bytes())
        .map_err(|e| Error::failed(format!("writing {SSAID_FILE}: {e}")))?;
    if table.is_none() {
        run(r, CHOWN, &["system:system", SSAID_FILE])?;
        run(r, CHMOD, &["600", SSAID_FILE])?;
    }
    Ok(())
}

/// Gives every installed app the ID as its SSAID, restarting the framework.
pub fn set(r: &dyn Run, p: &AndroidIdSet) -> Result<AndroidIdSetResult, Error> {
    let android_id = check_android_id(&p.android_id)?;
    // Before stopping anything: the table must be readable (or absent).
    match r.read_file(SSAID_FILE) {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(Error::failed(format!("reading {SSAID_FILE}: {e}"))),
    }
    let apps = parse_packages(
        &run(
            r,
            CMD,
            &["package", "list", "packages", "-U", "--user", "0"],
        )?
        .stdout,
    );
    if apps.is_empty() {
        return Err(Error::failed("cmd package listed no apps"));
    }
    run(r, STOP, &[])?;
    let rewritten = rewrite(r, &apps, &android_id);
    // Whatever happened, the framework comes back.
    run(r, START, &[])?;
    rewritten?;
    Ok(AndroidIdSetResult {
        android_id,
        apps: apps.len() as u32,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn android_ids_are_16_hex_digits() {
        assert_eq!(
            check_android_id("0123456789ABCDEF").unwrap(),
            "0123456789abcdef"
        );
        for bad in [
            "",
            "123",
            "0123456789abcdefa",
            "0123456789abcdeg",
            "0123456789abcde ",
        ] {
            assert_eq!(
                check_android_id(bad).unwrap_err().code,
                crate::rpc::ErrorCode::Usage,
                "{bad:?}"
            );
        }
    }

    #[test]
    fn packages_are_app_uids_once_each() {
        let list = "package:com.android.shell uid:2000\n\
                    package:android uid:1000\n\
                    package:com.example.b uid:10123\n\
                    package:com.example.a uid:10100\n\
                    package:com.example.shared uid:10123\n\
                    package:com.example.multi uid:10200,1010200\n\
                    garbage\n";
        assert_eq!(
            parse_packages(list),
            vec![
                ("com.example.a".into(), 10100),
                ("com.example.b".into(), 10123),
                ("com.example.multi".into(), 10200),
            ]
        );
    }

    const TABLE: &str = r#"<?xml version='1.0' encoding='UTF-8' standalone='yes' ?>
<settings version="-1">
  <setting id="0" name="userkey" value="ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789" package="android" defaultValue="x" defaultSysSet="true" />
  <setting id="1" name="10100" value="1111111111111111" package="com.example.a" />
  <setting id="3" name="10999" value="2222222222222222" package="com.gone" />
</settings>
"#;

    #[test]
    fn the_table_keeps_its_user_key_and_gives_every_app_the_id() {
        let table = parse_table(TABLE);
        assert_eq!(table.version.as_deref(), Some("-1"));
        let key = table.user_key.clone().unwrap();
        assert_eq!(attr(&key, "package"), Some("android"));
        let apps = vec![("com.example.a".into(), 10100), ("com.x&y".into(), 10123)];
        let xml = render_table(&table, &apps, "0123456789abcdef");
        assert!(xml.contains(r#"<setting id="0" name="userkey" value="ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789" package="android" defaultValue="x" defaultSysSet="true" />"#), "{xml}");
        assert!(xml.contains(r#"<setting id="1" name="10100" value="0123456789abcdef" package="com.example.a" />"#), "{xml}");
        assert!(
            xml.contains(
                r#"<setting id="2" name="10123" value="0123456789abcdef" package="com.x&amp;y" />"#
            ),
            "{xml}"
        );
        assert!(!xml.contains("10999"));
        // What it writes reads back the same.
        assert_eq!(parse_table(&xml), table);
    }

    #[test]
    fn single_quotes_and_escapes_are_read() {
        let t = parse_table(
            "<settings version='3'><setting name='userkey' value='a&amp;b'/></settings>",
        );
        assert_eq!(t.version.as_deref(), Some("3"));
        assert_eq!(attr(t.user_key.as_ref().unwrap(), "value"), Some("a&b"));
    }
}
