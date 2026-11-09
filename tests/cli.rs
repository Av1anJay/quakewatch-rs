use serde_json::Value;
use std::process::Command;

#[test]
fn replay_prints_alert_update_cancel_once() {
    let out = Command::new(env!("CARGO_BIN_EXE_quakewatch"))
        .args([
            "replay",
            "--lat",
            "25.04",
            "--lon",
            "121.57",
            "--min",
            "2",
            "--at-ms",
            "1700000005000",
            "fixtures/demo.sse",
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = String::from_utf8(out.stdout).unwrap();
    let notices: Vec<Value> = text
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(
        notices
            .iter()
            .map(|n| n["action"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["alert", "update", "cancel"]
    );
    assert_eq!(notices[2]["estimate"], Value::Null);
}

#[test]
fn invalid_location_fails_before_network() {
    let out = Command::new(env!("CARGO_BIN_EXE_quakewatch"))
        .args(["live", "--lat", "-91", "--lon", "0", "--check"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("latitude/longitude out of range"));
}
