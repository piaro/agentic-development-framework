//! Reviewed baseline comparison for repository-wide detector audits.
//!
//! A matching baseline proves only that a pinned repository reproduced the
//! reviewed audit output. It never turns an audit coverage gap into complete
//! runtime coverage or an authoritative project Binding.

use crate::canonical_digest;
use crate::detector_audit::{DetectorAuditGap, run_repository_detector_audit};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::fmt;
use std::fs;
use std::path::Path;

pub const DETECTOR_AUDIT_BASELINE_SCHEMA_VERSION: &str = "1";
pub const DETECTOR_AUDIT_BASELINE_REPORT_SCHEMA_VERSION: &str = "1";
const MAX_BASELINE_BYTES: u64 = 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
struct DetectorAuditBaseline {
    schema_version: String,
    id: String,
    repository: String,
    revision: String,
    content_digest: String,
    report_digest: String,
    expected_audit_status: String,
    expected_gaps: Vec<DetectorAuditGap>,
    review: DetectorAuditBaselineReview,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
struct DetectorAuditBaselineReview {
    reviewed_at: String,
    basis: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DetectorAuditBaselineMismatch {
    pub field: String,
    pub expected: Value,
    pub actual: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DetectorAuditBaselineReport {
    pub schema_version: String,
    pub baseline_id: String,
    pub baseline_digest: String,
    pub status: String,
    pub repository: String,
    pub revision: String,
    pub audit_status: String,
    pub content_digest: String,
    pub report_digest: String,
    pub expected_gaps: Vec<DetectorAuditGap>,
    pub actual_gaps: Vec<DetectorAuditGap>,
    pub mismatches: Vec<DetectorAuditBaselineMismatch>,
}

impl DetectorAuditBaselineReport {
    pub fn matched(&self) -> bool {
        self.status == "matched"
    }

    pub fn as_value(&self) -> Value {
        serde_json::to_value(self).expect("Detector Audit Baseline Report is serializable")
    }

    pub fn render_text(&self) -> String {
        let mut lines = vec![
            format!("Detector audit baseline: {}", self.status),
            format!("baseline: {}", self.baseline_id),
            format!("baseline_digest: {}", self.baseline_digest),
            format!("repository: {}", self.repository),
            format!("revision: {}", self.revision),
            format!("audit_status: {}", self.audit_status),
            format!("content_digest: {}", self.content_digest),
            format!("report_digest: {}", self.report_digest),
        ];
        if self.audit_status == "blocked" {
            lines.push(
                "runtime_coverage: blocked (a baseline match does not waive coverage gaps)"
                    .to_owned(),
            );
        }
        for mismatch in &self.mismatches {
            lines.push(format!(
                "- mismatch {}: expected={} actual={}",
                mismatch.field, mismatch.expected, mismatch.actual
            ));
        }
        lines.join("\n") + "\n"
    }
}

pub fn check_repository_detector_audit_baseline(
    repository_root: &Path,
    baseline_path: &Path,
) -> Result<DetectorAuditBaselineReport, DetectorAuditBaselineError> {
    let metadata = fs::symlink_metadata(baseline_path)
        .map_err(|error| baseline_error(format!("cannot read audit baseline: {error}")))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(baseline_error(
            "detector audit baseline is not a regular file",
        ));
    }
    if metadata.len() > MAX_BASELINE_BYTES {
        return Err(baseline_error("detector audit baseline exceeds 1 MiB"));
    }
    let baseline_text = fs::read_to_string(baseline_path)
        .map_err(|error| baseline_error(format!("cannot read audit baseline: {error}")))?;
    let baseline_value: Value = serde_yaml::from_str(&baseline_text)
        .map_err(|error| baseline_error(format!("invalid detector audit baseline: {error}")))?;
    let baseline: DetectorAuditBaseline = serde_json::from_value(baseline_value.clone())
        .map_err(|error| baseline_error(format!("invalid detector audit baseline: {error}")))?;
    validate_baseline(&baseline)?;
    let baseline_digest = canonical_digest(&baseline_value)
        .map_err(|error| baseline_error(format!("cannot digest audit baseline: {error}")))?;

    let audit = run_repository_detector_audit(repository_root, true)
        .map_err(|error| baseline_error(error.to_string()))?;
    let report_digest = canonical_digest(&audit.as_value())
        .map_err(|error| baseline_error(format!("cannot digest detector audit: {error}")))?;
    let mut expected_gaps = baseline.expected_gaps.clone();
    expected_gaps.sort();
    let mut actual_gaps = audit.gaps.clone();
    actual_gaps.sort();
    let comparisons = [
        ("revision", json!(baseline.revision), json!(audit.revision)),
        (
            "content_digest",
            json!(baseline.content_digest),
            json!(audit.content_digest),
        ),
        (
            "report_digest",
            json!(baseline.report_digest),
            json!(report_digest),
        ),
        (
            "audit_status",
            json!(baseline.expected_audit_status),
            json!(audit.status),
        ),
        ("gaps", json!(expected_gaps), json!(actual_gaps)),
    ];
    let mismatches = comparisons
        .into_iter()
        .filter(|(_, expected, actual)| expected != actual)
        .map(|(field, expected, actual)| DetectorAuditBaselineMismatch {
            field: field.to_owned(),
            expected,
            actual,
        })
        .collect::<Vec<_>>();

    Ok(DetectorAuditBaselineReport {
        schema_version: DETECTOR_AUDIT_BASELINE_REPORT_SCHEMA_VERSION.to_owned(),
        baseline_id: baseline.id,
        baseline_digest,
        status: if mismatches.is_empty() {
            "matched"
        } else {
            "mismatched"
        }
        .to_owned(),
        repository: baseline.repository,
        revision: audit.revision,
        audit_status: audit.status,
        content_digest: audit.content_digest,
        report_digest,
        expected_gaps,
        actual_gaps,
        mismatches,
    })
}

fn validate_baseline(baseline: &DetectorAuditBaseline) -> Result<(), DetectorAuditBaselineError> {
    if baseline.schema_version != DETECTOR_AUDIT_BASELINE_SCHEMA_VERSION {
        return Err(baseline_error("unsupported detector audit baseline schema"));
    }
    if baseline.id.trim().is_empty() {
        return Err(baseline_error("detector audit baseline id is empty"));
    }
    if !baseline.repository.starts_with("https://") {
        return Err(baseline_error(
            "detector audit baseline repository must use https",
        ));
    }
    if !is_lower_hex(&baseline.revision, 40) {
        return Err(baseline_error(
            "detector audit baseline revision must be a 40-character lowercase Git SHA",
        ));
    }
    for (label, digest) in [
        ("content_digest", &baseline.content_digest),
        ("report_digest", &baseline.report_digest),
    ] {
        if !digest
            .strip_prefix("sha256:")
            .is_some_and(|value| is_lower_hex(value, 64))
        {
            return Err(baseline_error(format!(
                "detector audit baseline {label} is not a SHA-256 digest"
            )));
        }
    }
    if !matches!(
        baseline.expected_audit_status.as_str(),
        "complete" | "blocked"
    ) {
        return Err(baseline_error(
            "detector audit baseline status must be complete or blocked",
        ));
    }
    if !valid_date(&baseline.review.reviewed_at) {
        return Err(baseline_error(
            "detector audit baseline reviewed_at must use YYYY-MM-DD",
        ));
    }
    if baseline.review.basis.trim().is_empty() {
        return Err(baseline_error(
            "detector audit baseline review basis is empty",
        ));
    }
    Ok(())
}

fn valid_date(value: &str) -> bool {
    value.len() == 10
        && value.as_bytes()[4] == b'-'
        && value.as_bytes()[7] == b'-'
        && value
            .bytes()
            .enumerate()
            .all(|(index, byte)| matches!(index, 4 | 7) || byte.is_ascii_digit())
}

fn is_lower_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetectorAuditBaselineError {
    message: String,
}

impl fmt::Display for DetectorAuditBaselineError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for DetectorAuditBaselineError {}

fn baseline_error(message: impl Into<String>) -> DetectorAuditBaselineError {
    DetectorAuditBaselineError {
        message: message.into(),
    }
}
