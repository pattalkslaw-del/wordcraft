//! `file.checkForUpdates` — desktop update check and apply (#15).

use serde_json::{Value, json};

use crate::update::{
    CheckResult, CheckStatus, InstallHandoff, LATEST_API, ReleaseInfo, SystemEnv, checksum_for, checksums_download_url, detect_install_kind, due,
    http_get, offer, parse_latest_release, parse_version, pick_asset_name, skipped, verify_sha256, version_text,
};
use crate::{CmdError, CmdResult, CommandSpec, Session, p};

pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec::new("file.checkForUpdates", "Check for Updates", "Help", check).params(
        r#"{"force"?: bool (ignore the once-a-day throttle), "apply"?: bool (download, verify checksum and hand off to the installer), "startup"?: bool (respect check-on-startup and throttle; default false for explicit checks)}"#,
    ).pure()]
}

fn check(s: &mut Session, v: &Value) -> CmdResult {
    let force = p::bool(v, "force").unwrap_or(false);
    let apply = p::bool(v, "apply").unwrap_or(false);
    let startup = p::bool(v, "startup").unwrap_or(false);
    let current = parse_version(env!("CARGO_PKG_VERSION")).unwrap_or((0, 0, 0));
    let current_text = version_text(current);
    let now = (s.update_now_unix)();
    let check_on_startup = s.update_prefs.check_on_startup;
    let last_check = s.update_prefs.last_check_unix;
    let skipped_ver = s.update_prefs.skipped_version.clone();

    if startup && !due(check_on_startup, last_check, now, false) {
        return Ok(json_result(CheckResult { status: CheckStatus::Throttled, current: current_text, release: None, error: None }));
    }
    if !force && !startup && last_check != 0 {
        let elapsed = now.checked_sub(last_check);
        if now != 0 && elapsed.is_some_and(|e| e < crate::update::INTERVAL_SECS) {
            return Ok(json_result(CheckResult { status: CheckStatus::Throttled, current: current_text, release: None, error: None }));
        }
    }

    if startup && !check_on_startup {
        return Ok(json_result(CheckResult { status: CheckStatus::Throttled, current: current_text, release: None, error: None }));
    }

    s.update_prefs.last_check_unix = now;

    let json = match http_get(s, LATEST_API) {
        Ok(b) => b,
        Err(e) => {
            return Ok(json_result(CheckResult { status: CheckStatus::Error, current: current_text, release: None, error: Some(e) }));
        }
    };

    let release = match parse_latest_release(&json, current) {
        Ok(Some(r)) => r,
        Ok(None) => {
            return Ok(json_result(CheckResult { status: CheckStatus::Current, current: current_text, release: None, error: None }));
        }
        Err(e) => {
            return Ok(json_result(CheckResult { status: CheckStatus::Error, current: current_text, release: None, error: Some(e) }));
        }
    };

    if skipped(&release, &skipped_ver) {
        return Ok(json_result(CheckResult { status: CheckStatus::Current, current: current_text, release: None, error: None }));
    }

    let exe = s.update_exe.as_deref().unwrap_or("");
    let kind = detect_install_kind(exe, &SystemEnv);
    let offer = offer(release.clone(), &skipped_ver, kind);

    if apply {
        if kind.external_updater() || kind == crate::update::InstallKind::Unknown {
            return Err(CmdError::Failed("open the release page in your browser to update".into()));
        }
        if let Err(e) = download_and_install(s, &json, &release, kind) {
            return Ok(json_result(CheckResult { status: CheckStatus::Error, current: current_text, release: Some(offer), error: Some(e) }));
        }
        s.ui_requests.push(json!({"quit": true}));
    } else {
        s.ui_requests.push(json!({
            "update": {
                "version": offer.version,
                "notes": offer.notes,
                "pageUrl": offer.page_url,
                "oneClick": offer.one_click,
                "externalUpdater": offer.external_updater,
                "installKind": offer.install_kind,
            }
        }));
    }

    Ok(json_result(CheckResult { status: CheckStatus::Available, current: current_text, release: Some(offer), error: None }))
}

fn json_result(r: CheckResult) -> Value {
    serde_json::to_value(r).unwrap_or(json!({"status": "error"}))
}

