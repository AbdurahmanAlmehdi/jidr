//! The D22 "auto-grant all" toggle: Magisk's superuser *automatic response*.
//!
//! Magisk's su grant decision (native `uid_granted_root`) only knows per-uid
//! `policies` rows; the *global* "grant every app without a prompt" behaviour
//! lives in the Magisk app, which auto-answers superuser requests when its
//! `su_auto_response` preference is `2` (allow) and shows the normal prompt at
//! `0`. That preference is a SharedPreferences value, not a `magisk --sqlite`
//! setting, so Jidr edits just that one key over `adb root` and reads it back.
//!
//! Learned read-only from Magisk v30.7 (`Config.kt`, `SuRequestHandler.kt`,
//! `PreferenceConfig.kt`); no GPL code was copied (docs/clean-room.md).

use jidr_adb::Adb;
use jidr_core::{JidrError, JidrResult};

use super::grant;

/// The Magisk app's SharedPreferences file (device-protected storage).
const PREF_FILE: &str =
    "/data/user_de/0/com.topjohnwu.magisk/shared_prefs/com.topjohnwu.magisk_preferences.xml";
const PREF_DIR: &str = "/data/user_de/0/com.topjohnwu.magisk/shared_prefs";

/// The Magisk application id.
const MAGISK_PACKAGE: &str = "com.topjohnwu.magisk";

/// The preference key (`Config.Key.SU_AUTO_RESPONSE`).
const PREF_KEY: &str = "su_auto_response";

/// `Config.Value`: prompt (default) and auto-allow.
const SU_PROMPT: &str = "0";
const SU_AUTO_ALLOW: &str = "2";

/// Applies the D22 "auto-grant all" flag: sets Magisk's automatic response to
/// `allow` (enabled) or back to `prompt` (disabled), but only when the stored
/// preference actually differs — the value is read first, so a boot with the
/// flag off and no key present never touches the file. Runs as root and drops
/// back to the normal shell afterwards.
pub(crate) async fn apply(adb: &Adb, enabled: bool) -> JidrResult<()> {
    let desired = if enabled { SU_AUTO_ALLOW } else { SU_PROMPT };
    grant::ensure_adbd_root(adb).await?;
    let result = sync(adb, desired).await;
    let _ = grant::drop_adbd_root(adb).await;
    result
}

/// Reads the current value, writes only when it differs, then restarts the app
/// so its cached SharedPreferences reflect the change.
async fn sync(adb: &Adb, desired: &str) -> JidrResult<()> {
    let current = read_auto_response(adb).await?;
    // An absent key means Magisk's default: prompt (`0`).
    if current.as_deref().unwrap_or(SU_PROMPT) == desired {
        return Ok(());
    }
    let content = read_raw(adb).await?;
    let edited = edit_content(content.as_deref(), desired);
    write_raw(adb, &edited).await?;
    // SharedPreferences are cached in the app process; force-stop so the next
    // launch re-reads the file instead of writing the stale value back.
    force_stop(adb).await?;
    // Confirm the value stuck.
    let after = read_auto_response(adb).await?;
    if after.as_deref() != Some(desired) {
        return Err(JidrError::root_failed(format!(
            "Magisk's automatic response was not stored (prefs: {})",
            after.as_deref().unwrap_or("(absent)")
        )));
    }
    Ok(())
}

/// Reads the raw preferences file, or `None` when it does not exist yet.
async fn read_raw(adb: &Adb) -> JidrResult<Option<String>> {
    let (status, out) = adb.shell_status(&format!("cat {PREF_FILE}")).await?;
    if status != 0 { Ok(None) } else { Ok(Some(out)) }
}

/// Reads the `su_auto_response` value, or `None` when the file or key is absent.
async fn read_auto_response(adb: &Adb) -> JidrResult<Option<String>> {
    Ok(read_raw(adb).await?.as_deref().and_then(extract_value))
}

/// Rewrites the preferences keeping every other key: removes any existing
/// `su_auto_response` entry and appends the new one before `</map>`.
fn edit_content(current: Option<&str>, value: &str) -> String {
    let entry = format!("    <string name=\"{PREF_KEY}\">{value}</string>");
    let mut lines: Vec<&str> = match current {
        None => vec![
            "<?xml version=\"1.0\" encoding=\"utf-8\" standalone=\"yes\" ?>",
            "<map>",
        ],
        Some(content) => content
            .lines()
            .filter(|l| !l.contains(&format!("name=\"{PREF_KEY}\"")))
            .collect(),
    };
    // Drop a trailing `</map>` (the XML root close) so the entry sits inside it.
    while lines.last().is_some_and(|l| l.trim() == "</map>") {
        lines.pop();
    }
    // An empty SharedPreferences file is written self-closing (`<map />`, or
    // `<map/>`); turn that into an opening `<map>` so the appended entry and
    // `</map>` keep the document well-formed (otherwise Android discards the
    // file and the Magisk app loses every setting).
    if lines
        .last()
        .is_some_and(|l| matches!(l.trim(), "<map />" | "<map/>" | "<map></map>"))
    {
        let i = lines.len() - 1;
        lines[i] = "<map>";
    }
    lines.push(&entry);
    lines.push("</map>");
    let mut out = lines.join("\n");
    out.push('\n');
    out
}

