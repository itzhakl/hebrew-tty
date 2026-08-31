#![forbid(unsafe_code)]

use std::ffi::OsStr;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use hebrew_tty::classify::{
    select_mode, Classifier, Confidence, Host, ObservedEvidence, Order, RowDisposition, Wrapping,
};
use hebrew_tty::config::{Config, Mode};
use hebrew_tty::diagnostics::{DiagnosticRecord, Diagnostics};

fn temp_path(label: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("hebrew-tty-{label}-{}-{nonce}", std::process::id()))
}

fn measured_evidence(command: &str) -> ObservedEvidence {
    let version = match command {
        "claude" => "2.1.251 (Claude Code)",
        "pi" => "0.84.4",
        "codex" => "codex-cli 0.151.0",
        _ => "unmeasured",
    };
    ObservedEvidence {
        agent_version: Some(version.to_owned()),
        ..ObservedEvidence::default()
    }
}

#[test]
fn recorded_agent_and_host_paths_are_verified() {
    let classifier = Classifier;
    for (command, order) in [
        ("claude", Order::Visual),
        ("pi", Order::Visual),
        ("codex", Order::Logical),
    ] {
        for host in [Host::Direct, Host::Herdr] {
            let path =
                classifier.observe(OsStr::new(command), Some(host), measured_evidence(command));
            assert_eq!(path.confidence, Confidence::Verified);
            assert_eq!(path.order, Some(order));
            assert_eq!(path.wrapping, Some(Wrapping::PostBidi));
            assert!(path.evidence.iter().any(|item| item.contains(command)));
        }
    }
}

#[test]
fn an_agent_version_past_the_recording_keeps_the_recorded_order() {
    let path = Classifier.observe(
        OsStr::new("claude"),
        Some(Host::Direct),
        ObservedEvidence {
            agent_version: Some("2.1.252 (Claude Code)".to_owned()),
            ..ObservedEvidence::default()
        },
    );

    assert_eq!(path.confidence, Confidence::Verified);
    assert_eq!(path.order, Some(Order::Visual));
    assert_eq!(
        select_mode(Mode::Auto, &path).disposition,
        RowDisposition::RecoverVisual
    );
    assert!(path
        .evidence
        .iter()
        .any(|item| item.contains("2.1.252 (Claude Code)")));
}

#[test]
fn a_contradicting_row_still_overrides_an_unrecorded_version() {
    let path = Classifier.observe(
        OsStr::new("claude"),
        Some(Host::Direct),
        ObservedEvidence {
            agent_version: Some("2.1.252 (Claude Code)".to_owned()),
            order: Some(Order::Logical),
            ..ObservedEvidence::default()
        },
    );

    assert_eq!(path.confidence, Confidence::Unknown);
    assert_eq!(
        select_mode(Mode::Auto, &path).disposition,
        RowDisposition::PassThrough
    );
}

#[test]
fn incomplete_contradictory_and_unknown_paths_are_safe() {
    let classifier = Classifier;
    let cases = [
        classifier.observe(OsStr::new("claude"), None, measured_evidence("claude")),
        classifier.observe(
            OsStr::new("unknown-agent"),
            Some(Host::Direct),
            ObservedEvidence::default(),
        ),
        classifier.observe(
            OsStr::new("claude"),
            Some(Host::Direct),
            ObservedEvidence::default(),
        ),
        classifier.observe(
            OsStr::new("claude"),
            Some(Host::Direct),
            ObservedEvidence {
                order: Some(Order::Logical),
                ..measured_evidence("claude")
            },
        ),
        classifier.observe(
            OsStr::new("codex"),
            Some(Host::Herdr),
            ObservedEvidence {
                wrapping: Some(Wrapping::PreBidi),
                ..measured_evidence("codex")
            },
        ),
    ];
    let original = b"\x1b[31mambiguous \xd7\x90\xd7\x91\xd7\x92\x1b[0m";

    for path in cases {
        assert_eq!(path.confidence, Confidence::Unknown);
        let selection = select_mode(Mode::Auto, &path);
        assert_eq!(selection.disposition, RowDisposition::PassThrough);
        assert_eq!(selection.apply_noop(original), original);
        assert_eq!(
            selection.safe_mode_reason.as_deref(),
            Some("unclassified_execution_path")
        );
    }
}