fn download_and_install(s: &mut Session, latest_json: &[u8], release: &ReleaseInfo, kind: crate::update::InstallKind) -> Result<(), String> {
    let file_name = pick_asset_name(release.version, kind).ok_or_else(|| "no installer for this install type".to_string())?;
    let asset_url = crate::update::asset_download_url(latest_json, &file_name).ok_or_else(|| "release has no matching installer".to_string())?;
    let sums_url = checksums_download_url(latest_json).ok_or_else(|| "release has no checksum file".to_string())?;
    let sums = http_get(s, &sums_url)?;
    let expected = checksum_for(&sums, &file_name)?.ok_or_else(|| format!("checksums do not list {file_name}"))?;
    let bytes = http_get(s, &asset_url)?;
    if !verify_sha256(&bytes, &expected) {
        return Err("downloaded file does not match the published SHA-256 checksum".into());
    }
    let install = s.update_install.as_ref().ok_or_else(|| "installer is not available".to_string())?;
    let exe = s.update_exe.clone().unwrap_or_default();
    install(InstallHandoff { kind, file_name, bytes, exe_path: exe })
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use serde_json::json;

    use super::*;

    fn test_now() -> u64 {
        2_000_000_000
    }
    use crate::update::{InstallHandoff, InstallKind, MAX_JSON};

    fn session_with(http: Arc<dyn Fn(&str) -> Result<Vec<u8>, String> + Send + Sync>) -> Session {
        let mut s = Session::new(wordcraft_doc::Document::new());
        s.update_http = Some(http);
        s.update_exe = Some("/usr/bin/wordcraft".into());
        s.update_now_unix = test_now;
        s
    }

    const LATEST: &str = r#"{"tag_name":"v9.0.0","draft":false,"prerelease":false,"body":"Hi","html_url":"https://github.com/storytold/wordcraft/releases/tag/v9.0.0","assets":[{"name":"SHA256SUMS.txt","browser_download_url":"https://example.com/sums"},{"name":"wordcraft-9.0.0-linux-x86_64.deb","browser_download_url":"https://example.com/pkg.deb"}]}"#;

    #[test]
    fn no_newer_version() {
        let cur = env!("CARGO_PKG_VERSION");
        let tag = format!("v{cur}");
        let body = format!(r#"{{"tag_name":"{tag}","draft":false,"prerelease":false,"assets":[]}}"#);
        let mut s = session_with(Arc::new(move |u| {
            assert_eq!(u, LATEST_API);
            Ok(body.clone().into_bytes())
        }));
        let r = check(&mut s, &json!({"force": true})).unwrap();
        assert_eq!(r["status"], "current");
    }

    #[test]
    fn newer_version_notice() {
        let s = session_with(Arc::new(|_| Ok(LATEST.as_bytes().to_vec())));
        let mut s = s;
        let r = check(&mut s, &json!({"force": true})).unwrap();
        assert_eq!(r["status"], "available");
        assert!(s.ui_requests.iter().any(|x| x.get("update").is_some()));
    }

    #[test]
    fn skip_this_version() {
        let s = session_with(Arc::new(|_| Ok(LATEST.as_bytes().to_vec())));
        let mut s = s;
        s.update_prefs.skipped_version = "9.0.0".into();
        let r = check(&mut s, &json!({"force": true})).unwrap();
        assert_eq!(r["status"], "current");
    }

    #[test]
    fn once_a_day_throttle() {
        let calls = Arc::new(Mutex::new(0));
        let c = Arc::clone(&calls);
        let mut s = session_with(Arc::new(move |_| {
            *c.lock().unwrap_or_else(|e| e.into_inner()) += 1;
            Ok(LATEST.as_bytes().to_vec())
        }));
        s.update_prefs.last_check_unix = 1_999_999_000;
        let r = check(&mut s, &json!({})).unwrap();
        assert_eq!(r["status"], "throttled");
        assert_eq!(*calls.lock().unwrap_or_else(|e| e.into_inner()), 0);
    }

    #[test]
    fn setting_off_means_no_request_on_startup() {
        let calls = Arc::new(Mutex::new(0));
        let c = Arc::clone(&calls);
        let mut s = session_with(Arc::new(move |_| {
            *c.lock().unwrap_or_else(|e| e.into_inner()) += 1;
            Ok(vec![])
        }));
        s.update_prefs.check_on_startup = false;
        let r = check(&mut s, &json!({"startup": true})).unwrap();
        assert_eq!(r["status"], "throttled");
        assert_eq!(*calls.lock().unwrap_or_else(|e| e.into_inner()), 0);
    }

    #[test]
    fn checksum_mismatch_refused() {
        let sums = b"0000000000000000000000000000000000000000000000000000000000000000  wordcraft-9.0.0-linux-x86_64.deb\n";
        let mut s = session_with(Arc::new(move |u| {
            Ok(if u.contains("sums") {
                sums.to_vec()
            } else if u.contains("pkg") {
                b"bad".to_vec()
            } else {
                LATEST.as_bytes().to_vec()
            })
        }));
        s.update_install = Some(Arc::new(|_| Ok(())));
        let r = check(&mut s, &json!({"force": true, "apply": true})).unwrap();
        assert_eq!(r["status"], "error");
        assert!(r["error"].as_str().unwrap_or("").contains("checksum"));
    }

    #[test]
    fn each_install_handoff_selected() {
        let installed = Arc::new(Mutex::new(None));
        let got = Arc::clone(&installed);
        let sums = b"2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824  wordcraft-9.0.0-linux-x86_64.deb\n";
        let mut s = session_with(Arc::new(move |u| {
            Ok(if u.contains("sums") {
                sums.to_vec()
            } else if u.contains("pkg") {
                b"hello".to_vec()
            } else {
                LATEST.as_bytes().to_vec()
            })
        }));
        s.update_install = Some(Arc::new(move |h: InstallHandoff| {
            *got.lock().unwrap_or_else(|e| e.into_inner()) = Some(h.kind);
            Ok(())
        }));
        let r = check(&mut s, &json!({"force": true, "apply": true})).unwrap();
        assert_eq!(r["status"], "available");
        assert_eq!(*installed.lock().unwrap_or_else(|e| e.into_inner()), Some(InstallKind::LinuxDeb));
    }

    #[test]
    fn unknown_install_type_falls_back_on_apply() {
        let s = session_with(Arc::new(|_| Ok(LATEST.as_bytes().to_vec())));
        let mut s = s;
        s.update_exe = Some("/tmp/dev/wordcraft".into());
        s.update_prefs.check_on_startup = false;
        let err = check(&mut s, &json!({"force": true, "apply": true})).unwrap_err();
        assert!(err.to_string().contains("release page"));
    }

    #[test]
    fn hostile_json_never_panics() {
        let s = session_with(Arc::new(|_| Ok(vec![b'x'; MAX_JSON + 1])));
        let mut s = s;
        let r = check(&mut s, &json!({"force": true}));
        assert!(r.is_ok());
    }
}
