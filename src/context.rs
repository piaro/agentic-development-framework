//! Deterministic projection of a Next Action into working Context.
//!
//! Generated Context is disposable. Persisted Results retain source IDs and
//! digests, while the full payload can be regenerated from authoritative input.

use crate::canonical_digest;
use crate::contract_health::ContractHealthReport;
use crate::contract_scope::{
    clause_matches_subjects, contract_matches_subjects, contract_uses_clause_scopes,
    effective_clause_applies_to,
};
use crate::detection::DetectionReport;
use crate::kernel::{KernelDecision, NextAction, ProjectSnapshot, RequirementInstance};
use crate::rules::Assurance;
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

pub const CONTEXT_COMPILER_VERSION: &str = "4";
pub const CONTRACT_CLAUSE_PROJECTION_VERSION: &str = "1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ContractClauseContext {
    pub contract_id: String,
    pub clause_id: String,
    pub clause_ref: String,
    pub text: String,
    pub applies_to: Vec<String>,
    pub authority_ref: Option<String>,
    pub digest: String,
    pub selected_for: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct GeneratedContext {
    pub action_id: String,
    pub role: String,
    pub source_refs: Vec<String>,
    pub source_digests: BTreeMap<String, String>,
    pub instance_source_digests: BTreeMap<String, BTreeMap<String, String>>,
    pub contract_clause_projection_version: String,
    pub contract_clauses: Vec<ContractClauseContext>,
    pub contract_clauses_digest: String,
    pub payload: Value,
    pub digest: String,
}

impl GeneratedContext {
    pub fn compatibility_checkpoint(&self) -> Value {
        let instance_source_refs: BTreeMap<&str, Vec<&str>> = self
            .instance_source_digests
            .iter()
            .map(|(instance_key, sources)| {
                (
                    instance_key.as_str(),
                    sources.keys().map(String::as_str).collect(),
                )
            })
            .collect();
        let signal_candidate_fingerprints = self.payload["signal_candidates"]
            .as_array()
            .expect("Context payload signal candidates are an array")
            .iter()
            .filter_map(|candidate| candidate["fingerprint"].as_str())
            .collect::<Vec<_>>();
        json!({
            "action_id": self.action_id,
            "role": self.role,
            "source_refs": self.source_refs,
            "instance_source_refs": instance_source_refs,
            "signal_candidate_fingerprints": signal_candidate_fingerprints,
            "digest": self.digest,
        })
    }
}

pub struct ContextCompiler;

impl ContextCompiler {
    pub fn compile(
        &self,
        decision: &KernelDecision,
        snapshot: &ProjectSnapshot,
        detection: &DetectionReport,
    ) -> Option<GeneratedContext> {
        self.compile_with_health(decision, snapshot, detection, None)
    }

