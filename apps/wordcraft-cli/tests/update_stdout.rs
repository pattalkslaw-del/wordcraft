//! `wordcraft-cli update --check` must print exactly one JSON object on stdout (#15).

use std::process::{Command, Stdio};

#[test]
fn update_check_subprocess_stdout_is_one_json_value() {
    let out = Command::new(env!("CARGO_BIN_EXE_wordcraft-cli"))
        .args(["update", "--check"])
        .env("WORDCRAFT_INSTALL", "unknown")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn cli");
    assert!(out.status.success() || out.status.code() == Some(0), "cli exited: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let trimmed = stdout.trim();
    assert!(trimmed.starts_with('{') && trimmed.ends_with('}'), "stdout must be one JSON object, got: {stdout:?}");
    serde_json::from_str::<serde_json::Value>(trimmed).expect("stdout parses as JSON");
    assert!(!stdout.contains("dpkg-query"), "install probes must not leak to stdout: {stdout}");
}
