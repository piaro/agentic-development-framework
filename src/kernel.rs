//! Side-effect-free Thin Kernel state evaluation.
//!
//! The Kernel consumes an immutable Project Snapshot, compiled Rule Index, and
//! Detection Report. It performs no filesystem, network, Agent, or persistence
//! operations and therefore returns the same decision for the same inputs.

use crate::canonical_digest;
use crate::contract_health::{ClauseHealth, ContractHealthReport};
use crate::contract_scope::{clause_matches_subjects, contract_matches_subjects};
use crate::detection::{DetectionReport, SignalCandidate};
use crate::rules::{Assurance, RequirementDefinition, RuleIndex};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

pub const KERNEL_VERSION: &str = "6";
const SIGNAL_APPLICABILITY_REVIEW_REQUIREMENT: &str = "risk-signal-applicability-reviewed";
const CONTRACT_CLAUSE_REVALIDATION_REQUIREMENT: &str = "contract-clause-revalidated";
const IMPACT_GOVERNANCE_REQUIREMENT: &str = "impact-governance-established";

#[derive(Debug, Clone, PartialEq, Eq)]
struct CandidateDisposition {
    status: String,
    input_refs: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProjectSnapshot {
    pub change_id: String,
    pub change: Value,
    pub contracts: Vec<Value>,
    pub decisions: Vec<Value>,
    pub results: Vec<Value>,
    pub evidence: Vec<Value>,
    pub repository: Value,
    pub artifact_digests: BTreeMap<String, String>,
    pub digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequirementInstance {
    pub requirement_id: String,
    pub subject_refs: Vec<String>,
    pub instance_key: String,
    pub selected_by: Vec<String>,
    pub definition_digest: String,
    pub phase: String,
    pub role: String,
    pub result_schema: String,
    pub depends_on: Vec<String>,
    pub context: Vec<String>,
    #[serde(default, skip_serializing_if = "Assurance::is_attestation")]
    pub assurance: Assurance,
    pub status: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExecutionGuidance {
    pub preferred_model_tier: String,
    pub escalation_conditions: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NextAction {
    pub id: String,
    pub role: String,
    pub action: String,
    pub requirement_instances: Vec<RequirementInstance>,
    pub reason: String,
    pub expected_result_schema: String,
    pub candidate_fingerprints: Vec<String>,
    #[serde(skip)]
    pub execution_guidance: ExecutionGuidance,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct KernelDecision {
    pub state: String,
    pub action: Option<NextAction>,
    pub requirement_instances: Vec<RequirementInstance>,
    pub diagnostics: Vec<String>,
}

impl KernelDecision {
    /// Compact reviewed output plus a digest of the complete decision.
    pub fn compatibility_checkpoint(&self) -> Value {
        let body =
            serde_json::to_value(self).expect("Kernel Decision contains serializable fields");
        let decision_digest =
            canonical_digest(&body).expect("Kernel Decision contains no floating-point values");
        let instance_statuses: BTreeMap<&str, &str> = self
            .requirement_instances
            .iter()
            .map(|instance| (instance.instance_key.as_str(), instance.status.as_str()))
            .collect();
        let action = self.action.as_ref().map(|action| {
            json!({
                "id": action.id,
                "role": action.role,
                "action": action.action,
                "result_schema": action.expected_result_schema,
                "instance_keys": action
                    .requirement_instances
                    .iter()
                    .map(|instance| instance.instance_key.as_str())
                    .collect::<Vec<_>>(),
                "candidate_fingerprints": action.candidate_fingerprints,
            })
        });
        json!({
            "state": self.state,
            "action": action,
            "instance_statuses": instance_statuses,
            "diagnostics": self.diagnostics,
            "decision_digest": decision_digest,
        })
    }
}

pub struct ThinKernel;

impl ThinKernel {
    pub fn evaluate(
        &self,
        snapshot: &ProjectSnapshot,
        rule_index: &RuleIndex,
        detection: &DetectionReport,
    ) -> KernelDecision {
        self.evaluate_with_health(snapshot, rule_index, detection, None)
    }

    pub fn evaluate_with_health(
        &self,
        snapshot: &ProjectSnapshot,
        rule_index: &RuleIndex,
        detection: &DetectionReport,
        contract_health: Option<&ContractHealthReport>,
    ) -> KernelDecision {
        let assessment = fresh_impact_assessment(snapshot);
        let assessment_required = snapshot.change["impact_assessment"].as_str() == Some("required");
        if assessment_required && assessment.is_none() {
            let previous = current_impact_assessments(snapshot);
            let action = make_action(
                snapshot,
                "Analyst",
                "assess-change-impact",
                Vec::new(),
                if previous.is_empty() {
                    "Assess the intended effects before implementation.".to_owned()
                } else {
                    "Reassess the intended effects because the prior assessment is inconclusive or stale. Reuse its findings where they still apply.".to_owned()
                },
                "result.impact-assessment",
                &previous.join("|"),
                Vec::new(),
            );
            let state = if repository_phase(snapshot) == "post-build" {
                "needs-post-build-impact-assessment"
            } else {
                "needs-impact-assessment"
            };
            return decision(state, Some(action), Vec::new(), Vec::new());
        }
        if detection.coverage.status != "complete" {
            let diagnostics = detection
                .coverage
                .gaps
                .iter()
                .map(|gap| {
                    let reference = gap
                        .get("ref")
                        .map(|reference| format!(" ({reference})"))
                        .unwrap_or_default();
                    format!(
                        "detection coverage incomplete: {}{}: {}",
                        gap["kind"], reference, gap["reason"]
                    )
                })
                .collect();
            return decision("blocked-detection", None, Vec::new(), diagnostics);
        }
        let declared_candidates = assessment
            .map(|assessment| impact_candidates(assessment, snapshot))
            .unwrap_or_default();
        let dispositions = candidate_dispositions(snapshot);
        let build_results = fresh_build_results(snapshot);
        let mut confirmed: Vec<&SignalCandidate> = declared_candidates
            .iter()
            .chain(detection.candidates.iter().filter(|candidate| {
                dispositions
                    .get(&candidate.fingerprint)
                    .is_some_and(|disposition| disposition.status == "confirmed")
            }))
            .collect();
        let mut applicability_reviews = Vec::new();
        for candidate in detection.candidates.iter().filter(|candidate| {
            dispositions
                .get(&candidate.fingerprint)
                .is_some_and(|disposition| {
                    disposition.status == "not-applicable"
                        && disposition_evidence_is_current(candidate, disposition, snapshot)
                })
        }) {
            let mut instance = signal_applicability_review_instance(candidate);
            if let Some(status) = signal_applicability_review_outcome(&instance, snapshot) {
                // A completed independent review resolves this control. If the
                // Challenger does not support non-applicability, fail closed by
                // treating the signal as confirmed and selecting its Rules.
                instance.status = "satisfied".to_owned();
                if status != "satisfied" {
                    confirmed.push(candidate);
                }
            } else if repository_phase(snapshot) == "post-build"
                && build_baseline_satisfies(&instance, &build_results, snapshot)
            {
                instance.status = "satisfied".to_owned();
            }
            applicability_reviews.push(instance);
        }
        let unreviewed: Vec<&SignalCandidate> = detection
            .candidates
            .iter()
            .filter(|candidate| {
                !dispositions
                    .get(&candidate.fingerprint)
                    .is_some_and(|disposition| {
                        disposition.status == "confirmed"
                            || (disposition.status == "not-applicable"
                                && disposition_evidence_is_current(
                                    candidate,
                                    disposition,
                                    snapshot,
                                ))
                    })
            })
            .collect();
        let mut instances = match instantiate(snapshot, rule_index, &confirmed) {
            Ok(instances) => instances,
            Err(error) => {
                return decision("invalid", None, Vec::new(), vec![error]);
            }
        };
        if let Some(assessment) = assessment {
            instances.extend(impact_governance_instances(assessment));
        }
        instances.extend(applicability_reviews);
        if let Some(contract_health) = contract_health {
            instances.extend(contract_revalidation_instances(contract_health, &instances));
        }
        instances.sort_by(|left, right| left.instance_key.cmp(&right.instance_key));

        for instance in &mut instances {
            if instance.requirement_id == SIGNAL_APPLICABILITY_REVIEW_REQUIREMENT {
                continue;
            }
            instance.status = if (instance.requirement_id == "risk-signals-reviewed"
                && unreviewed.is_empty())
                || is_satisfied(instance, snapshot)
                || (repository_phase(snapshot) == "post-build"
                    && instance.phase == "before-build"
                    && build_baseline_satisfies(instance, &build_results, snapshot))
            {
                "satisfied".to_owned()
            } else {
                "unsatisfied".to_owned()
            };
        }

        if !unreviewed.is_empty() {
            let risk_instances = instances
                .iter()
                .filter(|instance| instance.requirement_id == "risk-signals-reviewed")
                .cloned()
                .collect();
            let fingerprints = unreviewed
                .iter()
                .map(|candidate| candidate.fingerprint.clone())
                .collect();
            let action = make_action(
                snapshot,
                "Analyst",
                "review-risk-signals",
                risk_instances,
                format!("{}件の新しいSignal候補を確認してください", unreviewed.len()),
                "result.risk-signal-review",
                "",
                fingerprints,
            );
            let state = if repository_phase(snapshot) == "post-build" {
                "needs-post-build-analysis"
            } else {
                "needs-analysis"
            };
            return decision(state, Some(action), instances, Vec::new());
        }

        // A Decision Request that no Decision has answered outranks the work
        // that raised it. The Analyst cannot satisfy a requirement whose rule is
        // the very thing being asked about, so leaving it for later would issue
        // that same Action again with the question still unasked.
        if let Some(request_id) = pending_decision_request(snapshot) {
            if !has_fresh_human_answer(snapshot, &request_id) {
                let action = make_action(
                    snapshot,
                    "Human",
                    "answer-decision-request",
                    Vec::new(),
                    format!("判断依頼 {request_id} への回答が必要です"),
                    "result.human-answer",
                    &request_id,
                    Vec::new(),
                );
                return decision("needs-human-decision", Some(action), instances, Vec::new());
            }
            if !decision_is_recorded(snapshot, &request_id) {
                let targets = instances
                    .iter()
                    .filter(|instance| {
                        instance.status == "unsatisfied"
                            && instance.role == "Analyst"
                            && instance.phase == "before-build"
                    })
                    .cloned()
                    .collect();
                let action = make_action(
                    snapshot,
                    "Analyst",
                    "record-human-decision",
                    targets,
                    format!("回答済みの判断 {request_id} をDecisionとContractへ反映してください"),
                    "result.analysis",
                    &request_id,
                    Vec::new(),
                );
                return decision(
                    "needs-decision-recording",
                    Some(action),
                    instances,
                    Vec::new(),
                );
            }
        }

        let pending_impact_governance: Vec<RequirementInstance> = instances
            .iter()
            .filter(|instance| {
                instance.requirement_id == IMPACT_GOVERNANCE_REQUIREMENT
                    && instance.status == "unsatisfied"
            })
            .cloned()
            .collect();
        if !pending_impact_governance.is_empty() {
            let action = make_action(
                snapshot,
                "Analyst",
                "establish-impact-governance",
                pending_impact_governance,
                "Establish only the Decisions and Contracts needed for the assessed impacts."
                    .to_owned(),
                "result.analysis",
                "",
                Vec::new(),
            );
            let state = if repository_phase(snapshot) == "post-build" {
                "needs-post-build-analysis"
            } else {
                "needs-analysis"
            };
            return decision(state, Some(action), instances, Vec::new());
        }

        let pending_applicability_reviews: Vec<RequirementInstance> = instances
            .iter()
            .filter(|instance| {
                instance.requirement_id == SIGNAL_APPLICABILITY_REVIEW_REQUIREMENT
                    && instance.status == "unsatisfied"
            })
            .cloned()
            .collect();
        if !pending_applicability_reviews.is_empty() {
            let action = batch_action(snapshot, &pending_applicability_reviews);
            return decision(
                "needs-pre-build-challenge",
                Some(action),
                instances,
                Vec::new(),
            );
        }

        let before_build: Vec<RequirementInstance> = instances
            .iter()
            .filter(|instance| {
                instance.phase == "before-build"
                    && instance.status == "unsatisfied"
                    && dependencies_satisfied(instance, &instances)
            })
            .cloned()
            .collect();
        if !before_build.is_empty() {
            let action = batch_action(snapshot, &before_build);
            let state = if repository_phase(snapshot) == "post-build" && action.role != "Challenger"
            {
                "needs-post-build-analysis"
            } else if action.role == "Challenger" {
                "needs-pre-build-challenge"
            } else {
                "needs-analysis"
            };
            return decision(state, Some(action), instances, Vec::new());
        }
        if instances
            .iter()
            .any(|instance| instance.phase == "before-build" && instance.status == "unsatisfied")
        {
            return decision(
                "invalid",
                None,
                instances,
                vec!["before-build Requirementの依存関係を解決できません".to_owned()],
            );
        }

        if repository_phase(snapshot) != "post-build" {
            let action = make_action(
                snapshot,
                "Builder",
                "implement-change",
                Vec::new(),
                "実装前に必要なRequirementを満たしました".to_owned(),
                "result.build",
                "",
                Vec::new(),
            );
            return decision("ready-to-build", Some(action), instances, Vec::new());
        }

        let before_merge: Vec<RequirementInstance> = instances
            .iter()
            .filter(|instance| {
                instance.phase == "before-merge"
                    && instance.status == "unsatisfied"
                    && dependencies_satisfied(instance, &instances)
            })
            .cloned()
            .collect();
        if !before_merge.is_empty() {
            let action = batch_action(snapshot, &before_merge);
            let state = match action.role.as_str() {
                "Builder" => "needs-evidence",
                "Challenger" => "needs-post-build-challenge",
                _ => "needs-post-build-analysis",
            };
            return decision(state, Some(action), instances, Vec::new());
        }
        if instances
            .iter()
            .any(|instance| instance.phase == "before-merge" && instance.status == "unsatisfied")
        {
            return decision(
                "invalid",
                None,
                instances,
                vec!["before-merge Requirementの依存関係を解決できません".to_owned()],
            );
        }
        decision("ready-to-merge", None, instances, Vec::new())
    }
}

fn current_impact_assessments(snapshot: &ProjectSnapshot) -> Vec<String> {
    snapshot
        .results
        .iter()
        .filter(|result| {
            string_field(result, "result_schema") == Some("result.impact-assessment")
                && string_field(result, "role") == Some("Analyst")
        })
        .filter_map(|result| string_field(result, "id").map(str::to_owned))
        .rev()
        .take(3)
        .collect()
}

pub(crate) fn fresh_impact_assessment(snapshot: &ProjectSnapshot) -> Option<&Value> {
    snapshot.results.iter().find(|result| {
        string_field(result, "result_schema") == Some("result.impact-assessment")
            && string_field(result, "role") == Some("Analyst")
            && matches!(
                nested_string(result, &["payload", "status"]),
                Some("impacts-identified" | "no-impact")
            )
            && result_is_fresh(result, snapshot)
    })
}

fn impact_candidates(assessment: &Value, snapshot: &ProjectSnapshot) -> Vec<SignalCandidate> {
    let assessment_id = string_field(assessment, "id").unwrap_or("result.impact-assessment");
    let assessment_digest = snapshot
        .artifact_digests
        .get(assessment_id)
        .cloned()
        .unwrap_or_else(|| "missing".to_owned());
    nested_array(assessment, &["payload", "impacts"])
        .iter()
        .filter_map(|impact| {
            let signal = string_field(impact, "signal")?.to_owned();
            let bindings = string_map(impact.get("bindings"));
            let fingerprint = canonical_digest(&json!({
                "source": "declared-impact-assessment-v1",
                "signal": signal,
                "bindings": bindings,
            }))
            .expect("validated impact candidates are canonical");
            Some(SignalCandidate {
                signal,
                bindings,
                evidence_refs: vec![assessment_id.to_owned()],
                detector_id: "declared-impact-assessment".to_owned(),
                detector_version: "1".to_owned(),
                fingerprint,
                evidence_digest: assessment_digest.clone(),
            })
        })
        .collect()
}

fn impact_governance_instances(assessment: &Value) -> Vec<RequirementInstance> {
    nested_array(assessment, &["payload", "impacts"])
        .iter()
        .filter(|impact| nested_array(impact, &["governing_refs"]).is_empty())
        .filter_map(|impact| {
            let signal = string_field(impact, "signal")?;
            let bindings = string_map(impact.get("bindings"));
            let mut subjects = bindings.values().cloned().collect::<Vec<_>>();
            subjects.sort();
            subjects.dedup();
            let fingerprint = canonical_digest(&json!({
                "signal": signal,
                "bindings": bindings,
            }))
            .expect("validated impacts are canonical");
            let definition = json!({
                "id": IMPACT_GOVERNANCE_REQUIREMENT,
                "phase": "before-build",
                "role": "Analyst",
                "result_schema": "result.analysis",
                "depends_on": [],
                "context": ["change", "impact-assessment", "matching-contracts", "matching-decisions"],
            });
            let definition_digest = canonical_digest(&definition)
                .expect("built-in impact governance definition is canonical");
            Some(RequirementInstance {
                requirement_id: IMPACT_GOVERNANCE_REQUIREMENT.to_owned(),
                subject_refs: subjects,
                instance_key: format!("{IMPACT_GOVERNANCE_REQUIREMENT}|{fingerprint}"),
                selected_by: vec!["kernel.impact-governance-v1".to_owned()],
                definition_digest,
                phase: "before-build".to_owned(),
                role: "Analyst".to_owned(),
                result_schema: "result.analysis".to_owned(),
                depends_on: Vec::new(),
                context: vec![
                    "change".to_owned(),
                    "impact-assessment".to_owned(),
                    "matching-contracts".to_owned(),
                    "matching-decisions".to_owned(),
                ],
                assurance: Assurance::Attestation,
                status: "unsatisfied".to_owned(),
            })
        })
        .collect()
}

fn decision(
    state: &str,
    action: Option<NextAction>,
    requirement_instances: Vec<RequirementInstance>,
    diagnostics: Vec<String>,
) -> KernelDecision {
    KernelDecision {
        state: state.to_owned(),
        action,
        requirement_instances,
        diagnostics,
    }
}

fn repository_phase(snapshot: &ProjectSnapshot) -> &str {
    snapshot.repository["phase"].as_str().unwrap_or("pre-build")
}

fn candidate_dispositions(snapshot: &ProjectSnapshot) -> BTreeMap<String, CandidateDisposition> {
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
                    CandidateDisposition {
                        status: status.to_owned(),
                        input_refs: string_map(result.get("input_refs")),
                    },
                );
            }
        }
    }
    dispositions
}

fn disposition_evidence_is_current(
    candidate: &SignalCandidate,
    disposition: &CandidateDisposition,
    snapshot: &ProjectSnapshot,
) -> bool {
    candidate.evidence_refs.iter().all(|reference| {
        snapshot.artifact_digests.get(reference) == disposition.input_refs.get(reference)
            && disposition.input_refs.contains_key(reference)
    })
}

fn signal_applicability_review_instance(candidate: &SignalCandidate) -> RequirementInstance {
    let definition = json!({
        "id": SIGNAL_APPLICABILITY_REVIEW_REQUIREMENT,
        "phase": "before-build",
        "role": "Challenger",
        "result_schema": "result.challenge",
        "depends_on": [],
        "context": ["change", "repository-artifacts", "results"],
    });
    let definition_digest = canonical_digest(&definition)
        .expect("built-in signal applicability review definition is canonical");
    RequirementInstance {
        requirement_id: SIGNAL_APPLICABILITY_REVIEW_REQUIREMENT.to_owned(),
        subject_refs: vec![candidate.fingerprint.clone()],
        instance_key: format!(
            "{SIGNAL_APPLICABILITY_REVIEW_REQUIREMENT}|{}|{}",
            candidate.fingerprint, candidate.evidence_digest
        ),
        selected_by: vec!["kernel.signal-applicability-review-v1".to_owned()],
        definition_digest,
        phase: "before-build".to_owned(),
        role: "Challenger".to_owned(),
        result_schema: "result.challenge".to_owned(),
        depends_on: Vec::new(),
        context: vec![
            "change".to_owned(),
            "repository-artifacts".to_owned(),
            "results".to_owned(),
        ],
        assurance: Assurance::Attestation,
        status: "unsatisfied".to_owned(),
    }
}

fn signal_applicability_review_outcome<'a>(
    instance: &RequirementInstance,
    snapshot: &'a ProjectSnapshot,
) -> Option<&'a str> {
    snapshot.results.iter().find_map(|result| {
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
                    && outcome_is_fresh(result, outcome, snapshot)
            })
            .and_then(|outcome| string_field(outcome, "status"))
    })
}

