//! HTTPS for `wordcraft-cli update --check` (#15).

use std::sync::Arc;
use std::time::Duration;

use wordcraft_engine::update::{HttpGet, MAX_JSON};

pub fn http_get() -> HttpGet {
    Arc::new(|url| {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(30)))
            .user_agent(concat!("wordcraft-cli/", env!("CARGO_PKG_VERSION"), " (+https://github.com/storytold/wordcraft)"))
            .build()
            .into();
        let mut response = agent
            .get(url)
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .call()
            .map_err(|e| e.to_string())?;
        response.body_mut().with_config().limit(MAX_JSON as u64).read_to_vec().map_err(|e| e.to_string())
    })
}
