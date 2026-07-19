use std::io;
use std::process::Command;

/// GitHub "owner/repo" to query for releases. Edit this to your repository.
const REPO: &str = "USER/btix";

/// Checks GitHub for a newer release and prints the result. Invoked by
/// --check-update. Network access goes through the system curl so we add no
/// HTTP dependency; failures degrade gracefully instead of panicking.
pub fn check() -> io::Result<()> {
    let current = env!("CARGO_PKG_VERSION");
    println!("Current version: {current}");

    let latest = match fetch_latest_tag() {
        Some(tag) => tag,
        None => {
            println!("Could not check for updates (no network or no releases yet).");
            return Ok(());
        }
    };

    let latest_v = parse_version(&latest);
    let current_v = parse_version(current);
    if is_newer(&latest_v, &current_v) {
        println!("Update available: {latest}");
        println!("  https://github.com/{REPO}/releases/latest");
    } else {
        println!("You are up to date ({latest}).");
    }
    Ok(())
}

/// Asks the GitHub API for the latest release tag, e.g. "v0.2.0".
fn fetch_latest_tag() -> Option<String> {
    let url = format!("https://api.github.com/repos/{REPO}/releases/latest");
    let output = Command::new("/usr/bin/curl")
        .args([
            "-sSL",
            "--max-time",
            "5",
            "-H",
            "User-Agent: btix",
            "-H",
            "Accept: application/vnd.github+json",
            &url,
        ])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let body = String::from_utf8_lossy(&output.stdout);
    parse_tag(&body)
}

/// Extracts the `tag_name` string from a GitHub release JSON without pulling in
/// a JSON parser.
fn parse_tag(json: &str) -> Option<String> {
    let key = "\"tag_name\"";
    let idx = json.find(key)?;
    let after = &json[idx + key.len()..];
    let colon = after.find(':')?;
    let after = &after[colon + 1..];
    let start = after.find('"')? + 1;
    let end = after[start..].find('"')? + start;
    let tag = after[start..end].trim();
    if tag.is_empty() {
        None
    } else {
        Some(tag.to_string())
    }
}

/// Splits a version like "v1.2.3" into numeric components [1, 2, 3].
fn parse_version(s: &str) -> Vec<u32> {
    s.trim()
        .trim_start_matches('v')
        .split('.')
        .map(|part| {
            let digits: String = part.chars().take_while(|c| c.is_ascii_digit()).collect();
            digits.parse::<u32>().unwrap_or(0)
        })
        .collect()
}

/// Whether `latest` is a strictly higher version than `current`.
fn is_newer(latest: &[u32], current: &[u32]) -> bool {
    for i in 0..latest.len().max(current.len()) {
        let l = latest.get(i).copied().unwrap_or(0);
        let c = current.get(i).copied().unwrap_or(0);
        if l != c {
            return l > c;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_tag_from_json() {
        let json = r#"{"url":"x","tag_name":"v0.2.0","name":"r"}"#;
        assert_eq!(parse_tag(json).as_deref(), Some("v0.2.0"));
    }

    #[test]
    fn missing_tag_is_none() {
        assert!(parse_tag(r#"{"message":"Not Found"}"#).is_none());
    }

    #[test]
    fn version_compare() {
        assert!(is_newer(&parse_version("v0.2.0"), &parse_version("0.1.0")));
        assert!(is_newer(&parse_version("1.0.0"), &parse_version("0.9.9")));
        assert!(!is_newer(&parse_version("0.1.0"), &parse_version("0.1.0")));
        assert!(!is_newer(&parse_version("0.1.0"), &parse_version("0.2.0")));
        assert!(is_newer(&parse_version("0.1.2"), &parse_version("0.1.1")));
    }
}