    pub fn compile_with_health(
        &self,
        decision: &KernelDecision,
        snapshot: &ProjectSnapshot,
        detection: &DetectionReport,
        contract_health: Option<&ContractHealthReport>,
    ) -> Option<GeneratedContext> {
        let action = decision.action.as_ref()?;
        let risk_review_only = action.action == "review-risk-signals"
            && action
                .requirement_instances
                .iter()
                .map(|instance| instance.requirement_id.as_str())
                .collect::<BTreeSet<_>>()
                == BTreeSet::from(["risk-signals-reviewed"]);

        let instance_source_refs: BTreeMap<String, Vec<String>> = action
            .requirement_instances
            .iter()
            .map(|instance| {
                (
                    instance.instance_key.clone(),
                    self.instance_source_refs(instance, &decision.requirement_instances, snapshot),
                )
            })
            .collect();
        let mut source_refs: Vec<String> = if instance_source_refs.is_empty() {
            self.action_source_refs(&action.action, snapshot)
        } else {
            instance_source_refs
                .values()
                .flatten()
                .cloned()
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect()
        };
        if risk_review_only {
            source_refs.extend(
                detection
                    .candidates
                    .iter()
                    .filter(|candidate| {
                        action
                            .candidate_fingerprints
                            .contains(&candidate.fingerprint)
                    })
                    .flat_map(|candidate| candidate.evidence_refs.iter().cloned()),
            );
            source_refs.sort();
            source_refs.dedup();
        }
        let source_digests = source_refs
            .iter()
            .filter_map(|reference| {
                snapshot
                    .artifact_digests
                    .get(reference)
                    .map(|digest| (reference.clone(), digest.clone()))
            })
            .collect::<BTreeMap<_, _>>();
        let instance_source_digests = instance_source_refs
            .iter()
            .map(|(instance_key, references)| {
                let digests = references
                    .iter()
                    .filter_map(|reference| {
                        snapshot
                            .artifact_digests
                            .get(reference)
                            .map(|digest| (reference.clone(), digest.clone()))
                    })
                    .collect();
                (instance_key.clone(), digests)
            })
            .collect::<BTreeMap<_, _>>();
        let contract_clauses = self.contract_clause_contexts(action, snapshot, &source_refs);
        let contract_clauses_digest = canonical_digest(
            &serde_json::to_value(&contract_clauses)
                .expect("Contract clause Context is serializable"),
        )
        .expect("Contract clause Context contains no floating-point values");

        let requirement_instances = action
            .requirement_instances
            .iter()
            .map(|item| {
                let mut value = json!({
                    "requirement_id": item.requirement_id,
                    "instance_key": item.instance_key,
                    "subject_refs": item.subject_refs,
                    "definition_digest": item.definition_digest,
                    "selected_by": item.selected_by,
                    "context_selectors": item.context,
                    "sources": instance_source_digests[&item.instance_key],
                });
                if item.assurance != Assurance::Attestation {
                    value
                        .as_object_mut()
                        .expect("Requirement Context is an object")
                        .insert(
                            "assurance".to_owned(),
                            Value::String(item.assurance.as_str().to_owned()),
                        );
                }
                value
            })
            .collect::<Vec<_>>();
        let signal_candidates = if risk_review_only {
            detection
                .candidates
                .iter()
                .filter(|candidate| {
                    action
                        .candidate_fingerprints
                        .contains(&candidate.fingerprint)
                })
                .map(|candidate| {
                    json!({
                        "signal": candidate.signal,
                        "bindings": candidate.bindings,
                        "evidence_refs": candidate.evidence_refs,
                        "fingerprint": candidate.fingerprint,
                        "evidence_digest": candidate.evidence_digest,
                    })
                })
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        let mut payload = json!({
            "change": snapshot.change,
            "action": {
                "id": action.id,
                "role": action.role,
                "action": action.action,
                "reason": action.reason,
                "expected_result_schema": action.expected_result_schema,
                "candidate_fingerprints": action.candidate_fingerprints,
            },
            "requirement_instances": requirement_instances,
            "signal_candidates": signal_candidates,
            "sources": source_digests,
        });
        if action
            .requirement_instances
            .iter()
            .any(|instance| instance.requirement_id == "risk-signal-applicability-reviewed")
        {
            let reviewed_fingerprints = action
                .requirement_instances
                .iter()
                .flat_map(|instance| instance.subject_refs.iter())
                .collect::<BTreeSet<_>>();
            let candidates = detection
                .candidates
                .iter()
                .filter(|candidate| reviewed_fingerprints.contains(&candidate.fingerprint))
                .filter_map(|candidate| {
                    not_applicable_disposition(snapshot, candidate).map(
                        |(result_id, reason, basis_refs)| {
                            json!({
                                "signal": candidate.signal,
                                "bindings": candidate.bindings,
                                "evidence_refs": candidate.evidence_refs,
                                "fingerprint": candidate.fingerprint,
                                "evidence_digest": candidate.evidence_digest,
                                "disposition": "not-applicable",
                                "reason": reason,
                                "basis_refs": basis_refs,
                                "disposition_result_id": result_id,
                            })
                        },
                    )
                })
                .collect::<Vec<_>>();
            payload
                .as_object_mut()
                .expect("Context payload is an object")
                .insert(
                    "not_applicable_signal_candidates".to_owned(),
                    Value::Array(candidates),
                );
        }
        if let Some(contract_health) = contract_health {
            let revalidation_instances = action
                .requirement_instances
                .iter()
                .filter(|instance| instance.requirement_id == "contract-clause-revalidated")
                .collect::<Vec<_>>();
            if !revalidation_instances.is_empty() {
                let findings = contract_health
                    .clauses
                    .iter()
                    .filter_map(|clause| {
                        let selected_for = revalidation_instances
                            .iter()
                            .filter(|instance| {
                                instance
                                    .subject_refs
                                    .iter()
                                    .any(|subject| subject == &clause.clause_ref)
                            })
                            .map(|instance| instance.instance_key.as_str())
                            .collect::<Vec<_>>();
                        (!selected_for.is_empty()).then(|| {
                            json!({
                                "clause_ref": clause.clause_ref,
                                "status": clause.status,
                                "stale_refs": clause.stale_refs,
                                "evidence_refs": clause.evidence_refs,
                                "selected_for": selected_for,
                            })
                        })
                    })
                    .collect::<Vec<_>>();
                payload
                    .as_object_mut()
                    .expect("Context payload is an object")
                    .insert(
                        "contract_health_findings".to_owned(),
                        Value::Array(findings),
                    );
            }
        }
        let digest =
            canonical_digest(&payload).expect("Context payload contains no floating-point values");
        Some(GeneratedContext {
            action_id: action.id.clone(),
            role: action.role.clone(),
            source_refs,
            source_digests,
            instance_source_digests,
            contract_clause_projection_version: CONTRACT_CLAUSE_PROJECTION_VERSION.to_owned(),
            contract_clauses,
            contract_clauses_digest,
            payload,
            digest,
        })
    }

    fn instance_source_refs(
        &self,
        instance: &RequirementInstance,
        all_instances: &[RequirementInstance],
        snapshot: &ProjectSnapshot,
    ) -> Vec<String> {
        let selectors: BTreeSet<&str> = instance.context.iter().map(String::as_str).collect();
        let mut references = BTreeSet::new();
        if selectors.contains("change") {
            references.insert(snapshot.change_id.clone());
        }
        if selectors.contains("repository-artifacts") {
            references.extend(
                array_field(&snapshot.repository, "artifacts")
                    .iter()
                    .filter_map(value_id),
            );
        }
        if selectors.contains("affected-code") {
            references.extend(
                array_field(&snapshot.repository, "artifacts")
                    .iter()
                    .filter(|artifact| matches_subjects(artifact, &instance.subject_refs))
                    .filter_map(value_id),
            );
        }
        let subjects = instance
            .subject_refs
            .iter()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        let matching_contracts: Vec<&Value> = snapshot
            .contracts
            .iter()
            .filter(|contract| contract_matches_subjects(contract, &subjects))
            .collect();
        if selectors.contains("matching-contracts") {
            for contract in &matching_contracts {
                if contract_uses_clause_scopes(contract) {
                    references.extend(
                        matching_clauses(contract, &instance.subject_refs)
                            .iter()
                            .filter_map(|clause| clause_ref(contract, clause)),
                    );
                } else if let Some(contract_id) = value_id(contract) {
                    references.insert(contract_id);
                }
            }
        }
        if selectors.contains("matching-decisions") {
            let authority_refs: BTreeSet<&str> = matching_contracts
                .iter()
                .flat_map(|contract| matching_clauses(contract, &instance.subject_refs))
                .filter_map(|clause| clause["authority_ref"].as_str())
                .collect();
            references.extend(
                snapshot
                    .decisions
                    .iter()
                    .filter(|decision| {
                        decision["id"]
                            .as_str()
                            .is_some_and(|id| authority_refs.contains(id))
                    })
                    .filter_map(value_id),
            );
        }
        if selectors.contains("dependency-results") {
            let dependency_keys: BTreeSet<&str> = all_instances
                .iter()
                .filter(|candidate| instance.depends_on.contains(&candidate.requirement_id))
                .map(|candidate| candidate.instance_key.as_str())
                .collect();
            references.extend(
                snapshot
                    .results
                    .iter()
                    .filter(|result| {
                        result["payload"]["outcomes"]
                            .as_array()
                            .is_some_and(|outcomes| {
                                outcomes.iter().any(|outcome| {
                                    outcome["instance_key"]
                                        .as_str()
                                        .is_some_and(|key| dependency_keys.contains(key))
                                })
                            })
                    })
                    .filter_map(value_id),
            );
        }
        if selectors.contains("matching-evidence") {
            references.extend(
                snapshot
                    .evidence
                    .iter()
                    .filter(|evidence| {
                        matches_subjects(evidence, &instance.subject_refs)
                            || array_field(evidence, "requirement_instances")
                                .iter()
                                .any(|value| value.as_str() == Some(&instance.instance_key))
                    })
                    .filter_map(value_id),
            );
        }
        if selectors.contains("contracts") {
            references.extend(snapshot.contracts.iter().filter_map(value_id));
        }
        if selectors.contains("decisions") {
            references.extend(snapshot.decisions.iter().filter_map(value_id));
        }
        if selectors.contains("results") {
            references.extend(snapshot.results.iter().filter_map(value_id));
        }
        if selectors.contains("evidence") {
            references.extend(snapshot.evidence.iter().filter_map(value_id));
        }
        references.into_iter().collect()
    }

    fn contract_clause_contexts(
        &self,
        action: &NextAction,
        snapshot: &ProjectSnapshot,
        source_refs: &[String],
    ) -> Vec<ContractClauseContext> {
        let mut selected: BTreeMap<String, (ContractClauseContext, BTreeSet<String>)> =
            BTreeMap::new();
        for instance in &action.requirement_instances {
            let selectors = instance
                .context
                .iter()
                .map(String::as_str)
                .collect::<BTreeSet<_>>();
            let subjects = instance
                .subject_refs
                .iter()
                .map(String::as_str)
                .collect::<BTreeSet<_>>();
            if !selectors.contains("matching-contracts")
                && !selectors.contains("matching-decisions")
            {
                continue;
            }
            for contract in snapshot
                .contracts
                .iter()
                .filter(|contract| contract_matches_subjects(contract, &subjects))
            {
                for clause in matching_clauses(contract, &instance.subject_refs) {
                    insert_clause_context(
                        &mut selected,
                        contract,
                        clause,
                        snapshot,
                        &instance.instance_key,
                    );
                }
            }
        }

        if action.requirement_instances.is_empty() {
            let source_refs = source_refs
                .iter()
                .map(String::as_str)
                .collect::<BTreeSet<_>>();
            for contract in &snapshot.contracts {
                let Some(contract_id) = contract["id"].as_str() else {
                    continue;
                };
                for clause in array_field(contract, "clauses") {
                    let Some(clause_ref) = clause_ref(contract, clause) else {
                        continue;
                    };
                    if source_refs.contains(contract_id)
                        || source_refs.contains(clause_ref.as_str())
                    {
                        insert_clause_context(
                            &mut selected,
                            contract,
                            clause,
                            snapshot,
                            &action.id,
                        );
                    }
                }
            }
        }

        selected
            .into_values()
            .map(|(mut clause, selected_for)| {
                clause.selected_for = selected_for.into_iter().collect();
                clause
            })
            .collect()
    }

    fn action_source_refs(&self, action: &str, snapshot: &ProjectSnapshot) -> Vec<String> {
        let mut references = BTreeSet::from([snapshot.change_id.clone()]);
        if action == "answer-decision-request" {
            references.extend(snapshot.results.iter().filter_map(value_id));
        } else if action == "implement-change" {
            references.extend(snapshot.contracts.iter().filter_map(value_id));
            references.extend(snapshot.decisions.iter().filter_map(value_id));
            references.extend(snapshot.results.iter().filter_map(value_id));
            references.extend(
                array_field(&snapshot.repository, "artifacts")
                    .iter()
                    .filter_map(value_id),
            );
        }
        references.into_iter().collect()
    }
}

fn matching_clauses<'a>(contract: &'a Value, subject_refs: &[String]) -> Vec<&'a Value> {
    let subjects = subject_refs
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let contract_id = contract["id"].as_str().unwrap_or_default();
    let explicit_clause_refs = subjects
        .iter()
        .filter(|subject| subject.contains('#'))
        .copied()
        .collect::<BTreeSet<_>>();
    array_field(contract, "clauses")
        .iter()
        .filter(|clause| {
            if explicit_clause_refs.is_empty() {
                clause_matches_subjects(contract, clause, &subjects)
            } else {
                clause["id"].as_str().is_some_and(|clause_id| {
                    explicit_clause_refs.contains(format!("{contract_id}#{clause_id}").as_str())
                })
            }
        })
        .collect()
}