fn contract_revalidation_instances(
    contract_health: &ContractHealthReport,
    selected_instances: &[RequirementInstance],
) -> Vec<RequirementInstance> {
    let active_subjects: BTreeSet<&str> = selected_instances
        .iter()
        .flat_map(|instance| instance.subject_refs.iter().map(String::as_str))
        .collect();
    contract_health
        .clauses
        .iter()
        .filter(|clause| matches!(clause.status.as_str(), "stale" | "failed"))
        .filter(|clause| {
            clause
                .applies_to
                .iter()
                .any(|target| active_subjects.contains(target.as_str()))
        })
        .map(contract_revalidation_instance)
        .collect()
}

fn contract_revalidation_instance(clause: &ClauseHealth) -> RequirementInstance {
    let definition = json!({
        "id": CONTRACT_CLAUSE_REVALIDATION_REQUIREMENT,
        "phase": "before-merge",
        "role": "Builder",
        "result_schema": "result.evidence",
        "assurance": "evidence-backed",
        "depends_on": [],
        "context": ["change", "matching-contracts", "affected-code", "matching-evidence"],
    });
    let definition_digest = canonical_digest(&definition)
        .expect("built-in Contract clause revalidation definition is canonical");
    let mut subject_refs = clause.applies_to.clone();
    subject_refs.push(clause.clause_ref.clone());
    subject_refs.sort();
    subject_refs.dedup();
    RequirementInstance {
        requirement_id: CONTRACT_CLAUSE_REVALIDATION_REQUIREMENT.to_owned(),
        subject_refs,
        instance_key: format!(
            "{CONTRACT_CLAUSE_REVALIDATION_REQUIREMENT}|{}",
            clause.clause_ref
        ),
        selected_by: vec!["kernel.contract-health-v1".to_owned()],
        definition_digest,
        phase: "before-merge".to_owned(),
        role: "Builder".to_owned(),
        result_schema: "result.evidence".to_owned(),
        depends_on: Vec::new(),
        context: vec![
            "change".to_owned(),
            "matching-contracts".to_owned(),
            "affected-code".to_owned(),
            "matching-evidence".to_owned(),
        ],
        assurance: Assurance::EvidenceBacked,
        status: "unsatisfied".to_owned(),
    }
}

