//! Read-only explanation of one Thin Kernel decision.
//!
//! An Explain Report is derived from the same Snapshot, Rule Index, Detection
//! Report, and Kernel Decision used by `next`. It is not authoritative state
//! and deliberately contains references and short reasons instead of copying
//! full Context, Contract text, or Agent conversations.

use crate::detection::{DetectionReport, SignalCandidate};
use crate::kernel::{
    KernelDecision, ProjectSnapshot, RequirementInstance, outcome_has_current_evidence,
};
use crate::rules::{Assurance, RuleIndex};
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;

pub const EXPLAIN_REPORT_SCHEMA_VERSION: &str = "1";

#[derive(Debug, Clone, PartialEq, Eq)]
struct CandidateDispositionTrace {
    status: String,
    input_refs: BTreeMap<String, String>,
    result_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CandidateTrace {
    pub fingerprint: String,
    pub signal: String,
    pub bindings: BTreeMap<String, String>,
    pub evidence_refs: Vec<String>,
    pub disposition: String,
    pub disposition_result_id: Option<String>,
    pub applied_rules: Vec<String>,
    pub requirement_instances: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ResultCheck {
    pub result_id: String,
    pub outcome_status: Option<String>,
    pub definition_matches: bool,
    pub result_schema_matches: bool,
    pub role_matches: bool,
    pub stale_refs: Vec<String>,
    pub accepted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RequirementTrace {
    pub instance_key: String,
    pub requirement_id: String,
    pub subject_refs: Vec<String>,
    pub selected_by: Vec<String>,
    pub status: String,
    #[serde(default, skip_serializing_if = "Assurance::is_attestation")]
    pub assurance: Assurance,
    pub satisfaction_basis: String,
    pub blocked_by: Vec<String>,
    pub result_checks: Vec<ResultCheck>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AuthorityTrace {
    pub request_id: String,
    pub request_result_id: String,
    pub request_stale_refs: Vec<String>,
    pub status: String,
    pub answer_result_ids: Vec<String>,
    pub decision_ids: Vec<String>,
    pub contract_clause_refs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NextActionTrace {
    pub id: String,
    pub role: String,
    pub action: String,
    pub reason: String,
    pub requirement_instances: Vec<String>,
    pub candidate_fingerprints: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExplainReport {
    pub schema_version: String,
    pub change_id: String,
    pub state: String,
    pub candidates: Vec<CandidateTrace>,
    pub requirements: Vec<RequirementTrace>,
    pub authority: Vec<AuthorityTrace>,
    pub next_action: Option<NextActionTrace>,
    pub diagnostics: Vec<String>,
}

impl ExplainReport {
    /// Return the language-neutral representation checked by shared golden data.
    pub fn as_value(&self) -> Value {
        serde_json::to_value(self).expect("Explain Report contains serializable fields")
    }

    /// Render only the compact trace; source documents remain available by ID.
    pub fn render_text(&self) -> String {
        let mut lines = vec![
            format!("change: {}", self.change_id),
            format!("state: {}", self.state),
        ];
        match &self.next_action {
            Some(action) => lines.push(format!(
                "next: {}/{} ({})",
                action.role, action.action, action.reason
            )),
            None => lines.push("next: none".to_owned()),
        }

        lines.push("candidates:".to_owned());
        for candidate in &self.candidates {
            lines.push(format!(
                "  - {} [{}] {} rules={}",
                candidate.signal,
                candidate.disposition,
                candidate.fingerprint,
                joined_or_dash(&candidate.applied_rules)
            ));
        }

        lines.push("requirements:".to_owned());
        for requirement in &self.requirements {
            lines.push(format!(
                "  - {} [{}] rules={} blocked_by={}",
                requirement.instance_key,
                requirement.status,
                joined_or_dash(&requirement.selected_by),
                joined_or_dash(&requirement.blocked_by)
            ));
            for result in &requirement.result_checks {
                lines.push(format!(
                    "      result={} accepted={} stale_refs={}",
                    result.result_id,
                    result.accepted,
                    joined_or_dash(&result.stale_refs)
                ));
            }
        }

        if !self.authority.is_empty() {
            lines.push("authority:".to_owned());
            for request in &self.authority {
                lines.push(format!(
                    "  - {} [{}] answers={} decisions={}",
                    request.request_id,
                    request.status,
                    request.answer_result_ids.len(),
                    request.decision_ids.len()
                ));
            }
        }
        lines.join("\n") + "\n"
    }
}

pub struct ExplanationBuilder;

impl ExplanationBuilder {
    /// Build a trace without changing Records or issuing an Action.
    pub fn build(
        &self,
        snapshot: &ProjectSnapshot,
        rule_index: &RuleIndex,
        detection: &DetectionReport,
        decision: &KernelDecision,
    ) -> ExplainReport {
        let dispositions = candidate_dispositions(snapshot);
        let candidates = detection
            .candidates
            .iter()
            .map(|candidate| {
                let disposition = dispositions
                    .get(&candidate.fingerprint)
                    .cloned()
                    .filter(|disposition| {
                        disposition.status == "confirmed"
                            || candidate.evidence_refs.iter().all(|reference| {
                                snapshot.artifact_digests.get(reference)
                                    == disposition.input_refs.get(reference)
                                    && disposition.input_refs.contains_key(reference)
                            })
                    })
                    .map(|disposition| (disposition.status, disposition.result_id))
                    .unwrap_or_else(|| ("unreviewed".to_owned(), None));
                candidate_trace(candidate, disposition, rule_index, decision, snapshot)
            })
            .collect();
        let requirements = decision
            .requirement_instances
            .iter()
            .map(|instance| requirement_trace(instance, decision, snapshot))
            .collect();
        let next_action = decision.action.as_ref().map(|action| NextActionTrace {
            id: action.id.clone(),
            role: action.role.clone(),
            action: action.action.clone(),
            reason: action.reason.clone(),
            requirement_instances: action
                .requirement_instances
                .iter()
                .map(|instance| instance.instance_key.clone())
                .collect(),
            candidate_fingerprints: action.candidate_fingerprints.clone(),
        });
        ExplainReport {
            schema_version: EXPLAIN_REPORT_SCHEMA_VERSION.to_owned(),
            change_id: snapshot.change_id.clone(),
            state: decision.state.clone(),
            candidates,
            requirements,
            authority: authority_trace(snapshot),
            next_action,
            diagnostics: decision.diagnostics.clone(),
        }
    }
}

fn candidate_dispositions(
    snapshot: &ProjectSnapshot,
) -> BTreeMap<String, CandidateDispositionTrace> {
    let mut dispositions = BTreeMap::new();
    for result in &snapshot.results {
        if string_field(result, "result_schema") != Some("result.risk-signal-review")
            || string_field(result, "role") != Some("Analyst")
        {
            continue;
        }
        for review in nested_array(result, &["payload", "reviewed_candidates"]) {
            if let (Some(fingerprint), Some(status)) = (
                string_field(review, "fingerprint"),
                string_field(review, "status"),
            ) {
                dispositions.insert(
                    fingerprint.to_owned(),
                    CandidateDispositionTrace {
                        status: status.to_owned(),
                        input_refs: string_map(result.get("input_refs")),
                        result_id: string_field(result, "id").map(str::to_owned),
                    },
                );
            }
        }
    }
    dispositions
}

fn string_map(value: Option<&Value>) -> BTreeMap<String, String> {
    value
        .and_then(Value::as_object)
        .map(|items| {
            items
                .iter()
                .filter_map(|(key, value)| {
                    value.as_str().map(|value| (key.clone(), value.to_owned()))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn candidate_trace(
    candidate: &SignalCandidate,
    disposition: (String, Option<String>),
    rule_index: &RuleIndex,
    decision: &KernelDecision,
    snapshot: &ProjectSnapshot,
) -> CandidateTrace {
    let (original_status, result_id) = disposition;
    let status = effective_candidate_disposition(snapshot, candidate, &original_status, decision);
    let repository_phase = snapshot.repository.get("phase").and_then(Value::as_str);
    let mut applied_rules: Vec<String> = rule_index
        .rules
        .iter()
        .filter(|rule| {
            status == "confirmed"
                && rule.condition == "signal"
                && rule.signal.as_deref() == Some(candidate.signal.as_str())
                && (rule.repository_phase.is_none()
                    || rule.repository_phase.as_deref() == repository_phase)
        })
        .map(|rule| rule.id.clone())
        .collect();
    applied_rules.sort();
    let mut requirement_instances: Vec<String> = decision
        .requirement_instances
        .iter()
        .filter(|instance| {
            instance
                .selected_by
                .iter()
                .any(|rule| applied_rules.contains(rule))
        })
        .map(|instance| instance.instance_key.clone())
        .collect();
    requirement_instances.sort();
    CandidateTrace {
        fingerprint: candidate.fingerprint.clone(),
        signal: candidate.signal.clone(),
        bindings: candidate.bindings.clone(),
        evidence_refs: candidate.evidence_refs.clone(),
        disposition: status,
        disposition_result_id: result_id,
        applied_rules,
        requirement_instances,
    }
}

fn effective_candidate_disposition(
    snapshot: &ProjectSnapshot,
    candidate: &SignalCandidate,
    original_status: &str,
    decision: &KernelDecision,
) -> String {
    if original_status != "not-applicable" {
        return original_status.to_owned();
    }
    let instance_key = format!(
        "risk-signal-applicability-reviewed|{}|{}",
        candidate.fingerprint, candidate.evidence_digest
    );
    let Some(instance) = decision
        .requirement_instances
        .iter()
        .find(|instance| instance.instance_key == instance_key)
    else {
        return "applicability-pending".to_owned();
    };
    let review_status = snapshot.results.iter().find_map(|result| {
        if string_field(result, "result_schema") != Some("result.challenge")
            || string_field(result, "role") != Some("Challenger")
        {
            return None;
        }
        nested_array(result, &["payload", "outcomes"])
            .iter()
            .find(|outcome| {
                string_field(outcome, "instance_key") == Some(instance.instance_key.as_str())
                    && string_field(outcome, "definition_digest")
                        == Some(instance.definition_digest.as_str())
                    && stale_refs_for_outcome(outcome, result, snapshot).is_empty()
            })
            .and_then(|outcome| string_field(outcome, "status"))
    });
    match review_status {
        Some("satisfied") => "not-applicable".to_owned(),
        Some("unsatisfied" | "inconclusive") => "confirmed".to_owned(),
        _ => "applicability-pending".to_owned(),
    }
}

fn requirement_trace(
    instance: &RequirementInstance,
    decision: &KernelDecision,
    snapshot: &ProjectSnapshot,
) -> RequirementTrace {
    let mut result_checks = Vec::new();
    for result in &snapshot.results {
        for outcome in nested_array(result, &["payload", "outcomes"]) {
            if string_field(outcome, "instance_key") != Some(instance.instance_key.as_str()) {
                continue;
            }
            let stale_refs = stale_refs_for_outcome(outcome, result, snapshot);
            let definition_matches = string_field(outcome, "definition_digest")
                == Some(instance.definition_digest.as_str());
            let result_schema_matches =
                string_field(result, "result_schema") == Some(instance.result_schema.as_str());
            let role_matches = string_field(result, "role") == Some(instance.role.as_str());
            let outcome_status = string_field(outcome, "status").map(str::to_owned);
            let outcome_completes_review = outcome_status.as_deref() == Some("satisfied")
                || (instance.requirement_id == "risk-signal-applicability-reviewed"
                    && matches!(
                        outcome_status.as_deref(),
                        Some("unsatisfied" | "inconclusive")
                    ));
            let accepted = outcome_completes_review
                && definition_matches
                && result_schema_matches
                && role_matches
                && stale_refs.is_empty()
                && (instance.assurance == Assurance::Attestation
                    || outcome_has_current_evidence(instance, outcome, snapshot));
            result_checks.push(ResultCheck {
                result_id: string_field(result, "id").unwrap_or("").to_owned(),
                outcome_status,
                definition_matches,
                result_schema_matches,
                role_matches,
                stale_refs,
                accepted,
            });
        }
    }

    let mut blocked_by: Vec<String> = instance
        .depends_on
        .iter()
        .flat_map(|dependency| {
            decision
                .requirement_instances
                .iter()
                .filter(|candidate| {
                    candidate.requirement_id == *dependency && candidate.status != "satisfied"
                })
                .map(|candidate| candidate.instance_key.clone())
        })
        .collect();
    blocked_by.sort();
    let satisfaction_basis = if instance.requirement_id == "risk-signals-reviewed"
        && instance.status == "satisfied"
    {
        "all-current-candidates-reviewed"
    } else if instance.requirement_id == "risk-signal-applicability-reviewed"
        && instance.status == "satisfied"
    {
        "completed-signal-applicability-review"
    } else if instance.status == "satisfied" && instance.assurance == Assurance::EvidenceBacked {
        "current-evidence-recorded"
    } else if instance.status == "satisfied" {
        "accepted-result-outcome"
    } else if instance.assurance == Assurance::EvidenceBacked {
        "no-current-evidence"
    } else {
        "no-fresh-satisfying-result"
    };
    RequirementTrace {
        instance_key: instance.instance_key.clone(),
        requirement_id: instance.requirement_id.clone(),
        subject_refs: instance.subject_refs.clone(),
        selected_by: instance.selected_by.clone(),
        status: instance.status.clone(),
        assurance: instance.assurance,
        satisfaction_basis: satisfaction_basis.to_owned(),
        blocked_by,
        result_checks,
    }
}

fn authority_trace(snapshot: &ProjectSnapshot) -> Vec<AuthorityTrace> {
    let mut requests: BTreeMap<String, (String, Vec<String>)> = BTreeMap::new();
    for result in &snapshot.results {
        for request in nested_array(result, &["payload", "decision_requests"]) {
            if let Some(request_id) = string_field(request, "id") {
                requests.insert(
                    request_id.to_owned(),
                    (
                        string_field(result, "id").unwrap_or("").to_owned(),
                        record_stale_refs(result, snapshot),
                    ),
                );
            }
        }
    }

    requests
        .into_iter()
        .map(|(request_id, (request_result_id, request_stale_refs))| {
            let mut answer_result_ids: Vec<String> = snapshot
                .results
                .iter()
                .filter(|result| {
                    string_field(result, "result_schema") == Some("result.human-answer")
                        && string_field(result, "role") == Some("Human")
                        && record_stale_refs(result, snapshot).is_empty()
                        && nested_array(result, &["payload", "answers"])
                            .iter()
                            .any(|answer| {
                                string_field(answer, "request_id") == Some(request_id.as_str())
                            })
                })
                .filter_map(|result| string_field(result, "id").map(str::to_owned))
                .collect();
            answer_result_ids.sort();

            let mut decision_ids: Vec<String> = snapshot
                .decisions
                .iter()
                .filter(|decision| {
                    string_field(decision, "status") == Some("accepted")
                        && decision
                            .get("resolves")
                            .and_then(Value::as_array)
                            .is_some_and(|resolves| {
                                resolves
                                    .iter()
                                    .any(|resolved| resolved.as_str() == Some(request_id.as_str()))
                            })
                })
                .filter_map(|decision| string_field(decision, "id").map(str::to_owned))
                .collect();
            decision_ids.sort();

            let mut contract_clause_refs = Vec::new();
            for contract in &snapshot.contracts {
                let Some(contract_id) = string_field(contract, "id") else {
                    continue;
                };
                for clause in nested_array(contract, &["clauses"]) {
                    if string_field(clause, "authority_ref")
                        .is_some_and(|reference| decision_ids.iter().any(|id| id == reference))
                        && let Some(clause_id) = string_field(clause, "id")
                    {
                        contract_clause_refs.push(format!("{contract_id}#{clause_id}"));
                    }
                }
            }
            contract_clause_refs.sort();

            // Recording into an accepted Decision and Contract is terminal.
            // Stale request refs remain visible as detail without making a
            // completed authority flow appear open again.
            let status = if !contract_clause_refs.is_empty() {
                "recorded"
            } else if !request_stale_refs.is_empty() {
                "stale-request"
            } else if !answer_result_ids.is_empty() {
                "answered-not-recorded"
            } else {
                "open"
            };
            AuthorityTrace {
                request_id,
                request_result_id,
                request_stale_refs,
                status: status.to_owned(),
                answer_result_ids,
                decision_ids,
                contract_clause_refs,
            }
        })
        .collect()
}

fn stale_refs_for_outcome(
    outcome: &Value,
    result: &Value,
    snapshot: &ProjectSnapshot,
) -> Vec<String> {
    let refs = outcome
        .get("freshness_refs")
        .or_else(|| result.get("freshness_refs"))
        .or_else(|| result.get("input_refs"));
    stale_refs(refs, snapshot)
}

fn record_stale_refs(result: &Value, snapshot: &ProjectSnapshot) -> Vec<String> {
    stale_refs(
        result
            .get("freshness_refs")
            .or_else(|| result.get("input_refs")),
        snapshot,
    )
}

fn stale_refs(refs: Option<&Value>, snapshot: &ProjectSnapshot) -> Vec<String> {
    let Some(refs) = refs.and_then(Value::as_object) else {
        return vec!["<invalid-freshness-refs>".to_owned()];
    };
    let mut stale: Vec<String> = refs
        .iter()
        .filter(|(reference, digest)| {
            snapshot
                .artifact_digests
                .get(*reference)
                .map(String::as_str)
                != digest.as_str()
        })
        .map(|(reference, _)| reference.clone())
        .collect();
    stale.sort();
    stale
}

fn nested_array<'a>(value: &'a Value, path: &[&str]) -> &'a [Value] {
    let mut current = value;
    for field in path {
        let Some(next) = current.get(*field) else {
            return &[];
        };
        current = next;
    }
    current.as_array().map(Vec::as_slice).unwrap_or(&[])
}

fn string_field<'a>(value: &'a Value, field: &str) -> Option<&'a str> {
    value.get(field).and_then(Value::as_str)
}

fn joined_or_dash(values: &[String]) -> String {
    if values.is_empty() {
        "-".to_owned()
    } else {
        values.join(",")
    }
}
