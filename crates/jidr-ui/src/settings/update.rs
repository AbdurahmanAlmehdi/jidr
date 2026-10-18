//! Settings › About › Check for updates (D17): one request to GitHub's
//! latest-release API, only when the button is clicked. Nothing is downloaded
//! or installed; the result says whether a newer release exists.

use std::path::Path;

use serde::Deserialize;

/// The brew command shown to people who installed the cask.
pub const BREW_UPGRADE: &str = "brew upgrade --cask jidr";

/// What a check found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Check {
    UpToDate,
    /// A newer release: its version (no `v`) and its release page.
    Available {
        version: String,
        url: String,
    },
    /// Offline, rate-limited or an unexpected answer.
    Failed,
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    html_url: String,
}

/// `https://github.com/<owner>/<repo>` → its latest-release API URL.
pub fn latest_url(repo: &str) -> Option<String> {
    let path = repo
        .trim_end_matches('/')
        .strip_prefix("https://github.com/")?;
    Some(format!(
        "https://api.github.com/repos/{path}/releases/latest"
    ))
}

/// `v1.2.3`, `1.2` or `1.2.3-beta.1` → (1, 2, 3); a pre-release suffix is
/// ignored (`/releases/latest` never returns one).
fn parse(version: &str) -> Option<(u64, u64, u64)> {
    let core = version.trim().trim_start_matches('v');
    let core = core.split(['-', '+']).next()?;
    let mut parts = core.split('.').map(str::parse::<u64>);
    let major = parts.next()?.ok()?;
    let minor = parts.next().unwrap_or(Ok(0)).ok()?;
    let patch = parts.next().unwrap_or(Ok(0)).ok()?;
    Some((major, minor, patch))
}

/// Reads a `releases/latest` response against the running `current` version.
pub fn evaluate(body: &str, current: &str) -> Check {
    let Ok(release) = serde_json::from_str::<Release>(body) else {
        return Check::Failed;
    };
    match (parse(&release.tag_name), parse(current)) {
        (Some(latest), Some(now)) if latest > now => Check::Available {
            version: release.tag_name.trim_start_matches('v').to_string(),
            url: release.html_url,
        },
        (Some(_), Some(_)) => Check::UpToDate,
        _ => Check::Failed,
    }
}

/// The About tab's status line, its action button and the button's link
/// (empty = copy [`BREW_UPGRADE`]); no button when the action is empty.
pub fn view(check: &Check, current: &str, homebrew: bool, repo: &str) -> (String, String, String) {
    match check {
        Check::UpToDate => (
            format!("You're up to date ({current})"),
            String::new(),
            String::new(),
        ),
        Check::Available { version, .. } if homebrew => (
            format!("Jidr {version} is available. Update with {BREW_UPGRADE}"),
            "Copy command".into(),
            String::new(),
        ),
        Check::Available { version, url } => (
            format!("Jidr {version} is available"),
            "Download".into(),
            url.clone(),
        ),
        Check::Failed => (
            "Couldn't reach GitHub".into(),
            "Open releases page".into(),
            format!("{repo}/releases"),
        ),
    }
}

/// True when the app came from the Homebrew cask (it keeps a Caskroom entry).
pub fn installed_by_homebrew() -> bool {
    ["/opt/homebrew/Caskroom/jidr", "/usr/local/Caskroom/jidr"]
        .iter()
        .any(|p| Path::new(p).exists())
}

/// Asks GitHub for the latest release; any network or HTTP error is `Failed`.
pub async fn check(repo: &str, current: &str) -> Check {
    let Some(url) = latest_url(repo) else {
        return Check::Failed;
    };
    let Ok(client) = reqwest::Client::builder()
        .user_agent(concat!("Jidr/", env!("CARGO_PKG_VERSION")))
        .timeout(std::time::Duration::from_secs(8))
        .build()
    else {
        return Check::Failed;
    };
    let resp = match client
        .get(url)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
    {
        Ok(r) if r.status().is_success() => r,
        _ => return Check::Failed,
    };
    match resp.text().await {
        Ok(body) => evaluate(&body, current),
        Err(_) => Check::Failed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A trimmed `GET /repos/{owner}/{repo}/releases/latest` response.
    const LATEST: &str = r#"{
        "url": "https://api.github.com/repos/AbdurahmanAlmehdi/jidr/releases/1",
        "html_url": "https://github.com/AbdurahmanAlmehdi/jidr/releases/tag/v0.2.0",
        "tag_name": "v0.2.0",
        "name": "v0.2.0",
        "draft": false,
        "prerelease": false,
        "assets": []
    }"#;

    #[test]
    fn api_url_from_the_repo() {
        assert_eq!(
            latest_url("https://github.com/AbdurahmanAlmehdi/jidr").as_deref(),
            Some("https://api.github.com/repos/AbdurahmanAlmehdi/jidr/releases/latest")
        );
        assert_eq!(latest_url("https://example.com/x"), None);
    }

    #[test]
    fn newer_release_is_available() {
        assert_eq!(
            evaluate(LATEST, "0.1.0"),
            Check::Available {
                version: "0.2.0".into(),
                url: "https://github.com/AbdurahmanAlmehdi/jidr/releases/tag/v0.2.0".into(),
            }
        );
        assert_eq!(evaluate(LATEST, "0.1.9"), evaluate(LATEST, "0.1"));
    }

    #[test]
    fn same_or_newer_install_is_up_to_date() {
        assert_eq!(evaluate(LATEST, "0.2.0"), Check::UpToDate);
        assert_eq!(evaluate(LATEST, "0.10.0"), Check::UpToDate);
        assert_eq!(evaluate(LATEST, "1.0.0-beta.1"), Check::UpToDate);
    }

    #[test]
    fn versions_compare_numerically() {
        assert!(parse("v0.10.0") > parse("0.9.9"));
        assert_eq!(parse("1.2"), Some((1, 2, 0)));
        assert_eq!(parse("2.0.0-rc.1"), Some((2, 0, 0)));
        assert_eq!(parse("latest"), None);
    }

    #[test]
    fn homebrew_installs_get_the_brew_command() {
        let repo = "https://github.com/AbdurahmanAlmehdi/jidr";
        let new = evaluate(LATEST, "0.1.0");
        let (status, action, url) = view(&new, "0.1.0", true, repo);
        assert!(status.ends_with(BREW_UPGRADE));
        assert_eq!((action.as_str(), url.as_str()), ("Copy command", ""));
        let (_, action, url) = view(&new, "0.1.0", false, repo);
        assert_eq!(action, "Download");
        assert!(url.ends_with("/tag/v0.2.0"));
        let (status, action, _) = view(&Check::UpToDate, "0.2.0", false, repo);
        assert_eq!(
            (status.as_str(), action.as_str()),
            ("You're up to date (0.2.0)", "")
        );
        let (_, _, url) = view(&Check::Failed, "0.1.0", true, repo);
        assert_eq!(url, format!("{repo}/releases"));
    }

    #[test]
    fn bad_answers_fail() {
        assert_eq!(evaluate("rate limited", "0.1.0"), Check::Failed);
        assert_eq!(
            evaluate(r#"{"message":"Not Found"}"#, "0.1.0"),
            Check::Failed
        );
        assert_eq!(
            evaluate(r#"{"tag_name":"nightly","html_url":"x"}"#, "0.1.0"),
            Check::Failed
        );
    }
}