fn instantiate(
    snapshot: &ProjectSnapshot,
    rule_index: &RuleIndex,
    confirmed: &[&SignalCandidate],
) -> Result<Vec<RequirementInstance>, String> {
    let mut selected: BTreeMap<String, RequirementInstance> = BTreeMap::new();
    let candidates =
        std::iter::once(None).chain(confirmed.iter().map(|candidate| Some(*candidate)));
    for candidate in candidates {
        for rule in &rule_index.rules {
            if rule.condition == "always" && candidate.is_some() {
                continue;
            }
            if rule.condition == "signal"
                && candidate.is_none_or(|value| rule.signal.as_deref() != Some(&value.signal))
            {
                continue;
            }
            if rule
                .repository_phase
                .as_deref()
                .is_some_and(|phase| phase != repository_phase(snapshot))
            {
                continue;
            }
            let definition = &rule_index.requirements[&rule.requirement_id];
            let mut subjects = rule
                .subjects
                .iter()
                .map(|expression| resolve_subject(expression, snapshot, candidate))
                .collect::<Result<Vec<_>, _>>()?;
            subjects.sort();
            let instance_key = std::iter::once(definition.id.as_str())
                .chain(subjects.iter().map(String::as_str))
                .collect::<Vec<_>>()
                .join("|");
            let instance = new_instance(definition, subjects, instance_key.clone(), &rule.id);
            if let Some(existing) = selected.get_mut(&instance_key) {
                if existing.definition_digest != instance.definition_digest {
                    return Err(format!(
                        "Requirement Instanceの定義が競合しています: {instance_key}"
                    ));
                }
                existing.selected_by.push(rule.id.clone());
                existing.selected_by.sort();
                existing.selected_by.dedup();
            } else {
                selected.insert(instance_key, instance);
            }
        }
    }
    Ok(selected.into_values().collect())
}

fn new_instance(
    definition: &RequirementDefinition,
    subject_refs: Vec<String>,
    instance_key: String,
    rule_id: &str,
) -> RequirementInstance {
    RequirementInstance {
        requirement_id: definition.id.clone(),
        subject_refs,
        instance_key,
        selected_by: vec![rule_id.to_owned()],
        definition_digest: definition.definition_digest.clone(),
        phase: definition.phase.clone(),
        role: definition.role.clone(),
        result_schema: definition.result_schema.clone(),
        depends_on: definition.depends_on.clone(),
        context: definition.context.clone(),
        assurance: definition.assurance,
        status: "unsatisfied".to_owned(),
    }
}

