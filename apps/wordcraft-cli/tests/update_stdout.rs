//! `wordcraft-cli update --check` must print exactly one JSON object on stdout (#15).

use std::fs;
use std::process::{Command, Stdio};

#[test]
fn update_check_subprocess_stdout_is_one_json_value() {
    let dir = std::env::temp_dir().join(format!("wordcraft-update-check-{}", std::process::id()));
    fs::create_dir_all(&dir).expect("temp dir");
    let fixture = dir.join("latest.json");
    fs::write(&fixture, br#"{"tag_name":"v0.0.1","draft":false,"prerelease":false,"assets":[]}"#).expect("fixture");

    let out = Command::new(env!("CARGO_BIN_EXE_wordcraft-cli"))
        .args(["update", "--check"])
        .env("WORDCRAFT_UPDATE_CHECK_FIXTURE", &fixture)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn cli");
    let _ = fs::remove_dir_all(&dir);
    assert!(out.status.success() || out.status.code() == Some(0), "cli exited: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let trimmed = stdout.trim();
    assert!(trimmed.starts_with('{') && trimmed.ends_with('}'), "stdout must be one JSON object, got: {stdout:?}");
    let v: serde_json::Value = serde_json::from_str(trimmed).expect("stdout parses as JSON");
    assert_eq!(v.get("status").and_then(|s| s.as_str()), Some("current"), "fixture pins a deterministic response");
    assert!(!stdout.contains("dpkg-query"), "install probes must not leak to stdout: {stdout}");
}