fn clause_ref(contract: &Value, clause: &Value) -> Option<String> {
    Some(format!(
        "{}#{}",
        contract["id"].as_str()?,
        clause["id"].as_str()?
    ))
}

fn insert_clause_context(
    selected: &mut BTreeMap<String, (ContractClauseContext, BTreeSet<String>)>,
    contract: &Value,
    clause: &Value,
    snapshot: &ProjectSnapshot,
    selected_for: &str,
) {
    let (Some(contract_id), Some(clause_id), Some(text), Some(clause_ref)) = (
        contract["id"].as_str(),
        clause["id"].as_str(),
        clause["text"].as_str(),
        clause_ref(contract, clause),
    ) else {
        return;
    };
    let digest = snapshot
        .artifact_digests
        .get(&clause_ref)
        .cloned()
        .unwrap_or_else(|| {
            canonical_digest(clause)
                .expect("validated Contract clause contains no floating-point values")
        });
    let entry = selected.entry(clause_ref.clone()).or_insert_with(|| {
        (
            ContractClauseContext {
                contract_id: contract_id.to_owned(),
                clause_id: clause_id.to_owned(),
                clause_ref,
                text: text.to_owned(),
                applies_to: effective_clause_applies_to(contract, clause),
                authority_ref: clause["authority_ref"].as_str().map(str::to_owned),
                digest,
                selected_for: Vec::new(),
            },
            BTreeSet::new(),
        )
    });
    entry.1.insert(selected_for.to_owned());
}