fn resolve_subject(
    expression: &str,
    snapshot: &ProjectSnapshot,
    candidate: Option<&SignalCandidate>,
) -> Result<String, String> {
    if expression == "change.id" {
        return Ok(snapshot.change_id.clone());
    }
    if let Some(key) = expression.strip_prefix("binding.")
        && let Some(candidate) = candidate
    {
        return candidate
            .bindings
            .get(key)
            .cloned()
            .ok_or_else(|| format!("Signal bindingがありません: {key}"));
    }
    Err(format!("解決できないsubject式です: {expression}"))
}

fn is_satisfied(instance: &RequirementInstance, snapshot: &ProjectSnapshot) -> bool {
    snapshot.results.iter().any(|result| {
        nested_array(result, &["payload", "outcomes"])
            .iter()
            .any(|outcome| {
                string_field(result, "result_schema") == Some(&instance.result_schema)
                    && string_field(result, "role") == Some(&instance.role)
                    && string_field(outcome, "instance_key") == Some(&instance.instance_key)
                    && string_field(outcome, "definition_digest")
                        == Some(&instance.definition_digest)
                    && string_field(outcome, "status") == Some("satisfied")
                    && outcome_is_fresh(result, outcome, snapshot)
                    && match instance.assurance {
                        Assurance::Attestation => true,
                        Assurance::EvidenceBacked => {
                            outcome_has_current_evidence(instance, outcome, snapshot)
                        }
                    }
            })
    })
}

pub(crate) fn outcome_has_current_evidence(
    instance: &RequirementInstance,
    outcome: &Value,
    snapshot: &ProjectSnapshot,
) -> bool {
    let basis_refs = string_array_set(&outcome["basis_refs"]);
    let current_revision = string_field(&snapshot.repository, "revision");
    let required_clauses = matching_contract_clauses(instance, snapshot);
    let mut covered_clauses = BTreeSet::new();
    let mut found = false;

    for evidence in &snapshot.evidence {
        let Some(evidence_id) = string_field(evidence, "id") else {
            continue;
        };
        if !basis_refs.contains(evidence_id)
            || string_field(evidence, "change_id") != Some(snapshot.change_id.as_str())
            || current_revision.is_none()
            || string_field(evidence, "git_revision") != current_revision
            || string_field(evidence, "outcome") != Some("passed")
            || !array_contains(
                &evidence["requirement_instances"],
                instance.instance_key.as_str(),
            )
            || !has_reproducible_evidence_artifact(evidence)
        {
            continue;
        }
        let clause_refs = string_array_set(&evidence["contract_clause_refs"])
            .into_iter()
            .map(str::to_owned)
            .collect::<BTreeSet<_>>();
        covered_clauses.extend(clause_refs.intersection(&required_clauses).cloned());
        found = true;
    }

    found && required_clauses.is_subset(&covered_clauses)
}

fn matching_contract_clauses(
    instance: &RequirementInstance,
    snapshot: &ProjectSnapshot,
) -> BTreeSet<String> {
    let subjects = instance
        .subject_refs
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let explicit_clause_refs = subjects
        .iter()
        .filter(|subject| subject.contains('#'))
        .copied()
        .collect::<BTreeSet<_>>();
    let mut clause_refs = BTreeSet::new();
    for contract in &snapshot.contracts {
        if !contract_matches_subjects(contract, &subjects) {
            continue;
        }
        let contract_id = string_field(contract, "id").unwrap_or_default();
        for clause in nested_array(contract, &["clauses"]) {
            if let Some(clause_id) = string_field(clause, "id") {
                let clause_ref = format!("{contract_id}#{clause_id}");
                let matches = if explicit_clause_refs.is_empty() {
                    clause_matches_subjects(contract, clause, &subjects)
                } else {
                    explicit_clause_refs.contains(clause_ref.as_str())
                };
                if matches {
                    clause_refs.insert(clause_ref);
                }
            }
        }
    }
    clause_refs
}

fn has_reproducible_evidence_artifact(evidence: &Value) -> bool {
    let Some(method) = string_field(evidence, "method").filter(|value| !value.is_empty()) else {
        return false;
    };
    let Some(artifact) = evidence.get("artifact").and_then(Value::as_object) else {
        return false;
    };
    let uri_present = artifact
        .get("uri")
        .and_then(Value::as_str)
        .is_some_and(|value| !value.is_empty());
    let digest_valid = artifact
        .get("digest")
        .and_then(Value::as_str)
        .is_some_and(is_sha256_digest);
    let successful_exit = artifact.get("exit_code").and_then(Value::as_i64) == Some(0);
    !method.is_empty() && uri_present && digest_valid && successful_exit
}

fn is_sha256_digest(value: &str) -> bool {
    value
        .strip_prefix("sha256:")
        .is_some_and(|hex| hex.len() == 64 && hex.bytes().all(|byte| byte.is_ascii_hexdigit()))
}

fn array_contains(value: &Value, expected: &str) -> bool {
    value
        .as_array()
        .is_some_and(|items| items.iter().any(|item| item.as_str() == Some(expected)))
}

fn string_array_set(value: &Value) -> BTreeSet<&str> {
    value
        .as_array()
        .map(|items| items.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default()
}

fn outcome_is_fresh(result: &Value, outcome: &Value, snapshot: &ProjectSnapshot) -> bool {
    let empty = json!({});
    let refs = outcome
        .get("freshness_refs")
        .or_else(|| result.get("freshness_refs"))
        .or_else(|| result.get("input_refs"))
        .unwrap_or(&empty);
    refs_are_fresh(refs, snapshot)
}

fn result_is_fresh(result: &Value, snapshot: &ProjectSnapshot) -> bool {
    let empty = json!({});
    let refs = result
        .get("freshness_refs")
        .or_else(|| result.get("input_refs"))
        .unwrap_or(&empty);
    refs_are_fresh(refs, snapshot)
}

fn fresh_build_results(snapshot: &ProjectSnapshot) -> Vec<&Value> {
    if repository_phase(snapshot) != "post-build" {
        return Vec::new();
    }
    snapshot
        .results
        .iter()
        .filter(|result| {
            string_field(result, "result_schema") == Some("result.build")
                && string_field(result, "role") == Some("Builder")
                && result_is_fresh(result, snapshot)
        })
        .collect()
}

fn build_baseline_satisfies(
    instance: &RequirementInstance,
    build_results: &[&Value],
    snapshot: &ProjectSnapshot,
) -> bool {
    build_results.iter().any(|result| {
        let build_inputs = string_map(result.get("input_refs"));
        snapshot.results.iter().any(|baseline_result| {
            let Some(result_id) = string_field(baseline_result, "id") else {
                return false;
            };
            snapshot.artifact_digests.get(result_id) == build_inputs.get(result_id)
                && nested_array(baseline_result, &["payload", "outcomes"])
                    .iter()
                    .any(|outcome| {
                        string_field(baseline_result, "result_schema")
                            == Some(instance.result_schema.as_str())
                            && string_field(baseline_result, "role") == Some(instance.role.as_str())
                            && string_field(outcome, "instance_key")
                                == Some(instance.instance_key.as_str())
                            && string_field(outcome, "definition_digest")
                                == Some(instance.definition_digest.as_str())
                            && string_field(outcome, "status") == Some("satisfied")
                    })
        })
    })
}

fn refs_are_fresh(refs: &Value, snapshot: &ProjectSnapshot) -> bool {
    let Some(refs) = refs.as_object() else {
        return false;
    };
    refs.iter().all(|(reference, digest)| {
        digest.as_str().is_some_and(|digest| {
            snapshot.artifact_digests.get(reference) == Some(&digest.to_owned())
        })
    })
}

fn dependencies_satisfied(
    instance: &RequirementInstance,
    instances: &[RequirementInstance],
) -> bool {
    instance.depends_on.iter().all(|dependency| {
        let matching = instances
            .iter()
            .filter(|candidate| candidate.requirement_id == *dependency)
            .collect::<Vec<_>>();
        // A dependency that no Rule selected is not applicable to this Change.
        // Every selected instance of an applicable dependency must be satisfied.
        matching
            .iter()
            .all(|candidate| candidate.status == "satisfied")
    })
}

fn pending_decision_request(snapshot: &ProjectSnapshot) -> Option<String> {
    for result in &snapshot.results {
        if !result_is_fresh(result, snapshot) {
            continue;
        }
        for request in nested_array(result, &["payload", "decision_requests"]) {
            if let Some(request_id) = string_field(request, "id")
                && !decision_is_recorded(snapshot, request_id)
            {
                return Some(request_id.to_owned());
            }
        }
    }
    None
}

fn has_fresh_human_answer(snapshot: &ProjectSnapshot, request_id: &str) -> bool {
    snapshot.results.iter().any(|result| {
        string_field(result, "result_schema") == Some("result.human-answer")
            && string_field(result, "role") == Some("Human")
            && result_is_fresh(result, snapshot)
            && nested_array(result, &["payload", "answers"])
                .iter()
                .any(|answer| string_field(answer, "request_id") == Some(request_id))
    })
}

fn decision_is_recorded(snapshot: &ProjectSnapshot, request_id: &str) -> bool {
    let decision_ids: BTreeSet<&str> = snapshot
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
                            .any(|value| value.as_str() == Some(request_id))
                    })
        })
        .filter_map(|decision| string_field(decision, "id"))
        .collect();
    !decision_ids.is_empty()
        && snapshot.contracts.iter().any(|contract| {
            contract
                .get("clauses")
                .and_then(Value::as_array)
                .is_some_and(|clauses| {
                    clauses.iter().any(|clause| {
                        string_field(clause, "authority_ref")
                            .is_some_and(|reference| decision_ids.contains(reference))
                    })
                })
        })
}