/// Writes the file (as root), fixes the app's ownership and SELinux label.
async fn write_raw(adb: &Adb, content: &str) -> JidrResult<()> {
    let script = format!(
        "D={PREF_DIR}; P={PREF_FILE}; mkdir -p $D; printf '%s' {} > $P; \
         u=$(stat -c %u /data/data/{MAGISK_PACKAGE} 2>/dev/null); \
         if [ -n \"$u\" ]; then g=$(stat -c %g /data/data/{MAGISK_PACKAGE} 2>/dev/null); chown \"$u:$g\" $P; fi; \
         chmod 660 $P; restorecon $P 2>/dev/null",
        shell_single_quote(content)
    );
    adb.shell(&script)
        .await
        .map_err(|e| JidrError::root_failed(format!("writing Magisk's automatic response: {e}")))?;
    Ok(())
}

/// Force-stops the Magisk app so its cached SharedPreferences are re-read.
async fn force_stop(adb: &Adb) -> JidrResult<()> {
    adb.shell(&format!("am force-stop {MAGISK_PACKAGE}"))
        .await
        .map_err(|e| JidrError::root_failed(format!("stopping the Magisk app: {e}")))?;
    Ok(())
}

/// Single-quotes `s` for the device shell, escaping any embedded single quotes.
fn shell_single_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

fn extract_value(xml: &str) -> Option<String> {
    let marker = format!("name=\"{PREF_KEY}\">");
    let rest = xml.split_once(&marker)?.1;
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    (!digits.is_empty()).then_some(digits)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_match_magisk_30_7() {
        assert_eq!(SU_PROMPT, "0");
        assert_eq!(SU_AUTO_ALLOW, "2");
        assert_eq!(PREF_KEY, "su_auto_response");
    }

    #[test]
    fn edit_preserves_other_preferences_and_replaces_the_key() {
        let before = "<?xml version=\"1.0\" encoding=\"utf-8\" standalone=\"yes\" ?>\n\
                      <map>\n    <string name=\"denylist\">false</string>\n    \
                      <string name=\"su_auto_response\">0</string>\n    \
                      <string name=\"zygisk\">true</string>\n</map>\n";
        let after = edit_content(Some(before), "2");
        assert!(after.contains("<string name=\"denylist\">false</string>"));
        assert!(after.contains("<string name=\"zygisk\">true</string>"));
        assert!(after.contains("<string name=\"su_auto_response\">2</string>"));
        assert!(!after.contains("<string name=\"su_auto_response\">0</string>"));
        assert_eq!(after.lines().filter(|l| l.trim() == "</map>").count(), 1);
    }

    #[test]
    fn edit_adds_the_key_to_a_fresh_file() {
        let after = edit_content(None, "2");
        assert_eq!(
            after,
            "<?xml version=\"1.0\" encoding=\"utf-8\" standalone=\"yes\" ?>\n\
             <map>\n    <string name=\"su_auto_response\">2</string>\n</map>\n"
        );
    }

    #[test]
    fn edit_handles_an_empty_self_closing_map() {
        // Android writes an empty SharedPreferences file self-closing; the
        // result must stay well-formed (one <map> root) or Android discards it.
        for empty in ["<map />", "<map/>"] {
            let before = format!(
                "<?xml version=\"1.0\" encoding=\"utf-8\" standalone=\"yes\" ?>\n{empty}\n"
            );
            let after = edit_content(Some(&before), "2");
            assert_eq!(
                after,
                "<?xml version=\"1.0\" encoding=\"utf-8\" standalone=\"yes\" ?>\n\
                 <map>\n    <string name=\"su_auto_response\">2</string>\n</map>\n",
                "input: {empty}"
            );
            assert!(!after.contains("<map />") && !after.contains("<map/>"));
        }
    }

    #[test]
    fn extracts_the_value_from_shared_preferences_xml() {
        let xml = "<map><string name=\"denylist\">false</string>\
                   <string name=\"su_auto_response\">2</string></map>";
        assert_eq!(extract_value(xml).as_deref(), Some("2"));
        assert_eq!(
            extract_value("<map><string name=\"other\">1</string></map>"),
            None
        );
    }

    #[test]
    fn shell_quoting_escapes_single_quotes() {
        assert_eq!(shell_single_quote("a'b"), "'a'\\''b'");
    }
}