fn matches_subjects(source: &Value, subject_refs: &[String]) -> bool {
    let applies_to: BTreeSet<&str> = array_field(source, "applies_to")
        .iter()
        .filter_map(Value::as_str)
        .collect();
    subject_refs
        .iter()
        .any(|subject| applies_to.contains(subject.as_str()))
}

fn value_id(value: &Value) -> Option<String> {
    value["id"]
        .as_str()
        .or_else(|| value["ref"].as_str())
        .map(str::to_owned)
}

fn not_applicable_disposition<'a>(
    snapshot: &'a ProjectSnapshot,
    candidate: &crate::detection::SignalCandidate,
) -> Option<(&'a str, &'a str, &'a Value)> {
    snapshot.results.iter().find_map(|result| {
        if result["result_schema"].as_str() != Some("result.risk-signal-review")
            || result["role"].as_str() != Some("Analyst")
        {
            return None;
        }
        let input_refs = result["input_refs"].as_object()?;
        if !candidate.evidence_refs.iter().all(|reference| {
            input_refs.get(reference).and_then(Value::as_str)
                == snapshot.artifact_digests.get(reference).map(String::as_str)
        }) {
            return None;
        }
        array_field(&result["payload"], "reviewed_candidates")
            .iter()
            .find(|review| {
                review["fingerprint"].as_str() == Some(candidate.fingerprint.as_str())
                    && review["status"].as_str() == Some("not-applicable")
            })
            .and_then(|review| {
                Some((
                    result["id"].as_str()?,
                    review["reason"].as_str()?,
                    &review["basis_refs"],
                ))
            })
    })
}

