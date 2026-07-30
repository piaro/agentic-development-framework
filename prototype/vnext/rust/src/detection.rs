//! Deterministic conversion from declared repository facts to Signal candidates.
//!
//! The Detector reports possible signals. It never confirms a candidate or
//! selects a Requirement; those decisions remain in reviewed Results and the
//! Thin Kernel.

use crate::canonical_digest;
use crate::signal_catalog::{
    TYPED_FACT_DETECTOR_ID, TYPED_FACT_DETECTOR_VERSION, validate_signal_candidate,
};
use serde::Serialize;
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;
use std::fmt;

pub const DETECTOR_ID: &str = TYPED_FACT_DETECTOR_ID;
pub const DETECTOR_VERSION: &str = TYPED_FACT_DETECTOR_VERSION;

type SignalBindings = BTreeMap<String, String>;
type DetectedSignal = (String, SignalBindings);

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SignalCandidate {
    pub signal: String,
    pub bindings: BTreeMap<String, String>,
    pub evidence_refs: Vec<String>,
    pub detector_id: String,
    pub detector_version: String,
    /// Stable identity of the logical candidate. Evidence content is versioned
    /// separately so an implementation edit does not turn the same risk into a
    /// different candidate.
    pub fingerprint: String,
    pub evidence_digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DetectionCoverage {
    pub status: String,
    pub scope: String,
    pub analyzed_refs: Vec<String>,
    pub gaps: Vec<BTreeMap<String, String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DetectionReport {
    pub change_id: String,
    pub coverage: DetectionCoverage,
    pub candidates: Vec<SignalCandidate>,
    pub digest: String,
}

impl DetectionReport {
    pub fn compatibility_value(&self) -> Value {
        serde_json::to_value(self).expect("Detection Report contains serializable fields")
    }
}

/// Convert typed facts without reading code, invoking an Agent, or applying Rules.
pub fn detect_typed_facts(
    change_id: &str,
    facts: &[Value],
    coverage: &Value,
    artifact_digests: &BTreeMap<String, String>,
) -> Result<DetectionReport, DetectionError> {
    let coverage = detection_coverage(coverage)?;
    let mut detected = BTreeMap::<String, (String, SignalBindings, Vec<String>)>::new();
    for (fact_index, fact) in facts.iter().enumerate() {
        let fact = fact.as_object().ok_or_else(|| {
            DetectionError::new(format!("repository fact {fact_index} must be an object"))
        })?;
        for (signal, bindings) in signals_for(fact, fact_index)? {
            validate_signal_candidate(&signal, DETECTOR_ID, DETECTOR_VERSION, &bindings)
                .map_err(|error| DetectionError::new(error.to_string()))?;
            let mut evidence_refs = optional_string_array(fact, "evidence_refs", fact_index)?;
            evidence_refs.sort();
            let fingerprint_body = json!({
                "detector_id": DETECTOR_ID,
                "detector_version": DETECTOR_VERSION,
                "signal": signal,
                "bindings": bindings,
            });
            let fingerprint = canonical_digest(&fingerprint_body)
                .map_err(|error| DetectionError::new(error.to_string()))?;
            detected
                .entry(fingerprint)
                .and_modify(|(_, _, existing_refs)| {
                    existing_refs.extend(evidence_refs.iter().cloned());
                    existing_refs.sort();
                    existing_refs.dedup();
                })
                .or_insert((signal, bindings, evidence_refs));
        }
    }
    let mut candidates = detected
        .into_iter()
        .map(|(fingerprint, (signal, bindings, evidence_refs))| {
            let evidence: BTreeMap<String, String> = evidence_refs
                .iter()
                .map(|reference| {
                    (
                        reference.clone(),
                        artifact_digests
                            .get(reference)
                            .cloned()
                            .unwrap_or_else(|| "missing".to_owned()),
                    )
                })
                .collect();
            let evidence_digest = canonical_digest(&json!({"evidence": evidence}))
                .map_err(|error| DetectionError::new(error.to_string()))?;
            Ok(SignalCandidate {
                signal,
                bindings,
                evidence_refs,
                detector_id: DETECTOR_ID.to_owned(),
                detector_version: DETECTOR_VERSION.to_owned(),
                fingerprint,
                evidence_digest,
            })
        })
        .collect::<Result<Vec<_>, DetectionError>>()?;
    candidates.sort_by(|left, right| {
        (&left.signal, &left.fingerprint).cmp(&(&right.signal, &right.fingerprint))
    });

    // Detector identity is already part of every candidate fingerprint. The
    // report digest also includes the current evidence version.
    let candidate_body: Vec<Value> = candidates
        .iter()
        .map(|candidate| {
            json!({
                "signal": candidate.signal,
                "bindings": candidate.bindings,
                "evidence_refs": candidate.evidence_refs,
                "fingerprint": candidate.fingerprint,
                "evidence_digest": candidate.evidence_digest,
            })
        })
        .collect();
    let report_body = json!({
        "coverage": coverage,
        "candidates": candidate_body,
    });
    let digest =
        canonical_digest(&report_body).map_err(|error| DetectionError::new(error.to_string()))?;
    Ok(DetectionReport {
        change_id: change_id.to_owned(),
        coverage,
        candidates,
        digest,
    })
}

fn signals_for(
    fact: &Map<String, Value>,
    fact_index: usize,
) -> Result<Vec<DetectedSignal>, DetectionError> {
    let kind = fact.get("kind").and_then(Value::as_str).ok_or_else(|| {
        DetectionError::new(format!(
            "repository fact {fact_index} field kind must be a string"
        ))
    })?;
    match kind {
        "db_write" => Ok(vec![(
            "persistent-data-write".to_owned(),
            BTreeMap::from([
                (
                    "operation".to_owned(),
                    required_string(fact, "operation", fact_index)?.to_owned(),
                ),
                (
                    "data".to_owned(),
                    required_string(fact, "data", fact_index)?.to_owned(),
                ),
            ]),
        )]),
        "message_publish" => {
            let bindings = BTreeMap::from([
                (
                    "operation".to_owned(),
                    required_string(fact, "operation", fact_index)?.to_owned(),
                ),
                (
                    "integration".to_owned(),
                    required_string(fact, "integration", fact_index)?.to_owned(),
                ),
            ]);
            Ok(vec![
                ("distributed-effect".to_owned(), bindings.clone()),
                ("message-or-event-publish".to_owned(), bindings),
            ])
        }
        _ => Err(DetectionError::new(format!(
            "repository fact {fact_index} has unsupported kind: {kind}"
        ))),
    }
}

fn detection_coverage(value: &Value) -> Result<DetectionCoverage, DetectionError> {
    if value.is_null() {
        return Ok(DetectionCoverage {
            status: "incomplete".to_owned(),
            scope: "unknown".to_owned(),
            analyzed_refs: Vec::new(),
            gaps: vec![BTreeMap::from([
                ("kind".to_owned(), "coverage-not-reported".to_owned()),
                (
                    "reason".to_owned(),
                    "Detector coverage was not reported".to_owned(),
                ),
            ])],
        });
    }
    let object = value
        .as_object()
        .ok_or_else(|| DetectionError::new("repository coverage must be an object"))?;
    let fields: std::collections::BTreeSet<&str> = object.keys().map(String::as_str).collect();
    if fields != std::collections::BTreeSet::from(["status", "scope", "analyzed_refs", "gaps"]) {
        return Err(DetectionError::new(
            "repository coverage must contain status, scope, analyzed_refs, gaps",
        ));
    }
    let status = object["status"]
        .as_str()
        .filter(|status| matches!(*status, "complete" | "incomplete"))
        .ok_or_else(|| {
            DetectionError::new("repository coverage status must be complete or incomplete")
        })?;
    let scope = object["scope"]
        .as_str()
        .filter(|scope| !scope.is_empty())
        .ok_or_else(|| {
            DetectionError::new("repository coverage scope must be a non-empty string")
        })?;
    let mut analyzed_refs = required_string_array(object, "analyzed_refs", "repository coverage")?;
    analyzed_refs.sort();
    if analyzed_refs.windows(2).any(|items| items[0] == items[1]) {
        return Err(DetectionError::new(
            "repository coverage analyzed_refs must be unique strings",
        ));
    }
    let raw_gaps = object["gaps"]
        .as_array()
        .ok_or_else(|| DetectionError::new("repository coverage gaps must be an array"))?;
    let mut gaps = Vec::new();
    for (gap_index, gap) in raw_gaps.iter().enumerate() {
        let gap = gap.as_object().ok_or_else(|| {
            DetectionError::new(format!(
                "repository coverage gap {gap_index} must be an object"
            ))
        })?;
        let gap_fields: std::collections::BTreeSet<&str> = gap.keys().map(String::as_str).collect();
        let required = std::collections::BTreeSet::from(["kind", "reason"]);
        let allowed = std::collections::BTreeSet::from(["kind", "ref", "reason"]);
        if !required.is_subset(&gap_fields) || !gap_fields.is_subset(&allowed) {
            return Err(DetectionError::new(format!(
                "repository coverage gap {gap_index} must contain kind, optional ref, reason"
            )));
        }
        let mut normalized = BTreeMap::new();
        for (field, item) in gap {
            let item = item
                .as_str()
                .filter(|item| !item.is_empty())
                .ok_or_else(|| {
                    DetectionError::new(format!(
                        "repository coverage gap {gap_index} values must be non-empty strings"
                    ))
                })?;
            normalized.insert(field.clone(), item.to_owned());
        }
        gaps.push(normalized);
    }
    gaps.sort_by(|left, right| {
        (
            &left["kind"],
            left.get("ref").map(String::as_str).unwrap_or(""),
            &left["reason"],
        )
            .cmp(&(
                &right["kind"],
                right.get("ref").map(String::as_str).unwrap_or(""),
                &right["reason"],
            ))
    });
    if status == "complete" && !gaps.is_empty() {
        return Err(DetectionError::new(
            "complete repository coverage cannot contain gaps",
        ));
    }
    if status == "incomplete" && gaps.is_empty() {
        return Err(DetectionError::new(
            "incomplete repository coverage must contain a gap",
        ));
    }
    Ok(DetectionCoverage {
        status: status.to_owned(),
        scope: scope.to_owned(),
        analyzed_refs,
        gaps,
    })
}

fn required_string_array(
    object: &Map<String, Value>,
    field: &str,
    owner: &str,
) -> Result<Vec<String>, DetectionError> {
    object[field]
        .as_array()
        .ok_or_else(|| DetectionError::new(format!("{owner} {field} must be unique strings")))?
        .iter()
        .map(|item| {
            item.as_str().map(str::to_owned).ok_or_else(|| {
                DetectionError::new(format!("{owner} {field} must be unique strings"))
            })
        })
        .collect()
}

fn required_string<'a>(
    fact: &'a Map<String, Value>,
    field: &str,
    fact_index: usize,
) -> Result<&'a str, DetectionError> {
    fact.get(field).and_then(Value::as_str).ok_or_else(|| {
        DetectionError::new(format!(
            "repository fact {fact_index} field {field} must be a string"
        ))
    })
}

