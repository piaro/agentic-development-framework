//! Pure validation and construction of one immutable Result Record.
//!
//! Issued-Action lookup, exclusive persistence, and post-submit reevaluation
//! remain Application/Project Store responsibilities.

use crate::APPLICATION_PROTOCOL_VERSION;
use crate::canonical_digest;
use crate::context::GeneratedContext;
use crate::contract_scope::{clause_matches_subjects, contract_matches_subjects};
use crate::kernel::{ProjectSnapshot, evidence_matches_inputs};
use crate::project::{CONTRACT_INDEX_REF, DECISION_INDEX_REF, REPOSITORY_REVISION_REF};
use crate::schema::SchemaRegistry;
use crate::signal_catalog::{
    SignalCatalogRegistry, TYPED_FACT_DETECTOR_ID, TYPED_FACT_DETECTOR_VERSION,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

pub const RESULT_SUBMISSION_PROTOCOL_VERSION: &str = APPLICATION_PROTOCOL_VERSION;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResultSubmission {
    pub change_id: String,
    pub action_id: String,
    pub context_digest: String,
    pub role: String,
    pub result_schema: String,
    pub payload: Value,
    pub output_refs: Vec<String>,
    #[serde(default)]
    pub execution: Option<Value>,
}

pub fn prepare_result(
    context: &GeneratedContext,
    current: &ProjectSnapshot,
    submission: &ResultSubmission,
    schema_registry: &SchemaRegistry,
) -> Result<Value, ResultSubmissionError> {
    if context.action_id != submission.action_id {
        return Err(error("action does not match issued context"));
    }
    if context.digest != submission.context_digest {
        return Err(error("context digest does not match issued action"));
    }
    if context.role != submission.role {
        return Err(error("role does not match issued action"));
    }
    let expected_schema = context.payload["action"]["expected_result_schema"]
        .as_str()
        .ok_or_else(|| error("issued Context has no expected Result Schema"))?;
    if expected_schema != submission.result_schema {
        return Err(error(format!(
            "result schema mismatch: expected {expected_schema}, got {}",
            submission.result_schema
        )));
    }
    if current.change_id != submission.change_id {
        return Err(error(format!(
            "change does not match current Snapshot: expected {}, got {}",
            current.change_id, submission.change_id
        )));
    }

    let output_ref_set: BTreeSet<&str> =
        submission.output_refs.iter().map(String::as_str).collect();
    if output_ref_set.len() != submission.output_refs.len() {
        return Err(error("output refs must be unique"));
    }
    for (reference, issued_digest) in &context.source_digests {
        let current_digest = current.artifact_digests.get(reference);
        if current_digest != Some(issued_digest)
            && !output_ref_set.contains(reference.as_str())
            && !index_change_is_output(reference, &output_ref_set)
        {
            return Err(error(format!("issued context is stale: {reference}")));
        }
    }
    let mut missing_outputs = submission
        .output_refs
        .iter()
        .filter(|reference| !current.artifact_digests.contains_key(*reference))
        .cloned()
        .collect::<Vec<_>>();
    missing_outputs.sort();
    if !missing_outputs.is_empty() {
        return Err(error(format!(
            "output ref does not exist: {}",
            missing_outputs.join(", ")
        )));
    }
    let mut input_refs = context.source_digests.clone();
    let mut freshness_refs = context.source_digests.clone();
    if submission.result_schema == "result.impact-assessment" {
        input_refs.remove(REPOSITORY_REVISION_REF);
        freshness_refs.remove(REPOSITORY_REVISION_REF);
    }
    for index_ref in [CONTRACT_INDEX_REF, DECISION_INDEX_REF] {
        if index_change_is_output(index_ref, &output_ref_set)
            && let Some(digest) = current.artifact_digests.get(index_ref)
        {
            freshness_refs.insert(index_ref.to_owned(), digest.clone());
        }
    }
    for reference in &submission.output_refs {
        freshness_refs.insert(
            reference.clone(),
            current.artifact_digests[reference].clone(),
        );
    }

    let mut payload = submission.payload.clone();
    schema_registry
        .validate_result_payload(&submission.result_schema, &payload)
        .map_err(|validation| error(validation.to_string()))?;
    if submission.result_schema == "result.impact-assessment" {
        let mut selected_inputs = validate_impact_assessment(context, current, &payload)?;
        selected_inputs.remove(REPOSITORY_REVISION_REF);
        input_refs.extend(selected_inputs.clone());
        freshness_refs.extend(selected_inputs);
    }
    if submission.result_schema == "result.risk-signal-review" {
        validate_candidate_reviews(context, &payload)?;
    }
    enrich_outcomes(context, current, &mut payload, &submission.output_refs)?;

    let mut output_refs = submission.output_refs.clone();
    output_refs.sort();
    let execution = execution_record(context, submission.execution.as_ref())?;
    let body = json!({
        "schema_version": RESULT_SUBMISSION_PROTOCOL_VERSION,
        "change_id": submission.change_id,
        "action_id": submission.action_id,
        "role": submission.role,
        "result_schema": submission.result_schema,
        "context_digest": submission.context_digest,
        "input_refs": input_refs,
        "output_refs": output_refs,
        "freshness_refs": freshness_refs,
        "execution": execution,
        "payload": payload,
    });
    let mut identity_body = body.clone();
    identity_body
        .as_object_mut()
        .expect("Result body is an object")
        .remove("execution");
    let digest =
        canonical_digest(&identity_body).map_err(|canonical| error(canonical.to_string()))?;
    let mut result = body
        .as_object()
        .expect("Result body is constructed as an object")
        .clone();
    result.insert(
        "id".to_owned(),
        Value::String(format!(
            "result.{}",
            digest
                .strip_prefix("sha256:")
                .expect("canonical digests have a sha256 prefix")
                .chars()
                .take(20)
                .collect::<String>()
        )),
    );
    let result = Value::Object(result);
    schema_registry
        .validate("result", &result)
        .map_err(|validation| error(validation.to_string()))?;
    Ok(result)
}

fn index_change_is_output(reference: &str, output_refs: &BTreeSet<&str>) -> bool {
    (reference == CONTRACT_INDEX_REF
        && output_refs
            .iter()
            .any(|output| output.starts_with("contract.")))
        || (reference == DECISION_INDEX_REF
            && output_refs
                .iter()
                .any(|output| output.starts_with("decision.")))
}

fn execution_record(
    context: &GeneratedContext,
    reported: Option<&Value>,
) -> Result<Value, ResultSubmissionError> {
    let mut execution = match reported {
        None => Map::new(),
        Some(Value::Object(values)) => values.clone(),
        Some(_) => return Err(error("execution must be an object")),
    };
    if execution.contains_key("context_bytes") {
        return Err(error("execution.context_bytes is measured by ADF"));
    }
    for field in [
        "duration_ms",
        "input_tokens",
        "output_tokens",
        "tool_calls",
        "retries",
    ] {
        if execution
            .get(field)
            .is_some_and(|value| value.as_u64().is_none())
        {
            return Err(error(format!(
                "execution.{field} must be a non-negative integer"
            )));
        }
    }
    let context_bytes = serde_json::to_vec(context)
        .map_err(|serialization| error(serialization.to_string()))?
        .len() as u64;
    execution.insert("context_bytes".to_owned(), Value::from(context_bytes));
    Ok(Value::Object(execution))
}

fn validate_impact_assessment(
    context: &GeneratedContext,
    current: &ProjectSnapshot,
    payload: &Value,
) -> Result<BTreeMap<String, String>, ResultSubmissionError> {
    let status = payload["status"]
        .as_str()
        .expect("Impact Assessment Schema guarantees status");
    let impacts = array_field(payload, "impacts");
    let unknowns = array_field(payload, "unknowns");
    match status {
        "no-impact" if !impacts.is_empty() || !unknowns.is_empty() => {
            return Err(error("no-impact requires empty impacts and unknowns"));
        }
        "impacts-identified" if impacts.is_empty() || !unknowns.is_empty() => {
            return Err(error(
                "impacts-identified requires at least one impact and no unknowns",
            ));
        }
        "inconclusive" if unknowns.is_empty() => {
            return Err(error("inconclusive requires at least one unknown"));
        }
        _ => {}
    }

    let signal_registry =
        SignalCatalogRegistry::built_in().map_err(|catalog| error(catalog.to_string()))?;
    let mut seen = BTreeSet::new();
    for impact in impacts {
        let signal = impact["signal"]
            .as_str()
            .expect("Impact Assessment Schema guarantees signal");
        let bindings = impact["bindings"]
            .as_object()
            .expect("Impact Assessment Schema guarantees bindings")
            .iter()
            .map(|(key, value)| {
                (
                    key.clone(),
                    value
                        .as_str()
                        .expect("Impact Assessment Schema guarantees binding strings")
                        .to_owned(),
                )
            })
            .collect::<BTreeMap<_, _>>();
        signal_registry
            .validate_signal_candidate(
                signal,
                TYPED_FACT_DETECTOR_ID,
                TYPED_FACT_DETECTOR_VERSION,
                &bindings,
            )
            .map_err(|catalog| error(catalog.to_string()))?;
        let identity = canonical_digest(&json!({"signal": signal, "bindings": bindings}))
            .map_err(|canonical| error(canonical.to_string()))?;
        if !seen.insert(identity) {
            return Err(error("impacts must be unique by signal and bindings"));
        }
    }

    let offered = assessment_offered_refs(context);
    let selected = array_field(payload, "basis_refs")
        .iter()
        .chain(
            impacts
                .iter()
                .flat_map(|impact| array_field(impact, "governing_refs")),
        )
        .filter_map(Value::as_str)
        .collect::<BTreeSet<_>>();
    let unknown = selected.difference(&offered).copied().collect::<Vec<_>>();
    if !unknown.is_empty() {
        return Err(error(format!(
            "Impact Assessment refers to inputs not offered by the issued Context: {}",
            unknown.join(", ")
        )));
    }
    let mut selected_inputs = BTreeMap::new();
    for reference in selected {
        let digest = current.artifact_digests.get(reference).ok_or_else(|| {
            error(format!(
                "Impact Assessment input is not current: {reference}"
            ))
        })?;
        selected_inputs.insert(reference.to_owned(), digest.clone());
    }
    Ok(selected_inputs)
}

fn assessment_offered_refs(context: &GeneratedContext) -> BTreeSet<&str> {
    let mut offered = context
        .source_refs
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    offered.extend(
        array_field(&context.payload, "repository_index")
            .iter()
            .filter_map(|item| item["ref"].as_str()),
    );
    for contract in array_field(&context.payload, "governance_index") {
        if let Some(id) = contract["id"].as_str() {
            offered.insert(id);
        }
        for clause in array_field(contract, "clauses") {
            if let Some(reference) = clause["ref"].as_str() {
                offered.insert(reference);
            }
            if let Some(reference) = clause["authority_ref"].as_str() {
                offered.insert(reference);
            }
        }
    }
    offered.extend(
        array_field(&context.payload, "decision_index")
            .iter()
            .filter_map(|decision| decision["id"].as_str()),
    );
    offered
}

fn validate_candidate_reviews(
    context: &GeneratedContext,
    payload: &Value,
) -> Result<(), ResultSubmissionError> {
    let offered = array_field(&context.payload, "signal_candidates")
        .iter()
        .filter_map(|candidate| {
            candidate["fingerprint"]
                .as_str()
                .map(|fingerprint| (fingerprint, candidate))
        })
        .collect::<BTreeMap<_, _>>();
    let mut reviewed = BTreeSet::new();
    for review in array_field(payload, "reviewed_candidates") {
        let fingerprint = review["fingerprint"]
            .as_str()
            .expect("Result payload Schema guarantees a fingerprint");
        let candidate = offered.get(fingerprint).ok_or_else(|| {
            error(format!(
                "candidate was not offered by issued action: {fingerprint}"
            ))
        })?;
        if !reviewed.insert(fingerprint) {
            return Err(error(format!(
                "candidate was reviewed more than once: {fingerprint}"
            )));
        }
        let evidence_refs = string_set(&candidate["evidence_refs"]);
        let unknown = string_set(&review["basis_refs"])
            .difference(&evidence_refs)
            .copied()
            .collect::<Vec<_>>();
        if !unknown.is_empty() {
            return Err(error(format!(
                "candidate review refers to evidence not offered by issued action: {}",
                unknown.join(", ")
            )));
        }
    }
    Ok(())
}

fn enrich_outcomes(
    context: &GeneratedContext,
    current: &ProjectSnapshot,
    payload: &mut Value,
    output_refs: &[String],
) -> Result<(), ResultSubmissionError> {
    let requirements = array_field(&context.payload, "requirement_instances");
    let selectors = requirements
        .iter()
        .filter_map(|item| {
            item["instance_key"]
                .as_str()
                .map(|key| (key, string_set(&item["context_selectors"])))
        })
        .collect::<BTreeMap<_, _>>();
    let subjects = requirements
        .iter()
        .filter_map(|item| {
            item["instance_key"]
                .as_str()
                .map(|key| (key, string_set(&item["subject_refs"])))
        })
        .collect::<BTreeMap<_, _>>();
    let assurances = requirements
        .iter()
        .filter_map(|item| {
            item["instance_key"]
                .as_str()
                .map(|key| (key, item["assurance"].as_str().unwrap_or("attestation")))
        })
        .collect::<BTreeMap<_, _>>();
    let outcomes = payload
        .get_mut("outcomes")
        .and_then(Value::as_array_mut)
        .map(Vec::as_mut_slice)
        .unwrap_or(&mut []);
    for outcome in outcomes {
        let instance_key = outcome["instance_key"]
            .as_str()
            .expect("Result payload Schema guarantees an instance key");
        let instance_inputs = context
            .instance_source_digests
            .get(instance_key)
            .ok_or_else(|| {
                error(format!(
                    "outcome does not belong to issued action: {instance_key}"
                ))
            })?;
        let mut instance_freshness = instance_inputs.clone();
        for reference in output_refs {
            if output_is_relevant(
                reference,
                instance_key,
                instance_inputs,
                selectors
                    .get(instance_key)
                    .expect("issued Context contains matching selectors"),
                subjects
                    .get(instance_key)
                    .expect("issued Context contains matching subjects"),
                current,
            ) {
                instance_freshness.insert(
                    reference.clone(),
                    current.artifact_digests[reference].clone(),
                );
            }
        }
        let known_refs: BTreeSet<&str> = instance_freshness.keys().map(String::as_str).collect();
        let unknown = string_set(&outcome["basis_refs"])
            .difference(&known_refs)
            .copied()
            .collect::<Vec<_>>();
        if !unknown.is_empty() {
            return Err(error(format!(
                "outcome basis ref does not belong to issued action: {}",
                unknown.join(", ")
            )));
        }
        if assurances.get(instance_key) == Some(&"evidence-backed") {
            validate_evidence_basis(instance_key, instance_inputs, outcome, current)?;
        }
        let object = outcome
            .as_object_mut()
            .expect("Result payload Schema guarantees an outcome object");
        object.insert("input_refs".to_owned(), string_map_value(instance_inputs));
        object.insert(
            "freshness_refs".to_owned(),
            string_map_value(&instance_freshness),
        );
    }
    Ok(())
}

fn output_is_relevant(
    reference: &str,
    instance_key: &str,
    instance_inputs: &BTreeMap<String, String>,
    selectors: &BTreeSet<&str>,
    subjects: &BTreeSet<&str>,
    snapshot: &ProjectSnapshot,
) -> bool {
    if instance_inputs.contains_key(reference) {
        return true;
    }
    let source_kind = reference.split_once('.').map_or(reference, |value| value.0);
    match source_kind {
        "contract" => {
            selectors.contains("contracts")
                || (selectors.contains("matching-contracts")
                    && snapshot.contracts.iter().any(|contract| {
                        contract["id"].as_str() == Some(reference)
                            && contract_matches_subjects(contract, subjects)
                    }))
        }
        "decision" => {
            if selectors.contains("decisions") {
                return true;
            }
            let matching_authorities = snapshot
                .contracts
                .iter()
                .filter(|contract| contract_matches_subjects(contract, subjects))
                .flat_map(|contract| {
                    array_field(contract, "clauses")
                        .iter()
                        .filter(move |clause| clause_matches_subjects(contract, clause, subjects))
                })
                .filter_map(|clause| clause["authority_ref"].as_str())
                .collect::<BTreeSet<_>>();
            selectors.contains("matching-decisions") && matching_authorities.contains(reference)
        }
        "evidence" => {
            selectors.contains("evidence")
                || (selectors.contains("matching-evidence")
                    && snapshot.evidence.iter().any(|evidence| {
                        evidence["id"].as_str() == Some(reference)
                            && (overlaps(&evidence["applies_to"], subjects)
                                || string_set(&evidence["requirement_instances"])
                                    .contains(instance_key))
                    }))
                || snapshot.evidence.iter().any(|evidence| {
                    evidence["id"].as_str() == Some(reference)
                        && string_set(&evidence["requirement_instances"]).contains(instance_key)
                })
        }
        "result" => selectors.contains("dependency-results") || selectors.contains("results"),
        _ => false,
    }
}

fn validate_evidence_basis(
    instance_key: &str,
    instance_inputs: &BTreeMap<String, String>,
    outcome: &Value,
    current: &ProjectSnapshot,
) -> Result<(), ResultSubmissionError> {
    let basis_refs = string_set(&outcome["basis_refs"]);
    let matching = current.evidence.iter().any(|evidence| {
        evidence["id"]
            .as_str()
            .is_some_and(|evidence_id| basis_refs.contains(evidence_id))
            && string_set(&evidence["requirement_instances"]).contains(instance_key)
            && evidence_matches_inputs(evidence, instance_inputs, current)
    });
    if !matching {
        return Err(error(format!(
            "evidence-backed outcome requires an Evidence Record for: {instance_key}"
        )));
    }
    Ok(())
}

fn overlaps(value: &Value, subjects: &BTreeSet<&str>) -> bool {
    array_values(value)
        .iter()
        .filter_map(Value::as_str)
        .any(|subject| subjects.contains(subject))
}

fn string_set(value: &Value) -> BTreeSet<&str> {
    array_values(value)
        .iter()
        .filter_map(Value::as_str)
        .collect()
}

fn string_map_value(values: &BTreeMap<String, String>) -> Value {
    Value::Object(
        values
            .iter()
            .map(|(key, value)| (key.clone(), Value::String(value.clone())))
            .collect::<Map<_, _>>(),
    )
}

fn array_field<'a>(value: &'a Value, field: &str) -> &'a [Value] {
    array_values(&value[field])
}