fn batch_action(snapshot: &ProjectSnapshot, candidates: &[RequirementInstance]) -> NextAction {
    let mut ordered: Vec<&RequirementInstance> = candidates.iter().collect();
    ordered.sort_by(|left, right| {
        (
            role_rank(&left.role),
            &left.result_schema,
            &left.instance_key,
        )
            .cmp(&(
                role_rank(&right.role),
                &right.result_schema,
                &right.instance_key,
            ))
    });
    let first = ordered[0];
    let batch: Vec<RequirementInstance> = candidates
        .iter()
        .filter(|item| item.role == first.role && item.result_schema == first.result_schema)
        .cloned()
        .collect();
    let action_name = match first.role.as_str() {
        "Analyst" => "analyze-requirements",
        "Builder" => "record-evidence",
        "Challenger" => "challenge-result",
        _ => "complete-requirements",
    };
    make_action(
        snapshot,
        &first.role,
        action_name,
        batch.clone(),
        format!("{}件のRequirementを満たす必要があります", batch.len()),
        &first.result_schema,
        "",
        Vec::new(),
    )
}

fn role_rank(role: &str) -> u8 {
    match role {
        "Analyst" => 0,
        "Builder" => 1,
        "Challenger" => 2,
        _ => 9,
    }
}

#[allow(clippy::too_many_arguments)]
fn make_action(
    snapshot: &ProjectSnapshot,
    role: &str,
    action: &str,
    instances: Vec<RequirementInstance>,
    reason: String,
    expected_result_schema: &str,
    extra: &str,
    candidate_fingerprints: Vec<String>,
) -> NextAction {
    let action_body = json!({
        "change_id": snapshot.change_id,
        "role": role,
        "action": action,
        "instances": instances
            .iter()
            .map(|instance| {
                json!([instance.instance_key, instance.definition_digest])
            })
            .collect::<Vec<_>>(),
        "extra": extra,
        "candidate_fingerprints": candidate_fingerprints,
    });
    let digest =
        canonical_digest(&action_body).expect("Action identity contains no floating-point values");
    let suffix = digest
        .strip_prefix("sha256:")
        .expect("canonical digest has the sha256 prefix");
    let id = format!("action.{}", &suffix[..16]);
    NextAction {
        id,
        role: role.to_owned(),
        action: action.to_owned(),
        requirement_instances: instances,
        reason,
        expected_result_schema: expected_result_schema.to_owned(),
        candidate_fingerprints,
        execution_guidance: execution_guidance(action),
    }
}

fn execution_guidance(action: &str) -> ExecutionGuidance {
    match action {
        "assess-change-impact" => ExecutionGuidance {
            preferred_model_tier: "economy".to_owned(),
            escalation_conditions: vec![
                "The assessment would conclude no-impact.".to_owned(),
                "The available authority is insufficient or contradictory.".to_owned(),
                "A security, privacy, payment, or irreversible-data risk is plausible.".to_owned(),
            ],
        },
        "challenge-result" => ExecutionGuidance {
            preferred_model_tier: "high-accuracy".to_owned(),
            escalation_conditions: Vec::new(),
        },
        _ => ExecutionGuidance {
            preferred_model_tier: "standard".to_owned(),
            escalation_conditions: vec![
                "The action cannot be completed from its issued Context.".to_owned(),
            ],
        },
    }
}

fn string_field<'a>(value: &'a Value, field: &str) -> Option<&'a str> {
    value.get(field).and_then(Value::as_str)
}

fn nested_string<'a>(value: &'a Value, path: &[&str]) -> Option<&'a str> {
    let mut current = value;
    for field in path {
        current = current.get(*field)?;
    }
    current.as_str()
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