fn optional_string_array(
    fact: &Map<String, Value>,
    field: &str,
    fact_index: usize,
) -> Result<Vec<String>, DetectionError> {
    let Some(value) = fact.get(field) else {
        return Ok(Vec::new());
    };
    let values = value.as_array().ok_or_else(|| {
        DetectionError::new(format!(
            "repository fact {fact_index} field {field} must be an array"
        ))
    })?;
    values
        .iter()
        .map(|item| {
            item.as_str().map(str::to_owned).ok_or_else(|| {
                DetectionError::new(format!(
                    "repository fact {fact_index} field {field} items must be strings"
                ))
            })
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetectionError {
    message: String,
}

impl DetectionError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for DetectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for DetectionError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_unknown_fact_kinds() {
        let error = detect_typed_facts(
            "change.docs",
            &[json!({"kind": "file_read"})],
            &json!({
                "status": "complete",
                "scope": "declared-artifacts",
                "analyzed_refs": [],
                "gaps": [],
            }),
            &BTreeMap::new(),
        )
        .unwrap_err();
        assert_eq!(
            error.to_string(),
            "repository fact 0 has unsupported kind: file_read"
        );
    }

    #[test]
    fn rejects_missing_known_fact_binding() {
        let error = detect_typed_facts(
            "change.invalid",
            &[json!({"kind": "db_write", "data": "data.orders"})],
            &Value::Null,
            &BTreeMap::new(),
        )
        .unwrap_err();
        assert_eq!(
            error.to_string(),
            "repository fact 0 field operation must be a string"
        );
    }

    #[test]
    fn keeps_candidate_identity_when_only_evidence_content_changes() {
        let facts = [json!({
            "kind": "db_write",
            "operation": "operation.place-order",
            "data": "data.orders",
            "evidence_refs": ["code.place-order"],
        })];
        let coverage = json!({
            "status": "complete",
            "scope": "declared-artifacts",
            "analyzed_refs": ["code.place-order"],
            "gaps": [],
        });
        let before = detect_typed_facts(
            "change.place-order",
            &facts,
            &coverage,
            &BTreeMap::from([(
                "code.place-order".to_owned(),
                format!("sha256:{}", "1".repeat(64)),
            )]),
        )
        .unwrap();
        let after = detect_typed_facts(
            "change.place-order",
            &facts,
            &coverage,
            &BTreeMap::from([(
                "code.place-order".to_owned(),
                format!("sha256:{}", "9".repeat(64)),
            )]),
        )
        .unwrap();

        assert_eq!(
            before.candidates[0].fingerprint,
            after.candidates[0].fingerprint
        );
        assert_ne!(
            before.candidates[0].evidence_digest,
            after.candidates[0].evidence_digest
        );
    }

    #[test]
    fn creates_a_new_candidate_identity_when_a_binding_changes() {
        let coverage = json!({
            "status": "complete",
            "scope": "declared-artifacts",
            "analyzed_refs": ["code.place-order"],
            "gaps": [],
        });
        let artifact_digests = BTreeMap::from([(
            "code.place-order".to_owned(),
            format!("sha256:{}", "1".repeat(64)),
        )]);
        let before = detect_typed_facts(
            "change.place-order",
            &[json!({
                "kind": "db_write",
                "operation": "operation.place-order",
                "data": "data.orders",
                "evidence_refs": ["code.place-order"],
            })],
            &coverage,
            &artifact_digests,
        )
        .unwrap();
        let after = detect_typed_facts(
            "change.place-order",
            &[
                json!({
                    "kind": "db_write",
                    "operation": "operation.place-order",
                    "data": "data.orders",
                    "evidence_refs": ["code.place-order"],
                }),
                json!({
                    "kind": "db_write",
                    "operation": "operation.place-order",
                    "data": "data.customers",
                    "evidence_refs": ["code.place-order"],
                }),
            ],
            &coverage,
            &artifact_digests,
        )
        .unwrap();

        assert_eq!(before.candidates.len(), 1);
        assert_eq!(after.candidates.len(), 2);
        assert!(after.candidates.iter().any(|candidate| {
            candidate.fingerprint == before.candidates[0].fingerprint
                && candidate.bindings["data"] == "data.orders"
        }));
        assert!(after.candidates.iter().any(|candidate| {
            candidate.fingerprint != before.candidates[0].fingerprint
                && candidate.bindings["data"] == "data.customers"
        }));
    }
}
