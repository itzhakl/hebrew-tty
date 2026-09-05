#![forbid(unsafe_code)]

use std::ffi::OsStr;
use std::path::Path;

use serde::Serialize;

use crate::config::Mode;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Host {
    Direct,
    Herdr,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Order {
    Logical,
    Visual,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Wrapping {
    PreBidi,
    PostBidi,
    NotObserved,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Confidence {
    Verified,
    Unknown,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ObservedEvidence {
    pub agent_version: Option<String>,
    pub order: Option<Order>,
    pub wrapping: Option<Wrapping>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ExecutionPath {
    pub order: Option<Order>,
    pub wrapping: Option<Wrapping>,
    pub confidence: Confidence,
    pub evidence: Vec<String>,
}

impl ExecutionPath {
    fn verified(
        order: Order,
        fixture_prefix: &str,
        recorded_version: &str,
        observed_version: &str,
    ) -> Self {
        let mut evidence = vec![
            format!(
                "verified measurement pair: {fixture_prefix}-48.json, {fixture_prefix}-80.json"
            ),
            format!("recorded agent version: {recorded_version}"),
            format!("recorded order: {order:?}"),
            "48-column recording verifies post_bidi wrapping".to_owned(),
        ];
        if observed_version != recorded_version {
            evidence.push(format!(
                "observed agent version {observed_version} is newer than the recording; \
                 the recorded order carries forward until a row contradicts it"
            ));
        }
        Self {
            order: Some(order),
            wrapping: Some(Wrapping::PostBidi),
            confidence: Confidence::Verified,
            evidence,
        }
    }

    fn unknown(reason: impl Into<String>) -> Self {
        Self {
            order: None,
            wrapping: None,
            confidence: Confidence::Unknown,
            evidence: vec![reason.into()],
        }
    }
}

/// The names a measurement pair exists for. Anything else passes through.
pub fn is_recorded_agent(name: &str) -> bool {
    matches!(name, "claude" | "pi" | "codex")
}

fn product_marker(version: &str) -> String {
    version
        .chars()
        .filter(|character| !character.is_ascii_digit() && *character != '.')
        .collect::<String>()
        .trim()
        .to_owned()
}

fn reports_the_same_product(observed: &str, recorded: &str) -> bool {
    let marker = product_marker(recorded);
    if marker.is_empty() {
        return observed
            .split_whitespace()
            .next()
            .is_some_and(|first| first.chars().all(|c| c.is_ascii_digit() || c == '.'));
    }
    product_marker(observed) == marker
}

#[derive(Default)]
pub struct Classifier;

impl Classifier {
    pub fn observe(
        &self,
        command: &OsStr,
        host: Option<Host>,
        observed: ObservedEvidence,
    ) -> ExecutionPath {
        let Some(host) = host else {
            return ExecutionPath::unknown("host evidence is missing");
        };
        let command = Path::new(command)
            .file_name()
            .unwrap_or(command)
            .to_string_lossy();
        let host_name = match host {
            Host::Direct => "direct",
            Host::Herdr => "herdr",
        };
        let (expected_order, expected_version) = match command.as_ref() {
            "claude" => (Order::Visual, "2.1.251 (Claude Code)"),
            "pi" => (Order::Visual, "0.84.4"),
            "codex" => (Order::Logical, "codex-cli 0.151.0"),
            _ => {
                return ExecutionPath::unknown(format!(
                    "no recorded path for {command}/{host_name}"
                ))
            }
        };
        let Some(agent_version) = observed.agent_version.as_deref() else {
            return ExecutionPath::unknown("agent version evidence is missing");
        };
        if !reports_the_same_product(agent_version, expected_version) {
            return ExecutionPath::unknown(format!(
                "agent version {agent_version} is not a build of {expected_version}"
            ));
        }
        let expected = ExecutionPath::verified(
            expected_order,
            &format!("{command}-{host_name}"),
            expected_version,
            agent_version,
        );

        if observed
            .order
            .is_some_and(|order| Some(order) != expected.order)
        {
            return ExecutionPath::unknown("observed order contradicts recorded path");
        }
        if observed.wrapping.is_some_and(|wrapping| {
            wrapping != Wrapping::NotObserved && Some(wrapping) != expected.wrapping
        }) {
            return ExecutionPath::unknown("observed wrapping contradicts recorded path");
        }
        expected
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RowDisposition {
    TransformLogical,
    RecoverVisual,
    PassThrough,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Selection {
    pub disposition: RowDisposition,
    pub safe_mode_reason: Option<String>,
}

impl Selection {
    pub fn apply_noop<'a>(&self, input: &'a [u8]) -> &'a [u8] {
        input
    }
}

pub fn select_mode(mode: Mode, path: &ExecutionPath) -> Selection {
    match mode {
        Mode::Passthrough => Selection {
            disposition: RowDisposition::PassThrough,
            safe_mode_reason: Some("explicit_passthrough".to_owned()),
        },
        Mode::Logical => Selection {
            disposition: RowDisposition::TransformLogical,
            safe_mode_reason: None,
        },
        Mode::Visual => Selection {
            disposition: RowDisposition::RecoverVisual,
            safe_mode_reason: None,
        },
        Mode::Auto => match (path.confidence, path.order) {
            (Confidence::Verified, Some(Order::Logical)) => Selection {
                disposition: RowDisposition::TransformLogical,
                safe_mode_reason: None,
            },
            (Confidence::Verified, Some(Order::Visual)) => Selection {
                disposition: RowDisposition::RecoverVisual,
                safe_mode_reason: None,
            },
            _ => Selection {
                disposition: RowDisposition::PassThrough,
                safe_mode_reason: Some("unclassified_execution_path".to_owned()),
            },
        },
    }
}