fn array_field<'a>(value: &'a Value, field: &str) -> &'a [Value] {
    value[field].as_array().map(Vec::as_slice).unwrap_or(&[])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract_health::{ClauseHealth, ContractHealthSummary};
    use crate::detection::DetectionCoverage;
    use crate::kernel::NextAction;
    use crate::schema::SchemaRegistry;
    use serde_json::json;
    use std::path::PathBuf;

    fn requirement(subject: &str) -> RequirementInstance {
        RequirementInstance {
            requirement_id: "contract-reviewed".to_owned(),
            subject_refs: vec![subject.to_owned()],
            instance_key: format!("contract-reviewed|{subject}"),
            selected_by: vec!["test.rule".to_owned()],
            definition_digest: format!("sha256:{}", "a".repeat(64)),
            phase: "before-build".to_owned(),
            role: "Analyst".to_owned(),
            result_schema: "result.analysis".to_owned(),
            depends_on: Vec::new(),
            context: vec![
                "matching-contracts".to_owned(),
                "matching-decisions".to_owned(),
            ],
            assurance: Assurance::Attestation,
            status: "unsatisfied".to_owned(),
        }
    }

    fn detection() -> DetectionReport {
        DetectionReport {
            change_id: "change.test".to_owned(),
            coverage: DetectionCoverage {
                status: "complete".to_owned(),
                scope: "declared-artifacts".to_owned(),
                analyzed_refs: Vec::new(),
                gaps: Vec::new(),
            },
            candidates: Vec::new(),
            digest: String::new(),
        }
    }

    fn compile(snapshot: &ProjectSnapshot, instance: RequirementInstance) -> GeneratedContext {
        let decision = KernelDecision {
            state: "needs-analysis".to_owned(),
            action: Some(NextAction {
                id: "action.test".to_owned(),
                role: "Analyst".to_owned(),
                action: "analyze-requirements".to_owned(),
                requirement_instances: vec![instance.clone()],
                reason: "Contractを確認".to_owned(),
                expected_result_schema: "result.analysis".to_owned(),
                candidate_fingerprints: Vec::new(),
            }),
            requirement_instances: vec![instance],
            diagnostics: Vec::new(),
        };
        ContextCompiler
            .compile(&decision, snapshot, &detection())
            .unwrap()
    }

    #[test]
    fn projects_only_clauses_matching_the_requirement_subject() {
        let instance = requirement("integration.events");
        let event_clause_digest = format!("sha256:{}", "b".repeat(64));
        let snapshot = ProjectSnapshot {
            change_id: "change.test".to_owned(),
            change: json!({"id": "change.test"}),
            contracts: vec![json!({
                "schema_version": "1",
                "id": "contract.order",
                "applies_to": ["data.orders", "integration.events"],
                "clauses": [
                    {
                        "id": "no-duplicates",
                        "text": "注文を重複作成しない",
                        "applies_to": ["data.orders"],
                        "authority_ref": "decision.data"
                    },
                    {
                        "id": "retry-event",
                        "text": "event送信失敗時は再試行する",
                        "applies_to": ["integration.events"],
                        "authority_ref": "decision.event"
                    }
                ]
            })],
            decisions: vec![
                json!({"id": "decision.data"}),
                json!({"id": "decision.event"}),
            ],
            results: Vec::new(),
            evidence: Vec::new(),
            repository: json!({}),
            artifact_digests: BTreeMap::from([
                (
                    "contract.order#no-duplicates".to_owned(),
                    format!("sha256:{}", "c".repeat(64)),
                ),
                (
                    "contract.order#retry-event".to_owned(),
                    event_clause_digest.clone(),
                ),
                (
                    "decision.data".to_owned(),
                    format!("sha256:{}", "d".repeat(64)),
                ),
                (
                    "decision.event".to_owned(),
                    format!("sha256:{}", "e".repeat(64)),
                ),
            ]),
            digest: String::new(),
        };
        let schema_registry =
            SchemaRegistry::load(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("schemas/v1"))
                .unwrap();
        schema_registry
            .validate("contract", &snapshot.contracts[0])
            .unwrap();

        let context = compile(&snapshot, instance);
        assert_eq!(
            context.source_refs,
            ["contract.order#retry-event", "decision.event"]
        );
        assert_eq!(context.contract_clauses.len(), 1);
        let clause = &context.contract_clauses[0];
        assert_eq!(clause.clause_ref, "contract.order#retry-event");
        assert_eq!(clause.text, "event送信失敗時は再試行する");
        assert_eq!(clause.applies_to, ["integration.events"]);
        assert_eq!(clause.digest, event_clause_digest);
    }

    #[test]
    fn legacy_clause_scope_inherits_the_contract_scope() {
        let instance = requirement("integration.events");
        let snapshot = ProjectSnapshot {
            change_id: "change.test".to_owned(),
            change: json!({"id": "change.test"}),
            contracts: vec![json!({
                "id": "contract.order",
                "applies_to": ["integration.events"],
                "clauses": [
                    {"id": "retry", "text": "再試行する"},
                    {"id": "observe", "text": "失敗を観測する"}
                ]
            })],
            decisions: Vec::new(),
            results: Vec::new(),
            evidence: Vec::new(),
            repository: json!({}),
            artifact_digests: BTreeMap::from([(
                "contract.order".to_owned(),
                format!("sha256:{}", "f".repeat(64)),
            )]),
            digest: String::new(),
        };

        let context = compile(&snapshot, instance);
        assert_eq!(context.source_refs, ["contract.order"]);
        assert_eq!(context.contract_clauses.len(), 2);
        assert!(context.contract_clauses.iter().all(|clause| {
            clause.applies_to == ["integration.events"]
                && clause.selected_for == ["contract-reviewed|integration.events"]
        }));
    }

    #[test]
    fn revalidation_context_contains_only_the_selected_health_finding() {
        let instance = RequirementInstance {
            requirement_id: "contract-clause-revalidated".to_owned(),
            subject_refs: vec![
                "data.orders".to_owned(),
                "contract.order#no-duplicates".to_owned(),
            ],
            instance_key: "contract-clause-revalidated|contract.order#no-duplicates".to_owned(),
            selected_by: vec!["kernel.contract-health-v1".to_owned()],
            definition_digest: format!("sha256:{}", "a".repeat(64)),
            phase: "before-merge".to_owned(),
            role: "Builder".to_owned(),
            result_schema: "result.evidence".to_owned(),
            depends_on: Vec::new(),
            context: vec!["matching-contracts".to_owned()],
            assurance: Assurance::EvidenceBacked,
            status: "unsatisfied".to_owned(),
        };
        let snapshot = ProjectSnapshot {
            change_id: "change.test".to_owned(),
            change: json!({"id": "change.test"}),
            contracts: vec![json!({
                "id": "contract.order",
                "applies_to": ["data.orders"],
                "clauses": [
                    {
                        "id": "no-duplicates",
                        "text": "注文を重複作成しない",
                        "applies_to": ["data.orders"]
                    },
                    {
                        "id": "total-stable",
                        "text": "合計金額を維持する",
                        "applies_to": ["data.orders"]
                    }
                ]
            })],
            decisions: Vec::new(),
            results: Vec::new(),
            evidence: Vec::new(),
            repository: json!({}),
            artifact_digests: BTreeMap::from([(
                "contract.order#no-duplicates".to_owned(),
                format!("sha256:{}", "b".repeat(64)),
            )]),
            digest: String::new(),
        };
        let health = ContractHealthReport {
            schema_version: "1".to_owned(),
            repository_revision: Some("revision-2".to_owned()),
            summary: ContractHealthSummary {
                total: 2,
                verified: 0,
                stale: 1,
                unverified: 0,
                failed: 1,
            },
            clauses: vec![
                ClauseHealth {
                    contract_id: "contract.order".to_owned(),
                    clause_id: "no-duplicates".to_owned(),
                    clause_ref: "contract.order#no-duplicates".to_owned(),
                    text: "注文を重複作成しない".to_owned(),
                    applies_to: vec!["data.orders".to_owned()],
                    authority_ref: None,
                    status: "stale".to_owned(),
                    evidence_refs: vec!["evidence.old".to_owned()],
                    verification_result_ids: vec!["result.old".to_owned()],
                    stale_refs: vec!["code.orders".to_owned()],
                },
                ClauseHealth {
                    contract_id: "contract.order".to_owned(),
                    clause_id: "total-stable".to_owned(),
                    clause_ref: "contract.order#total-stable".to_owned(),
                    text: "合計金額を維持する".to_owned(),
                    applies_to: vec!["data.orders".to_owned()],
                    authority_ref: None,
                    status: "failed".to_owned(),
                    evidence_refs: vec!["evidence.failed".to_owned()],
                    verification_result_ids: vec!["result.failed".to_owned()],
                    stale_refs: Vec::new(),
                },
            ],
        };
        let decision = KernelDecision {
            state: "needs-evidence".to_owned(),
            action: Some(NextAction {
                id: "action.test".to_owned(),
                role: "Builder".to_owned(),
                action: "collect-evidence".to_owned(),
                requirement_instances: vec![instance.clone()],
                reason: "Contract条項を再検証".to_owned(),
                expected_result_schema: "result.evidence".to_owned(),
                candidate_fingerprints: Vec::new(),
            }),
            requirement_instances: vec![instance],
            diagnostics: Vec::new(),
        };

        let context = ContextCompiler
            .compile_with_health(&decision, &snapshot, &detection(), Some(&health))
            .unwrap();

        assert_eq!(context.contract_clauses.len(), 1);
        assert_eq!(
            context.contract_clauses[0].clause_ref,
            "contract.order#no-duplicates"
        );
        assert_eq!(
            context.payload["contract_health_findings"],
            json!([{
                "clause_ref": "contract.order#no-duplicates",
                "status": "stale",
                "stale_refs": ["code.orders"],
                "evidence_refs": ["evidence.old"],
                "selected_for": [
                    "contract-clause-revalidated|contract.order#no-duplicates"
                ]
            }])
        );
    }
}
