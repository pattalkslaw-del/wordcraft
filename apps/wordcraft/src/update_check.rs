//! HTTPS fetch for the daily update check (#15).

use std::sync::Arc;
use std::time::Duration;

use wordcraft_engine::update::{HttpGet, LATEST_API, MAX_JSON};
use wordcraft_ui_egui::update::Fetch;

fn turned_off(v: Option<&str>) -> bool {
    v.is_some_and(|v| !v.trim().is_empty() && v.trim() != "0")
}

pub fn service(prefs_enabled: bool) -> Option<Fetch> {
    if !prefs_enabled
        || turned_off(option_env!("WORDCRAFT_NO_UPDATE_CHECK"))
        || turned_off(std::env::var("WORDCRAFT_NO_UPDATE_CHECK").ok().as_deref())
        || std::env::var_os("FLATPAK_ID").is_some()
    {
        return None;
    }
    Some(Arc::new(fetch_latest))
}

pub fn http_get() -> HttpGet {
    Arc::new(fetch_url)
}

fn fetch_latest() -> Result<Vec<u8>, String> {
    fetch_url(LATEST_API)
}

fn fetch_url(url: &str) -> Result<Vec<u8>, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(30)))
        .user_agent(concat!("WordCraft/", env!("CARGO_PKG_VERSION"), " (+https://github.com/storytold/wordcraft)"))
        .build()
        .into();
    let mut response = agent
        .get(url)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .call()
        .map_err(|e| e.to_string())?;
    response.body_mut().with_config().limit(MAX_JSON as u64).read_to_vec().map_err(|e| e.to_string())
}