fn nested_array<'a>(value: &'a Value, path: &[&str]) -> &'a [Value] {
    let mut current = value;
    for field in path {
        let Some(next) = current.get(field) else {
            return &[];
        };
        current = next;
    }
    current.as_array().map(Vec::as_slice).unwrap_or(&[])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::ContextCompiler;
    use crate::contract_health::ContractHealthSummary;
    use crate::detection::{DetectionCoverage, DetectionReport};
    use crate::explain::ExplanationBuilder;
    use crate::rules::{ActivationRule, RequirementDefinition, RuleIndex};
    use crate::schema::SchemaRegistry;
    use crate::submission::{ResultSubmission, prepare_result};
    use std::path::PathBuf;

    fn evidence_instance(assurance: Assurance) -> RequirementInstance {
        RequirementInstance {
            requirement_id: "tests-passed".to_owned(),
            subject_refs: vec!["operation.test".to_owned()],
            instance_key: "tests-passed|operation.test".to_owned(),
            selected_by: vec!["tests-passed-rule".to_owned()],
            definition_digest: format!("sha256:{}", "a".repeat(64)),
            phase: "before-merge".to_owned(),
            role: "Builder".to_owned(),
            result_schema: "result.evidence".to_owned(),
            depends_on: Vec::new(),
            context: vec!["matching-contracts".to_owned()],
            assurance,
            status: "unsatisfied".to_owned(),
        }
    }

    fn dependency_instance(requirement_id: &str, status: &str) -> RequirementInstance {
        RequirementInstance {
            requirement_id: requirement_id.to_owned(),
            subject_refs: vec!["operation.test".to_owned()],
            instance_key: format!("{requirement_id}|operation.test"),
            selected_by: vec!["test.rule".to_owned()],
            definition_digest: format!("sha256:{}", "a".repeat(64)),
            phase: "before-build".to_owned(),
            role: "Analyst".to_owned(),
            result_schema: "result.analysis".to_owned(),
            depends_on: Vec::new(),
            context: Vec::new(),
            assurance: Assurance::Attestation,
            status: status.to_owned(),
        }
    }

    #[test]
    fn an_unselected_dependency_is_not_applicable() {
        let mut target = dependency_instance("design-challenged", "unsatisfied");
        target.depends_on = vec![
            "data-contracts-ready".to_owned(),
            "distributed-effect-contracts-ready".to_owned(),
        ];
        let data_contract = dependency_instance("data-contracts-ready", "satisfied");

        assert!(dependencies_satisfied(
            &target,
            &[target.clone(), data_contract]
        ));
    }

    #[test]
    fn a_selected_unsatisfied_dependency_blocks_downstream_work() {
        let mut target = dependency_instance("design-challenged", "unsatisfied");
        target.depends_on = vec![
            "data-contracts-ready".to_owned(),
            "distributed-effect-contracts-ready".to_owned(),
        ];
        let data_contract = dependency_instance("data-contracts-ready", "satisfied");
        let distributed_contract =
            dependency_instance("distributed-effect-contracts-ready", "unsatisfied");

        assert!(!dependencies_satisfied(
            &target,
            &[target.clone(), data_contract, distributed_contract]
        ));
    }

    #[test]
    fn a_single_signal_workflow_reaches_its_challenge_without_unselected_dependencies() {
        let data_definition = RequirementDefinition {
            id: "data-contracts-ready".to_owned(),
            phase: "before-build".to_owned(),
            role: "Analyst".to_owned(),
            result_schema: "result.analysis".to_owned(),
            depends_on: Vec::new(),
            context: Vec::new(),
            assurance: Assurance::Attestation,
            definition_digest: format!("sha256:{}", "c".repeat(64)),
        };
        let design_definition = RequirementDefinition {
            id: "design-challenged".to_owned(),
            phase: "before-build".to_owned(),
            role: "Challenger".to_owned(),
            result_schema: "result.challenge".to_owned(),
            depends_on: vec![
                "data-contracts-ready".to_owned(),
                "distributed-effect-contracts-ready".to_owned(),
            ],
            context: Vec::new(),
            assurance: Assurance::Attestation,
            definition_digest: format!("sha256:{}", "d".repeat(64)),
        };
        let distributed_definition = RequirementDefinition {
            id: "distributed-effect-contracts-ready".to_owned(),
            phase: "before-build".to_owned(),
            role: "Analyst".to_owned(),
            result_schema: "result.analysis".to_owned(),
            depends_on: Vec::new(),
            context: Vec::new(),
            assurance: Assurance::Attestation,
            definition_digest: format!("sha256:{}", "e".repeat(64)),
        };
        let rule_index = RuleIndex {
            requirements: BTreeMap::from([
                (data_definition.id.clone(), data_definition.clone()),
                (design_definition.id.clone(), design_definition),
                (distributed_definition.id.clone(), distributed_definition),
            ]),
            rules: vec![
                ActivationRule {
                    id: "persistent-data.prepare-contracts".to_owned(),
                    requirement_id: data_definition.id.clone(),
                    condition: "signal".to_owned(),
                    signal: Some("persistent-data-write".to_owned()),
                    repository_phase: None,
                    subjects: vec!["binding.data".to_owned()],
                },
                ActivationRule {
                    id: "persistent-data.challenge-design".to_owned(),
                    requirement_id: "design-challenged".to_owned(),
                    condition: "signal".to_owned(),
                    signal: Some("persistent-data-write".to_owned()),
                    repository_phase: None,
                    subjects: vec!["binding.operation".to_owned()],
                },
            ],
            digest: String::new(),
        };
        let fingerprint = format!("sha256:{}", "f".repeat(64));
        let detection = DetectionReport {
            change_id: "change.test".to_owned(),
            coverage: DetectionCoverage {
                status: "complete".to_owned(),
                scope: "declared-artifacts".to_owned(),
                analyzed_refs: vec!["code.accounts".to_owned()],
                gaps: Vec::new(),
            },
            candidates: vec![SignalCandidate {
                signal: "persistent-data-write".to_owned(),
                bindings: BTreeMap::from([
                    ("data".to_owned(), "data.accounts".to_owned()),
                    (
                        "operation".to_owned(),
                        "operation.register-account".to_owned(),
                    ),
                ]),
                evidence_refs: vec!["code.accounts".to_owned()],
                detector_id: "typed-repository-fact".to_owned(),
                detector_version: "2".to_owned(),
                fingerprint: fingerprint.clone(),
                evidence_digest: format!("sha256:{}", "1".repeat(64)),
            }],
            digest: String::new(),
        };
        let snapshot = ProjectSnapshot {
            change_id: "change.test".to_owned(),
            change: json!({"id": "change.test"}),
            contracts: Vec::new(),
            decisions: Vec::new(),
            results: vec![
                json!({
                    "result_schema": "result.risk-signal-review",
                    "role": "Analyst",
                    "input_refs": {},
                    "payload": {
                        "reviewed_candidates": [{
                            "fingerprint": fingerprint,
                            "status": "confirmed"
                        }]
                    }
                }),
                json!({
                    "result_schema": "result.analysis",
                    "role": "Analyst",
                    "payload": {
                        "outcomes": [{
                            "instance_key": "data-contracts-ready|data.accounts",
                            "definition_digest": data_definition.definition_digest,
                            "status": "satisfied",
                            "freshness_refs": {}
                        }]
                    }
                }),
            ],
            evidence: Vec::new(),
            repository: json!({"phase": "pre-build"}),
            artifact_digests: BTreeMap::new(),
            digest: String::new(),
        };

        let decision = ThinKernel.evaluate(&snapshot, &rule_index, &detection);

        assert_eq!(decision.state, "needs-pre-build-challenge");
        let action = decision.action.unwrap();
        assert_eq!(action.role, "Challenger");
        assert_eq!(action.requirement_instances.len(), 1);
        assert_eq!(
            action.requirement_instances[0].requirement_id,
            "design-challenged"
        );
    }

    fn evidence_snapshot() -> ProjectSnapshot {
        let instance = evidence_instance(Assurance::EvidenceBacked);
        ProjectSnapshot {
            change_id: "change.test".to_owned(),
            change: json!({}),
            contracts: vec![json!({
                "id": "contract.test",
                "applies_to": ["operation.test", "operation.other"],
                "clauses": [
                    {
                        "id": "stable-output",
                        "applies_to": ["operation.test"]
                    },
                    {
                        "id": "unrelated-output",
                        "applies_to": ["operation.other"]
                    }
                ]
            })],
            decisions: Vec::new(),
            results: vec![json!({
                "result_schema": "result.evidence",
                "role": "Builder",
                "payload": {
                    "outcomes": [{
                        "instance_key": instance.instance_key,
                        "definition_digest": instance.definition_digest,
                        "status": "satisfied",
                        "basis_refs": ["evidence.test"],
                        "freshness_refs": {}
                    }]
                }
            })],
            evidence: vec![json!({
                "id": "evidence.test",
                "change_id": "change.test",
                "requirement_instances": ["tests-passed|operation.test"],
                "contract_clause_refs": ["contract.test#stable-output"],
                "git_revision": "revision-2",
                "method": "cargo test --locked",
                "outcome": "passed",
                "artifact": {
                    "uri": "artifact://ci/test-report.json",
                    "digest": format!("sha256:{}", "b".repeat(64)),
                    "exit_code": 0
                }
            })],
            repository: json!({"revision": "revision-2"}),
            artifact_digests: BTreeMap::new(),
            digest: String::new(),
        }
    }

    fn not_applicable_signal_case() -> (ProjectSnapshot, RuleIndex, DetectionReport) {
        let fingerprint = format!("sha256:{}", "f".repeat(64));
        let candidate = SignalCandidate {
            signal: "persistent-data-write".to_owned(),
            bindings: BTreeMap::from([
                ("data".to_owned(), "data.orders".to_owned()),
                ("operation".to_owned(), "operation.place-order".to_owned()),
            ]),
            evidence_refs: vec!["code.place-order".to_owned()],
            detector_id: "typed-repository-fact".to_owned(),
            detector_version: "2".to_owned(),
            fingerprint: fingerprint.clone(),
            evidence_digest: format!("sha256:{}", "e".repeat(64)),
        };
        let snapshot = ProjectSnapshot {
            change_id: "change.test".to_owned(),
            change: json!({"id": "change.test"}),
            contracts: Vec::new(),
            decisions: Vec::new(),
            results: vec![json!({
                "id": "result.signal-review",
                "result_schema": "result.risk-signal-review",
                "role": "Analyst",
                "input_refs": {
                    "code.place-order": format!("sha256:{}", "c".repeat(64))
                },
                "payload": {
                    "reviewed_candidates": [{
                        "fingerprint": fingerprint,
                        "status": "not-applicable",
                        "reason": "write APIではなくread-only probe",
                        "basis_refs": ["code.place-order"]
                    }]
                }
            })],
            evidence: Vec::new(),
            repository: json!({
                "phase": "pre-build",
                "revision": "revision-1",
                "artifacts": [{
                    "ref": "code.place-order",
                    "digest": format!("sha256:{}", "c".repeat(64))
                }]
            }),
            artifact_digests: BTreeMap::from([
                (
                    "change.test".to_owned(),
                    format!("sha256:{}", "a".repeat(64)),
                ),
                (
                    "code.place-order".to_owned(),
                    format!("sha256:{}", "c".repeat(64)),
                ),
                (
                    "result.signal-review".to_owned(),
                    format!("sha256:{}", "d".repeat(64)),
                ),
            ]),
            digest: String::new(),
        };
        let definition = RequirementDefinition {
            id: "affected-data-confirmed".to_owned(),
            phase: "before-build".to_owned(),
            role: "Analyst".to_owned(),
            result_schema: "result.analysis".to_owned(),
            depends_on: Vec::new(),
            context: vec!["change".to_owned()],
            assurance: Assurance::Attestation,
            definition_digest: format!("sha256:{}", "e".repeat(64)),
        };
        let rule_index = RuleIndex {
            requirements: BTreeMap::from([(definition.id.clone(), definition)]),
            rules: vec![ActivationRule {
                id: "persistent-data.analyze".to_owned(),
                requirement_id: "affected-data-confirmed".to_owned(),
                condition: "signal".to_owned(),
                signal: Some("persistent-data-write".to_owned()),
                repository_phase: Some("pre-build".to_owned()),
                subjects: vec!["binding.data".to_owned()],
            }],
            digest: String::new(),
        };
        let detection = DetectionReport {
            change_id: "change.test".to_owned(),
            coverage: DetectionCoverage {
                status: "complete".to_owned(),
                scope: "declared-artifacts".to_owned(),
                analyzed_refs: vec!["code.place-order".to_owned()],
                gaps: Vec::new(),
            },
            candidates: vec![candidate],
            digest: String::new(),
        };
        (snapshot, rule_index, detection)
    }

    fn add_applicability_challenge_result(
        snapshot: &mut ProjectSnapshot,
        decision: &KernelDecision,
        detection: &DetectionReport,
        status: &str,
    ) {
        let context = ContextCompiler
            .compile(decision, snapshot, detection)
            .unwrap();
        let instance = decision
            .action
            .as_ref()
            .unwrap()
            .requirement_instances
            .first()
            .unwrap();
        let schema_registry =
            SchemaRegistry::load(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("schemas/v1"))
                .unwrap();
        let submission = ResultSubmission {
            change_id: snapshot.change_id.clone(),
            action_id: context.action_id.clone(),
            context_digest: context.digest.clone(),
            role: "Challenger".to_owned(),
            result_schema: "result.challenge".to_owned(),
            payload: json!({
                "outcomes": [{
                    "instance_key": instance.instance_key,
                    "definition_digest": instance.definition_digest,
                    "status": status,
                    "summary": "signal候補の非該当判定を独立確認",
                    "basis_refs": ["result.signal-review"]
                }]
            }),
            output_refs: Vec::new(),
            execution: None,
        };
        let result = prepare_result(&context, snapshot, &submission, &schema_registry).unwrap();
        snapshot.results.push(result);
    }

    #[test]
    fn considers_empty_freshness_manifest_fresh() {
        let snapshot = ProjectSnapshot {
            change_id: "change.test".to_owned(),
            change: json!({}),
            contracts: Vec::new(),
            decisions: Vec::new(),
            results: Vec::new(),
            evidence: Vec::new(),
            repository: json!({"phase": "pre-build", "artifacts": [], "facts": []}),
            artifact_digests: BTreeMap::new(),
            digest: String::new(),
        };
        assert!(result_is_fresh(&json!({}), &snapshot));
        assert!(outcome_is_fresh(&json!({}), &json!({}), &snapshot));
    }

    fn impact_snapshot(status: &str, impacts: Value, unknowns: Value) -> ProjectSnapshot {
        let change_digest = format!("sha256:{}", "a".repeat(64));
        let result_digest = format!("sha256:{}", "b".repeat(64));
        ProjectSnapshot {
            change_id: "change.test".to_owned(),
            change: json!({
                "id": "change.test",
                "impact_assessment": "required"
            }),
            contracts: Vec::new(),
            decisions: Vec::new(),
            results: if status.is_empty() {
                Vec::new()
            } else {
                vec![json!({
                    "id": "result.impact",
                    "result_schema": "result.impact-assessment",
                    "role": "Analyst",
                    "input_refs": {"change.test": change_digest},
                    "payload": {
                        "status": status,
                        "impacts": impacts,
                        "unknowns": unknowns
                    }
                })]
            },
            evidence: Vec::new(),
            repository: json!({"phase": "pre-build"}),
            artifact_digests: BTreeMap::from([
                ("change.test".to_owned(), change_digest),
                ("result.impact".to_owned(), result_digest),
            ]),
            digest: String::new(),
        }
    }

    fn empty_rule_index() -> RuleIndex {
        RuleIndex {
            requirements: BTreeMap::new(),
            rules: Vec::new(),
            digest: String::new(),
        }
    }

    fn complete_empty_detection() -> DetectionReport {
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

    #[test]
    fn a_new_empty_project_requires_impact_assessment_before_building() {
        let snapshot = impact_snapshot("", json!([]), json!([]));

        let decision =
            ThinKernel.evaluate(&snapshot, &empty_rule_index(), &complete_empty_detection());

        assert_eq!(decision.state, "needs-impact-assessment");
        let action = decision.action.unwrap();
        assert_eq!(action.action, "assess-change-impact");
        assert_eq!(action.expected_result_schema, "result.impact-assessment");
        assert_eq!(action.execution_guidance.preferred_model_tier, "economy");
    }

    #[test]
    fn an_explicit_no_impact_assessment_can_reach_build() {
        let snapshot = impact_snapshot("no-impact", json!([]), json!([]));

        let decision =
            ThinKernel.evaluate(&snapshot, &empty_rule_index(), &complete_empty_detection());

        assert_eq!(decision.state, "ready-to-build");
        assert_eq!(decision.action.unwrap().action, "implement-change");
    }

    #[test]
    fn changed_assessment_inputs_require_reassessment() {
        let mut snapshot = impact_snapshot("no-impact", json!([]), json!([]));
        snapshot.results[0]["input_refs"]["repository.content"] =
            json!(format!("sha256:{}", "c".repeat(64)));
        snapshot.artifact_digests.insert(
            "repository.content".to_owned(),
            format!("sha256:{}", "d".repeat(64)),
        );

        let decision =
            ThinKernel.evaluate(&snapshot, &empty_rule_index(), &complete_empty_detection());

        assert_eq!(decision.state, "needs-impact-assessment");
        assert_eq!(decision.action.unwrap().action, "assess-change-impact");
    }

    #[test]
    fn a_declared_greenfield_effect_requires_minimal_governance() {
        let snapshot = impact_snapshot(
            "impacts-identified",
            json!([{
                "signal": "persistent-data-write",
                "bindings": {
                    "data": "data.accounts",
                    "operation": "operation.register-account"
                },
                "governing_refs": []
            }]),
            json!([]),
        );

        let decision =
            ThinKernel.evaluate(&snapshot, &empty_rule_index(), &complete_empty_detection());

        assert_eq!(decision.state, "needs-analysis");
        let action = decision.action.unwrap();
        assert_eq!(action.action, "establish-impact-governance");
        assert_eq!(action.requirement_instances.len(), 1);
    }

    #[test]
    fn an_unanswered_request_outranks_the_governance_that_raised_it() {
        // The Analyst reached establish-impact-governance, found that no
        // authority settled the rule, and raised a Decision Request instead of
        // inferring one. Reissuing that Action would ask the same unanswerable
        // question forever, because only a Contract can satisfy the requirement
        // and only a person can authorize the Contract.
        let mut snapshot = impact_snapshot(
            "impacts-identified",
            json!([{
                "signal": "persistent-data-write",
                "bindings": {
                    "data": "data.accounts",
                    "operation": "operation.register-account"
                },
                "governing_refs": []
            }]),
            json!([]),
        );
        snapshot.results.push(json!({
            "id": "result.request",
            "result_schema": "result.analysis",
            "role": "Analyst",
            "input_refs": {"change.test": snapshot.artifact_digests["change.test"]},
            "payload": {
                "outcomes": [],
                "decision_requests": [{
                    "id": "decision-request.account-deletion",
                    "question": "Does deleting an account delete its orders?",
                    "known_fact_refs": ["change.test"]
                }]
            }
        }));
        snapshot.artifact_digests.insert(
            "result.request".to_owned(),
            format!("sha256:{}", "e".repeat(64)),
        );

        let decision =
            ThinKernel.evaluate(&snapshot, &empty_rule_index(), &complete_empty_detection());

        assert_eq!(decision.state, "needs-human-decision");
        let action = decision.action.unwrap();
        assert_eq!(action.action, "answer-decision-request");
        assert_eq!(action.role, "Human");
    }

    #[test]
    fn a_declared_effect_selects_signal_rules_without_detected_code() {
        let mut snapshot = impact_snapshot(
            "impacts-identified",
            json!([{
                "signal": "persistent-data-write",
                "bindings": {
                    "data": "data.accounts",
                    "operation": "operation.register-account"
                },
                "governing_refs": ["contract.accounts"]
            }]),
            json!([]),
        );
        snapshot.contracts.push(json!({"id": "contract.accounts"}));
        let definition = RequirementDefinition {
            id: "account-data-confirmed".to_owned(),
            phase: "before-build".to_owned(),
            role: "Analyst".to_owned(),
            result_schema: "result.analysis".to_owned(),
            depends_on: Vec::new(),
            context: vec!["change".to_owned()],
            assurance: Assurance::Attestation,
            definition_digest: format!("sha256:{}", "c".repeat(64)),
        };
        let rules = RuleIndex {
            requirements: BTreeMap::from([(definition.id.clone(), definition)]),
            rules: vec![ActivationRule {
                id: "persistent-data.account".to_owned(),
                requirement_id: "account-data-confirmed".to_owned(),
                condition: "signal".to_owned(),
                signal: Some("persistent-data-write".to_owned()),
                repository_phase: None,
                subjects: vec!["binding.data".to_owned()],
            }],
            digest: String::new(),
        };

        let decision = ThinKernel.evaluate(&snapshot, &rules, &complete_empty_detection());

        assert_eq!(decision.state, "needs-analysis");
        assert_eq!(decision.action.unwrap().action, "analyze-requirements");
        assert_eq!(
            decision.requirement_instances[0].subject_refs,
            ["data.accounts"]
        );
    }

    #[test]
    fn attestation_accepts_a_fresh_satisfied_result() {
        let snapshot = evidence_snapshot();
        assert!(is_satisfied(
            &evidence_instance(Assurance::Attestation),
            &snapshot
        ));
    }

    #[test]
    fn evidence_backed_requires_current_passing_clause_coverage() {
        let instance = evidence_instance(Assurance::EvidenceBacked);
        let snapshot = evidence_snapshot();
        assert!(is_satisfied(&instance, &snapshot));

        let mut missing_evidence = snapshot.clone();
        missing_evidence.evidence.clear();
        assert!(!is_satisfied(&instance, &missing_evidence));

        let mut stale_evidence = snapshot.clone();
        stale_evidence.evidence[0]["git_revision"] = json!("revision-1");
        assert!(!is_satisfied(&instance, &stale_evidence));

        let mut failed_evidence = snapshot.clone();
        failed_evidence.evidence[0]["outcome"] = json!("failed");
        assert!(!is_satisfied(&instance, &failed_evidence));

        let mut uncovered_clause = snapshot.clone();
        uncovered_clause.evidence[0]["contract_clause_refs"] = json!([]);
        assert!(!is_satisfied(&instance, &uncovered_clause));

        let mut missing_report = snapshot;
        missing_report.evidence[0]["artifact"] = json!({});
        assert!(!is_satisfied(&instance, &missing_report));
    }

    #[test]
    fn not_applicable_signal_requires_an_independent_challenge() {
        let (snapshot, rule_index, detection) = not_applicable_signal_case();
        let decision = ThinKernel.evaluate(&snapshot, &rule_index, &detection);
        assert_eq!(decision.state, "needs-pre-build-challenge");
        let action = decision.action.as_ref().unwrap();
        assert_eq!(action.role, "Challenger");
        assert_eq!(
            action.requirement_instances[0].requirement_id,
            SIGNAL_APPLICABILITY_REVIEW_REQUIREMENT
        );

        let context = ContextCompiler
            .compile(&decision, &snapshot, &detection)
            .unwrap();
        let not_applicable = &context.payload["not_applicable_signal_candidates"][0];
        assert_eq!(not_applicable["disposition"], "not-applicable");
        assert_eq!(not_applicable["reason"], "write APIではなくread-only probe");
        assert_eq!(
            not_applicable["disposition_result_id"],
            "result.signal-review"
        );

        let explanation = ExplanationBuilder.build(&snapshot, &rule_index, &detection, &decision);
        assert_eq!(
            explanation.candidates[0].disposition,
            "applicability-pending"
        );
    }

    #[test]
    fn changed_evidence_reopens_only_a_not_applicable_candidate_after_build() {
        let (mut snapshot, rule_index, mut detection) = not_applicable_signal_case();
        snapshot.repository["phase"] = json!("post-build");
        snapshot.artifact_digests.insert(
            "code.place-order".to_owned(),
            format!("sha256:{}", "9".repeat(64)),
        );
        detection.candidates[0].evidence_digest = format!("sha256:{}", "9".repeat(64));

        let decision = ThinKernel.evaluate(&snapshot, &rule_index, &detection);

        assert_eq!(decision.state, "needs-post-build-analysis");
        assert_eq!(
            decision.action.as_ref().unwrap().action,
            "review-risk-signals"
        );
    }

    #[test]
    fn a_new_logical_candidate_after_build_stays_in_post_build_analysis() {
        let (mut snapshot, rule_index, mut detection) = not_applicable_signal_case();
        snapshot.repository["phase"] = json!("post-build");
        snapshot.results[0]["payload"]["reviewed_candidates"][0]["status"] = json!("confirmed");
        let confirmed_fingerprint = detection.candidates[0].fingerprint.clone();
        let new_fingerprint = format!("sha256:{}", "1".repeat(64));
        let mut new_candidate = detection.candidates[0].clone();
        new_candidate
            .bindings
            .insert("data".to_owned(), "data.customers".to_owned());
        new_candidate.fingerprint = new_fingerprint.clone();
        detection.candidates.push(new_candidate);

        let decision = ThinKernel.evaluate(&snapshot, &rule_index, &detection);

        assert_eq!(decision.state, "needs-post-build-analysis");
        let action = decision.action.as_ref().unwrap();
        assert_eq!(action.action, "review-risk-signals");
        assert_eq!(action.candidate_fingerprints, [new_fingerprint]);
        assert!(
            !action
                .candidate_fingerprints
                .contains(&confirmed_fingerprint)
        );
    }

    #[test]
    fn applicability_challenge_accepts_or_fails_closed_per_candidate() {
        let (mut accepted_snapshot, rule_index, detection) = not_applicable_signal_case();
        let pending = ThinKernel.evaluate(&accepted_snapshot, &rule_index, &detection);
        add_applicability_challenge_result(
            &mut accepted_snapshot,
            &pending,
            &detection,
            "satisfied",
        );
        let accepted = ThinKernel.evaluate(&accepted_snapshot, &rule_index, &detection);
        assert_eq!(accepted.state, "ready-to-build");
        assert!(
            accepted
                .requirement_instances
                .iter()
                .all(|instance| instance.requirement_id != "affected-data-confirmed")
        );
        let explanation =
            ExplanationBuilder.build(&accepted_snapshot, &rule_index, &detection, &accepted);
        assert_eq!(explanation.candidates[0].disposition, "not-applicable");

        let (mut rejected_snapshot, rule_index, detection) = not_applicable_signal_case();
        let pending = ThinKernel.evaluate(&rejected_snapshot, &rule_index, &detection);
        add_applicability_challenge_result(
            &mut rejected_snapshot,
            &pending,
            &detection,
            "unsatisfied",
        );
        let rejected = ThinKernel.evaluate(&rejected_snapshot, &rule_index, &detection);
        assert_eq!(rejected.state, "needs-analysis");
        assert!(rejected.requirement_instances.iter().any(|instance| {
            instance.requirement_id == "affected-data-confirmed" && instance.status == "unsatisfied"
        }));

        let explanation =
            ExplanationBuilder.build(&rejected_snapshot, &rule_index, &detection, &rejected);
        assert_eq!(explanation.candidates[0].disposition, "confirmed");
        assert!(explanation.requirements.iter().any(|requirement| {
            requirement.requirement_id == SIGNAL_APPLICABILITY_REVIEW_REQUIREMENT
                && requirement.satisfaction_basis == "completed-signal-applicability-review"
                && requirement.result_checks[0].accepted
        }));
    }

    #[test]
    fn only_matching_stale_or_failed_clauses_require_revalidation() {
        let selected = RequirementInstance {
            requirement_id: "data-evidence-recorded".to_owned(),
            subject_refs: vec!["data.orders".to_owned()],
            instance_key: "data-evidence-recorded|data.orders".to_owned(),
            selected_by: vec!["test.rule".to_owned()],
            definition_digest: format!("sha256:{}", "a".repeat(64)),
            phase: "before-merge".to_owned(),
            role: "Builder".to_owned(),
            result_schema: "result.evidence".to_owned(),
            depends_on: Vec::new(),
            context: Vec::new(),
            assurance: Assurance::EvidenceBacked,
            status: "unsatisfied".to_owned(),
        };
        let clause = |clause_ref: &str, target: &str, status: &str| ClauseHealth {
            contract_id: clause_ref.split('#').next().unwrap().to_owned(),
            clause_id: clause_ref.split('#').nth(1).unwrap().to_owned(),
            clause_ref: clause_ref.to_owned(),
            text: "test clause".to_owned(),
            applies_to: vec![target.to_owned()],
            authority_ref: None,
            status: status.to_owned(),
            evidence_refs: Vec::new(),
            verification_result_ids: Vec::new(),
            stale_refs: Vec::new(),
        };
        let health = ContractHealthReport {
            schema_version: "1".to_owned(),
            repository_revision: Some("revision.test".to_owned()),
            summary: ContractHealthSummary {
                total: 3,
                verified: 0,
                stale: 1,
                unverified: 1,
                failed: 1,
            },
            clauses: vec![
                clause("contract.orders#persistence", "data.orders", "stale"),
                clause("contract.other#failure", "data.other", "failed"),
                clause("contract.orders#new", "data.orders", "unverified"),
            ],
        };

        let instances = contract_revalidation_instances(&health, &[selected]);

        assert_eq!(instances.len(), 1);
        assert_eq!(
            instances[0].instance_key,
            "contract-clause-revalidated|contract.orders#persistence"
        );
        assert_eq!(instances[0].assurance, Assurance::EvidenceBacked);
        assert_eq!(
            instances[0].subject_refs,
            ["contract.orders#persistence", "data.orders"]
        );
    }
}