fn array_values(value: &Value) -> &[Value] {
    value.as_array().map(Vec::as_slice).unwrap_or(&[])
}

fn error(message: impl Into<String>) -> ResultSubmissionError {
    ResultSubmissionError {
        message: message.into(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResultSubmissionError {
    message: String,
}

impl fmt::Display for ResultSubmissionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ResultSubmissionError {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn registry() -> SchemaRegistry {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("schemas/v1");
        SchemaRegistry::load(root).unwrap()
    }

    fn context() -> GeneratedContext {
        let change_digest = format!("sha256:{}", "a".repeat(64));
        GeneratedContext {
            action_id: "action.test".to_owned(),
            role: "Builder".to_owned(),
            source_refs: vec!["change.test".to_owned()],
            source_digests: BTreeMap::from([("change.test".to_owned(), change_digest.clone())]),
            instance_source_digests: BTreeMap::from([(
                "tests-passed|operation.test".to_owned(),
                BTreeMap::from([("change.test".to_owned(), change_digest)]),
            )]),
            contract_clause_projection_version: "1".to_owned(),
            contract_clauses: Vec::new(),
            contract_clauses_digest: canonical_digest(&json!([])).unwrap(),
            payload: json!({
                "action": {"expected_result_schema": "result.evidence"},
                "requirement_instances": [{
                    "instance_key": "tests-passed|operation.test",
                    "subject_refs": ["operation.test"],
                    "context_selectors": ["matching-contracts"],
                    "assurance": "evidence-backed"
                }],
                "signal_candidates": []
            }),
            digest: format!("sha256:{}", "c".repeat(64)),
        }
    }

    fn snapshot() -> ProjectSnapshot {
        let change_digest = format!("sha256:{}", "a".repeat(64));
        let evidence_digest = format!("sha256:{}", "b".repeat(64));
        ProjectSnapshot {
            change_id: "change.test".to_owned(),
            change: json!({}),
            contracts: Vec::new(),
            decisions: Vec::new(),
            results: Vec::new(),
            evidence: vec![json!({
                "schema_version": "1",
                "id": "evidence.test",
                "change_id": "change.test",
                "requirement_instances": ["tests-passed|operation.test"],
                "git_revision": "revision-2",
                "method": "cargo test --locked",
                "outcome": "passed",
                "summary": "tests passed",
                "artifact": {
                    "uri": "artifact://ci/test-report.json",
                    "digest": format!("sha256:{}", "d".repeat(64)),
                    "exit_code": 0
                }
            })],
            repository: json!({"revision": "revision-2"}),
            artifact_digests: BTreeMap::from([
                ("change.test".to_owned(), change_digest),
                ("evidence.test".to_owned(), evidence_digest),
            ]),
            digest: String::new(),
        }
    }

    fn submission(basis_refs: Value, output_refs: Vec<String>) -> ResultSubmission {
        ResultSubmission {
            change_id: "change.test".to_owned(),
            action_id: "action.test".to_owned(),
            context_digest: format!("sha256:{}", "c".repeat(64)),
            role: "Builder".to_owned(),
            result_schema: "result.evidence".to_owned(),
            payload: json!({
                "outcomes": [{
                    "instance_key": "tests-passed|operation.test",
                    "definition_digest": format!("sha256:{}", "e".repeat(64)),
                    "status": "satisfied",
                    "summary": "tests passed",
                    "basis_refs": basis_refs
                }]
            }),
            output_refs,
            execution: None,
        }
    }

    #[test]
    fn evidence_backed_submission_rejects_a_shallow_attestation() {
        let error = prepare_result(
            &context(),
            &snapshot(),
            &submission(json!(["change.test"]), Vec::new()),
            &registry(),
        )
        .unwrap_err();
        assert_eq!(
            error.to_string(),
            "evidence-backed outcome requires an Evidence Record for: \
             tests-passed|operation.test"
        );
    }

    #[test]
    fn evidence_backed_submission_accepts_a_matching_evidence_output() {
        let result = prepare_result(
            &context(),
            &snapshot(),
            &submission(json!(["evidence.test"]), vec!["evidence.test".to_owned()]),
            &registry(),
        )
        .unwrap();
        assert_eq!(
            result["payload"]["outcomes"][0]["freshness_refs"]["evidence.test"],
            format!("sha256:{}", "b".repeat(64))
        );
    }

    #[test]
    fn evidence_backed_submission_uses_content_binding_across_revision_changes() {
        let mut snapshot = snapshot();
        snapshot.evidence[0]["git_revision"] = json!("revision-before-record-commit");
        snapshot.evidence[0]["input_refs"] = json!({
            "change.test": format!("sha256:{}", "a".repeat(64))
        });

        prepare_result(
            &context(),
            &snapshot,
            &submission(json!(["evidence.test"]), vec!["evidence.test".to_owned()]),
            &registry(),
        )
        .unwrap();

        snapshot.evidence[0]["input_refs"] = json!({
            "change.test": format!("sha256:{}", "f".repeat(64))
        });
        let error = prepare_result(
            &context(),
            &snapshot,
            &submission(json!(["evidence.test"]), vec!["evidence.test".to_owned()]),
            &registry(),
        )
        .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("evidence-backed outcome requires an Evidence Record")
        );
    }

    fn impact_context() -> GeneratedContext {
        let digest = |character: char| format!("sha256:{}", character.to_string().repeat(64));
        GeneratedContext {
            action_id: "action.impact".to_owned(),
            role: "Analyst".to_owned(),
            source_refs: vec![
                "change.test".to_owned(),
                "repository.revision".to_owned(),
                "repository.content".to_owned(),
                "contracts.index".to_owned(),
            ],
            source_digests: BTreeMap::from([
                ("change.test".to_owned(), digest('a')),
                ("repository.revision".to_owned(), digest('9')),
                ("repository.content".to_owned(), digest('b')),
                ("contracts.index".to_owned(), digest('c')),
            ]),
            instance_source_digests: BTreeMap::new(),
            contract_clause_projection_version: "1".to_owned(),
            contract_clauses: Vec::new(),
            contract_clauses_digest: canonical_digest(&json!([])).unwrap(),
            payload: json!({
                "action": {"expected_result_schema": "result.impact-assessment"},
                "requirement_instances": [],
                "signal_candidates": [],
                "repository_index": [{"ref": "code.accounts"}],
                "governance_index": [{
                    "id": "contract.accounts",
                    "clauses": [{"ref": "contract.accounts#persist"}]
                }],
                "decision_index": []
            }),
            digest: digest('d'),
        }
    }

    fn impact_snapshot() -> ProjectSnapshot {
        let digest = |character: char| format!("sha256:{}", character.to_string().repeat(64));
        ProjectSnapshot {
            change_id: "change.test".to_owned(),
            change: json!({}),
            contracts: Vec::new(),
            decisions: Vec::new(),
            results: Vec::new(),
            evidence: Vec::new(),
            repository: json!({}),
            artifact_digests: BTreeMap::from([
                ("change.test".to_owned(), digest('a')),
                ("repository.revision".to_owned(), digest('9')),
                ("repository.content".to_owned(), digest('b')),
                ("contracts.index".to_owned(), digest('c')),
                ("code.accounts".to_owned(), digest('e')),
                ("contract.accounts".to_owned(), digest('f')),
                ("contract.accounts#persist".to_owned(), digest('1')),
            ]),
            digest: String::new(),
        }
    }

    fn impact_submission(status: &str, impacts: Value, unknowns: Value) -> ResultSubmission {
        ResultSubmission {
            change_id: "change.test".to_owned(),
            action_id: "action.impact".to_owned(),
            context_digest: format!("sha256:{}", "d".repeat(64)),
            role: "Analyst".to_owned(),
            result_schema: "result.impact-assessment".to_owned(),
            payload: json!({
                "status": status,
                "summary": "Assessed the requested account change.",
                "impacts": impacts,
                "unknowns": unknowns,
                "basis_refs": ["change.test", "code.accounts"]
            }),
            output_refs: Vec::new(),
            execution: Some(json!({
                "duration_ms": 25,
                "model": "economy-test",
                "input_tokens": 120,
                "output_tokens": 40,
                "tool_calls": 1,
                "retries": 0
            })),
        }
    }

    #[test]
    fn impact_assessment_validates_declared_signals_and_records_lightweight_metrics() {
        let submission = impact_submission(
            "impacts-identified",
            json!([{
                "signal": "persistent-data-write",
                "bindings": {
                    "data": "data.accounts",
                    "operation": "operation.register-account"
                },
                "description": "Stores a new account.",
                "risk": "medium",
                "governing_refs": ["contract.accounts#persist"]
            }]),
            json!([]),
        );

        let result = prepare_result(
            &impact_context(),
            &impact_snapshot(),
            &submission,
            &registry(),
        )
        .unwrap();

        assert_eq!(result["execution"]["duration_ms"], 25);
        assert!(result["input_refs"].get("repository.revision").is_none());
        assert!(
            result["freshness_refs"]
                .get("repository.revision")
                .is_none()
        );
        assert_eq!(result["execution"]["model"], "economy-test");
        assert!(result["execution"]["context_bytes"].as_u64().unwrap() > 0);
        assert_eq!(
            result["input_refs"]["contract.accounts#persist"],
            format!("sha256:{}", "1".repeat(64))
        );

        let mut alternate_execution = submission.clone();
        alternate_execution.execution = Some(json!({
            "duration_ms": 50,
            "model": "different-observer-value",
            "input_tokens": 240,
            "output_tokens": 80
        }));
        let alternate_result = prepare_result(
            &impact_context(),
            &impact_snapshot(),
            &alternate_execution,
            &registry(),
        )
        .unwrap();
        assert_eq!(result["id"], alternate_result["id"]);
    }

    #[test]
    fn impact_assessment_distinguishes_no_impact_from_inconclusive() {
        let invalid = impact_submission("no-impact", json!([]), json!(["Unknown storage"]));
        let error = prepare_result(&impact_context(), &impact_snapshot(), &invalid, &registry())
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            "no-impact requires empty impacts and unknowns"
        );

        let inconclusive = impact_submission(
            "inconclusive",
            json!([]),
            json!(["The storage boundary is not specified."]),
        );
        assert!(
            prepare_result(
                &impact_context(),
                &impact_snapshot(),
                &inconclusive,
                &registry(),
            )
            .is_ok()
        );
    }
}
