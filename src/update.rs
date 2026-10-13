//! Checks GitHub Releases for a newer version of FrameMap Studio.
//! Only a single read-only request to api.github.com is made; nothing is sent about the user.

use std::sync::{Arc, Mutex};
use std::time::Duration;

pub const REPO: &str = "muruhukyejacob/framemap-studio";

#[derive(Clone, Debug)]
pub enum State {
    Checking,
    UpToDate { latest: String },
    Available { version: String, notes: String },
    Failed(String),
}

pub fn releases_page() -> String {
    format!("https://github.com/{REPO}/releases/latest")
}

/// Direct link to the installer for this operating system.
pub fn download_url() -> String {
    if cfg!(target_os = "macos") {
        format!("https://github.com/{REPO}/releases/latest/download/FrameMap-Studio.dmg")
    } else if cfg!(target_os = "windows") {
        format!("https://github.com/{REPO}/releases/latest/download/FrameMap-Studio-Setup.exe")
    } else {
        releases_page()
    }
}

/// Parse "v1.2.3" / "1.2.3" into comparable numbers.
pub fn parse_version(v: &str) -> Vec<u64> {
    v.trim().trim_start_matches(['v', 'V']).split(['.', '-', '+']).take(3).map(|p| p.parse().unwrap_or(0)).collect()
}

pub fn is_newer(candidate: &str, current: &str) -> bool {
    let (mut a, mut b) = (parse_version(candidate), parse_version(current));
    a.resize(3, 0);
    b.resize(3, 0);
    a > b
}

/// Latest release as (tag, notes); Ok(None) when nothing has been published yet.
fn fetch_latest() -> Result<Option<(String, String)>, String> {
    let agent = ureq::AgentBuilder::new().timeout(Duration::from_secs(12)).try_proxy_from_env(true).build();
    let resp = agent
        .get(&format!("https://api.github.com/repos/{REPO}/releases/latest"))
        .set("User-Agent", concat!("FrameMap-Studio/", env!("CARGO_PKG_VERSION")))
        .set("Accept", "application/vnd.github+json")
        .call();
    let resp = match resp {
        Ok(r) => r,
        Err(ureq::Error::Status(404, _)) => return Ok(None),
        Err(e) => return Err(e.to_string()),
    };
    let v: serde_json::Value = resp.into_json().map_err(|e| e.to_string())?;
    let tag = v["tag_name"].as_str().ok_or("no release found")?.to_string();
    let notes = v["body"].as_str().unwrap_or("").to_string();
    Ok(Some((tag, notes)))
}

/// Start a background check; the returned state is filled in when it finishes.
pub fn check_in_background(ctx: eframe::egui::Context) -> Arc<Mutex<State>> {
    let state = Arc::new(Mutex::new(State::Checking));
    let out = state.clone();
    std::thread::spawn(move || {
        let current = env!("CARGO_PKG_VERSION");
        let result = match fetch_latest() {
            Ok(None) => State::UpToDate { latest: String::new() },
            Ok(Some((tag, notes))) => {
                let version = tag.trim_start_matches(['v', 'V']).to_string();
                if is_newer(&version, current) {
                    State::Available { version, notes }
                } else {
                    State::UpToDate { latest: version }
                }
            }
            Err(e) => State::Failed(e),
        };
        if let Ok(mut s) = out.lock() {
            *s = result;
        }
        ctx.request_repaint();
    });
    state
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn versions() {
        assert!(is_newer("1.1.0", "1.0.0"));
        assert!(is_newer("v1.10.0", "1.9.3"));
        assert!(is_newer("2.0", "1.9.9"));
        assert!(!is_newer("1.1.0", "1.1.0"));
        assert!(!is_newer("v1.0.9", "1.1.0"));
    }
}
