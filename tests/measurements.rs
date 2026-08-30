use std::path::PathBuf;
use std::process::Command;

#[test]
fn recorded_terminal_paths_have_complete_measurements() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let status = Command::new("python3")
        .arg(root.join("tools/terminal_proxy_probe.py"))
        .arg("verify")
        .arg(root.join("test/fixtures/terminal-proxy/measurements"))
        .status()
        .expect("measurement verifier must start");

    assert!(status.success(), "measurement fixtures failed verification");
}