#[test]
fn explicit_modes_override_unknown_classification_without_mutating_yet() {
    let path = Classifier.observe(
        OsStr::new("other"),
        Some(Host::Direct),
        ObservedEvidence::default(),
    );
    let original = b"original bytes";

    for (mode, expected) in [
        (Mode::Logical, RowDisposition::TransformLogical),
        (Mode::Visual, RowDisposition::RecoverVisual),
        (Mode::Passthrough, RowDisposition::PassThrough),
    ] {
        let selection = select_mode(mode, &path);
        assert_eq!(selection.disposition, expected);
        assert_eq!(selection.apply_noop(original), original);
    }
}

#[test]
fn config_loads_versioned_command_policies_with_cli_precedence() {
    let path = temp_path("config.toml");
    fs::write(
        &path,
        r#"
version = 1
default_mode = "passthrough"

[commands.claude]
mode = "visual"

[commands.codex]
mode = "logical"
"#,
    )
    .unwrap();
    let config = Config::load_from(&path).unwrap();
    fs::remove_file(path).unwrap();

    assert_eq!(
        config.policy_for(OsStr::new("/usr/bin/claude"), None).mode,
        Mode::Visual
    );
    assert_eq!(
        config.policy_for(OsStr::new("codex"), None).mode,
        Mode::Logical
    );
    assert_eq!(
        config.policy_for(OsStr::new("pi"), None).mode,
        Mode::Passthrough
    );
    assert_eq!(
        config
            .policy_for(OsStr::new("claude"), Some(Mode::Auto))
            .mode,
        Mode::Auto
    );
}

#[test]
fn config_rejects_unknown_versions_modes_and_fields() {
    for (label, source) in [
        ("version", "version = 2\n"),
        ("mode", "version = 1\ndefault_mode = \"guess\"\n"),
        ("field", "version = 1\nunexpected = true\n"),
    ] {
        let path = temp_path(label);
        fs::write(&path, source).unwrap();
        let result = Config::load_from(&path);
        fs::remove_file(path).unwrap();
        assert!(result.is_err(), "{label} must be rejected");
    }
}

#[test]
fn relative_xdg_config_home_falls_back_to_home() {
    let home = temp_path("home");
    let directory = home.join(".config/hebrew-tty");
    fs::create_dir_all(&directory).unwrap();
    fs::write(
        directory.join("config.toml"),
        "version = 1\ndefault_mode = \"visual\"\n",
    )
    .unwrap();

    let config =
        Config::load_from_environment(Some(OsStr::new("relative")), Some(home.as_os_str()))
            .unwrap();
    fs::remove_dir_all(home).unwrap();

    assert_eq!(
        config.policy_for(OsStr::new("claude"), None).mode,
        Mode::Visual
    );
}

#[test]
fn diagnostics_are_structured_and_do_not_touch_rendered_bytes() {
    let path = Classifier.observe(
        OsStr::new("mystery"),
        Some(Host::Herdr),
        ObservedEvidence::default(),
    );
    let selection = select_mode(Mode::Auto, &path);
    let rendered = b"unchanged terminal stream";
    let mut diagnostics = Diagnostics::new(Vec::new());
    diagnostics
        .emit(&DiagnosticRecord::new(
            "mystery",
            Some(Host::Herdr),
            Mode::Auto,
            &path,
            &selection,
        ))
        .unwrap();
    let output = diagnostics.into_inner();
    let value: serde_json::Value = serde_json::from_slice(&output).unwrap();

    assert_eq!(selection.apply_noop(rendered), rendered);
    assert_eq!(value["command"], "mystery");
    assert_eq!(value["host"], "herdr");
    assert_eq!(value["confidence"], "unknown");
    assert_eq!(value["selected_mode"], "auto");
    assert_eq!(value["row_disposition"], "pass_through");
    assert_eq!(value["safe_mode_reason"], "unclassified_execution_path");
    assert!(!value["evidence"].as_array().unwrap().is_empty());
}
