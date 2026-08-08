//! Repository-wide, read-only Contract conformance reporting.
//!
//! Contracts remain authoritative norms. This module derives whether each
//! clause currently has usable Evidence without writing health state back into
//! the Contract or treating an unknown state as verified.

use crate::canonical_digest;
use crate::contract_scope::{
    ClauseEvidenceMode, effective_clause_applies_to, effective_clause_evidence_mode,
};
use crate::schema::SchemaRegistry;
use serde::Serialize;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

pub const CONTRACT_HEALTH_REPORT_SCHEMA_VERSION: &str = "1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ContractHealthSummary {
    pub total: usize,
    pub verified: usize,
    pub stale: usize,
    pub unverified: usize,
    pub failed: usize,
    pub review: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ClauseHealth {
    pub contract_id: String,
    pub clause_id: String,
    pub clause_ref: String,
    pub text: String,
    pub applies_to: Vec<String>,
    pub authority_ref: Option<String>,
    pub evidence_mode: String,
    pub status: String,
    pub evidence_refs: Vec<String>,
    pub verification_result_ids: Vec<String>,
    pub stale_refs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ContractHealthReport {
    pub schema_version: String,
    pub repository_revision: Option<String>,
    pub summary: ContractHealthSummary,
    pub clauses: Vec<ClauseHealth>,
}

impl ContractHealthReport {
    pub fn as_value(&self) -> Value {
        serde_json::to_value(self).expect("Contract Health Report is serializable")
    }

    pub fn render_text(&self) -> String {
        let revision = self.repository_revision.as_deref().unwrap_or("-");
        let mut lines = vec![
            format!("repository_revision: {revision}"),
            format!(
                "clauses: total={} verified={} stale={} unverified={} failed={} review={}",
                self.summary.total,
                self.summary.verified,
                self.summary.stale,
                self.summary.unverified,
                self.summary.failed,
                self.summary.review
            ),
        ];
        for clause in &self.clauses {
            lines.push(format!(
                "  - {} [{}] evidence={} stale_refs={}",
                clause.clause_ref,
                clause.status,
                joined_or_dash(&clause.evidence_refs),
                joined_or_dash(&clause.stale_refs)
            ));
        }
        lines.join("\n") + "\n"
    }
}

pub fn build_contract_health_report(
    project: &Value,
    schema_registry: &SchemaRegistry,
) -> Result<ContractHealthReport, ContractHealthError> {
    let project = project
        .as_object()
        .ok_or_else(|| health_error("project must be an object"))?;
    for (collection, kind) in [
        ("changes", "change"),
        ("contracts", "contract"),
        ("decisions", "decision"),
        ("results", "result"),
        ("evidence", "evidence"),
    ] {
        let records = record_array(project.get(collection), collection)?;
        validate_unique_ids(records, collection)?;
        for record in records {
            schema_registry
                .validate(kind, record)
                .map_err(|error| health_error(format!("invalid {kind} record: {error}")))?;
        }
    }

    let current_digests = current_artifact_digests(project)?;
    let results = record_array(project.get("results"), "results")?;
    let evidence = record_array(project.get("evidence"), "evidence")?;
    let evidence_by_id = evidence
        .iter()
        .filter_map(|record| record["id"].as_str().map(|id| (id.to_owned(), record)))
        .collect::<BTreeMap<_, _>>();

    let mut clauses = Vec::new();
    let mut seen_clause_refs = BTreeSet::new();
    for contract in record_array(project.get("contracts"), "contracts")? {
        let contract_id = required_string(contract, "id", "Contract")?;
        for clause in contract["clauses"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or(&[])
        {
            let clause_id = required_string(clause, "id", "Contract clause")?;
            let clause_ref = format!("{contract_id}#{clause_id}");
            if !seen_clause_refs.insert(clause_ref.clone()) {
                return Err(health_error(format!(
                    "duplicate Contract clause ref: {clause_ref}"
                )));
            }
            let clause_applies_to = effective_clause_applies_to(contract, clause);
            let evidence_mode = effective_clause_evidence_mode(contract, clause);
            clauses.push(clause_health(
                contract_id,
                clause_id,
                &clause_ref,
                clause,
                &clause_applies_to,
                evidence_mode,
                results,
                &evidence_by_id,
                &current_digests,
            ));
        }
    }
    clauses.sort_by(|left, right| left.clause_ref.cmp(&right.clause_ref));

    let summary = ContractHealthSummary {
        total: clauses.len(),
        verified: count_status(&clauses, "verified"),
        stale: count_status(&clauses, "stale"),
        unverified: count_status(&clauses, "unverified"),
        failed: count_status(&clauses, "failed"),
        review: count_status(&clauses, "review"),
    };
    Ok(ContractHealthReport {
        schema_version: CONTRACT_HEALTH_REPORT_SCHEMA_VERSION.to_owned(),
        repository_revision: project
            .get("repository")
            .and_then(|value| value["revision"].as_str())
            .map(str::to_owned),
        summary,
        clauses,
    })
}

#[allow(clippy::too_many_arguments)]
fn clause_health(
    contract_id: &str,
    clause_id: &str,
    clause_ref: &str,
    clause: &Value,
    applies_to: &[String],
    evidence_mode: ClauseEvidenceMode,
    results: &[Value],
    evidence_by_id: &BTreeMap<String, &Value>,
    current_digests: &BTreeMap<String, String>,
) -> ClauseHealth {
    let mut evidence_refs = BTreeSet::new();
    let mut verification_result_ids = BTreeSet::new();
    let mut stale_refs = BTreeSet::new();
    let mut current_success = false;
    let mut current_failure = false;
    let mut historical_verification = false;

    for (evidence_id, evidence) in evidence_by_id {
        if !evidence_covers_clause(evidence, clause_ref, evidence_mode) {
            continue;
        }
        evidence_refs.insert(evidence_id.clone());
        for result in results.iter().filter(|result| {
            result["result_schema"].as_str() == Some("result.evidence")
                && result["role"].as_str() == Some("Builder")
                && result["change_id"] == evidence["change_id"]
        }) {
            for outcome in result["payload"]["outcomes"]
                .as_array()
                .map(Vec::as_slice)
                .unwrap_or(&[])
                .iter()
                .filter(|outcome| string_array(&outcome["basis_refs"]).contains(evidence_id))
            {
                let Some(result_id) = result["id"].as_str() else {
                    continue;
                };
                verification_result_ids.insert(result_id.to_owned());
                let mismatches = stale_refs_for_outcome(
                    outcome,
                    result,
                    current_digests,
                    &[contract_id, evidence_id],
                );
                if mismatches.is_empty() {
                    match (evidence["outcome"].as_str(), outcome["status"].as_str()) {
                        (Some("passed"), Some("satisfied"))
                            if evidence_verifies_outcome(evidence, outcome) =>
                        {
                            current_success = true;
                        }
                        (Some("failed" | "inconclusive"), _) => current_failure = true,
                        _ => {}
                    }
                } else {
                    historical_verification = true;
                    stale_refs.extend(mismatches);
                }
            }
        }
    }

    let status = if evidence_mode == ClauseEvidenceMode::Review {
        "review"
    } else if current_success {
        "verified"
    } else if current_failure {
        "failed"
    } else if historical_verification {
        "stale"
    } else {
        "unverified"
    };
    if status != "stale" {
        stale_refs.clear();
    }
    ClauseHealth {
        contract_id: contract_id.to_owned(),
        clause_id: clause_id.to_owned(),
        clause_ref: clause_ref.to_owned(),
        text: clause["text"].as_str().unwrap_or_default().to_owned(),
        applies_to: applies_to.to_vec(),
        authority_ref: clause["authority_ref"].as_str().map(str::to_owned),
        evidence_mode: evidence_mode.as_str().to_owned(),
        status: status.to_owned(),
        evidence_refs: evidence_refs.into_iter().collect(),
        verification_result_ids: verification_result_ids.into_iter().collect(),
        stale_refs: stale_refs.into_iter().collect(),
    }
}

fn evidence_covers_clause(
    evidence: &Value,
    clause_ref: &str,
    evidence_mode: ClauseEvidenceMode,
) -> bool {
    if !string_array(&evidence["contract_clause_refs"]).contains(&clause_ref.to_owned()) {
        return false;
    }
    if evidence_mode != ClauseEvidenceMode::Direct {
        return true;
    }
    evidence["claims"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[])
        .iter()
        .any(|claim| claim["contract_clause_ref"].as_str() == Some(clause_ref))
}

fn evidence_verifies_outcome(evidence: &Value, outcome: &Value) -> bool {
    let instance_key = outcome["instance_key"].as_str();
    let artifact = evidence["artifact"].as_object();
    evidence["git_revision"]
        .as_str()
        .is_some_and(|value| !value.is_empty())
        && evidence["method"]
            .as_str()
            .is_some_and(|value| !value.is_empty())
        && instance_key.is_some_and(|instance| {
            string_array(&evidence["requirement_instances"]).contains(&instance.to_owned())
        })
        && artifact
            .and_then(|value| value.get("uri"))
            .and_then(Value::as_str)
            .is_some_and(|value| !value.is_empty())
        && artifact
            .and_then(|value| value.get("digest"))
            .and_then(Value::as_str)
            .is_some_and(is_sha256_digest)
        && artifact
            .and_then(|value| value.get("exit_code"))
            .and_then(Value::as_i64)
            == Some(0)
}

fn is_sha256_digest(value: &str) -> bool {
    value
        .strip_prefix("sha256:")
        .is_some_and(|hex| hex.len() == 64 && hex.bytes().all(|byte| byte.is_ascii_hexdigit()))
}

fn current_artifact_digests(
    project: &serde_json::Map<String, Value>,
) -> Result<BTreeMap<String, String>, ContractHealthError> {
    let mut digests = BTreeMap::new();
    for collection in ["changes", "contracts", "decisions", "results", "evidence"] {
        for record in record_array(project.get(collection), collection)? {
            let id = required_string(record, "id", "Project record")?;
            digests.insert(id.to_owned(), digest_value(record)?);
            if collection == "contracts" {
                for clause in record["clauses"]
                    .as_array()
                    .map(Vec::as_slice)
                    .unwrap_or(&[])
                {
                    let clause_id = required_string(clause, "id", "Contract clause")?;
                    digests.insert(format!("{id}#{clause_id}"), digest_value(clause)?);
                }
            }
        }
    }
    for artifact in project
        .get("repository")
        .and_then(|repository| repository["artifacts"].as_array())
        .map(Vec::as_slice)
        .unwrap_or(&[])
    {
        let reference = required_string(artifact, "ref", "Repository artifact")?;
        let digest = artifact["digest"]
            .as_str()
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
            .unwrap_or(digest_value(artifact)?);
        digests.insert(reference.to_owned(), digest);
    }
    Ok(digests)
}

fn stale_refs_for_outcome(
    outcome: &Value,
    result: &Value,
    current_digests: &BTreeMap<String, String>,
    required_refs: &[&str],
) -> Vec<String> {
    let references = outcome
        .get("freshness_refs")
        .or_else(|| result.get("freshness_refs"))
        .or_else(|| result.get("input_refs"));
    let Some(references) = references.and_then(Value::as_object) else {
        return vec!["missing-freshness-manifest".to_owned()];
    };
    let mut stale = references
        .iter()
        .filter(|(reference, digest)| {
            current_digests.get(*reference).map(String::as_str) != digest.as_str()
        })
        .map(|(reference, _)| reference.clone())
        .collect::<BTreeSet<_>>();
    stale.extend(
        required_refs
            .iter()
            .filter(|reference| !references.contains_key(**reference))
            .map(|reference| (*reference).to_owned()),
    );
    stale.into_iter().collect()
}

fn count_status(clauses: &[ClauseHealth], status: &str) -> usize {
    clauses
        .iter()
        .filter(|clause| clause.status == status)
        .count()
}

fn record_array<'a>(
    value: Option<&'a Value>,
    collection: &str,
) -> Result<&'a [Value], ContractHealthError> {
    match value {
        None => Ok(&[]),
        Some(Value::Array(records)) => Ok(records),
        Some(_) => Err(health_error(format!("{collection} must be an array"))),
    }
}

fn validate_unique_ids(records: &[Value], collection: &str) -> Result<(), ContractHealthError> {
    let mut seen = BTreeSet::new();
    for record in records {
        let id = required_string(record, "id", collection)?;
        if !seen.insert(id) {
            return Err(health_error(format!("duplicate {collection} ID: {id}")));
        }
    }
    Ok(())
}

fn required_string<'a>(
    value: &'a Value,
    field: &str,
    label: &str,
) -> Result<&'a str, ContractHealthError> {
    value[field]
        .as_str()
        .ok_or_else(|| health_error(format!("{label} {field} must be a string")))
}

fn string_array(value: &Value) -> Vec<String> {
    value
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn digest_value(value: &Value) -> Result<String, ContractHealthError> {
    canonical_digest(value).map_err(|error| health_error(error.to_string()))
}

fn joined_or_dash(values: &[String]) -> String {
    if values.is_empty() {
        "-".to_owned()
    } else {
        values.join(",")
    }
}

fn health_error(message: impl Into<String>) -> ContractHealthError {
    ContractHealthError {
        message: message.into(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContractHealthError {
    message: String,
}

impl fmt::Display for ContractHealthError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ContractHealthError {}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::path::PathBuf;

    fn registry() -> SchemaRegistry {
        SchemaRegistry::load(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("schemas/v1")).unwrap()
    }

    fn project_with_evidence(evidence_outcome: &str, result_status: &str) -> Value {
        let change = json!({
            "schema_version": "1",
            "id": "change.test",
            "title": "test",
            "intent": "verify Contract health"
        });
        let contract = json!({
            "schema_version": "1",
            "id": "contract.test",
            "applies_to": ["operation.test"],
            "clauses": [{
                "id": "stable-output",
                "text": "output remains stable"
            }]
        });
        let evidence = json!({
            "schema_version": "1",
            "id": "evidence.test",
            "change_id": "change.test",
            "requirement_instances": ["tests-passed|operation.test"],
            "contract_clause_refs": ["contract.test#stable-output"],
            "git_revision": "revision-1",
            "method": "cargo test",
            "outcome": evidence_outcome,
            "summary": "Contract verification",
            "artifact": {
                "uri": "artifact://ci/contract-test.json",
                "digest": format!("sha256:{}", "e".repeat(64)),
                "exit_code": 0
            }
        });
        let contract_digest = digest_value(&contract).unwrap();
        let evidence_digest = digest_value(&evidence).unwrap();
        let code_digest = format!("sha256:{}", "c".repeat(64));
        let freshness_refs = json!({
            "contract.test": contract_digest,
            "code.test": code_digest,
            "evidence.test": evidence_digest
        });
        let result = json!({
            "schema_version": "1",
            "id": "result.test",
            "change_id": "change.test",
            "action_id": "action.test",
            "role": "Builder",
            "result_schema": "result.evidence",
            "context_digest": format!("sha256:{}", "a".repeat(64)),
            "input_refs": freshness_refs,
            "output_refs": ["evidence.test"],
            "freshness_refs": freshness_refs,
            "payload": {
                "outcomes": [{
                    "instance_key": "tests-passed|operation.test",
                    "definition_digest": format!("sha256:{}", "b".repeat(64)),
                    "status": result_status,
                    "summary": "Contract verification",
                    "basis_refs": ["evidence.test"],
                    "input_refs": freshness_refs,
                    "freshness_refs": freshness_refs
                }]
            }
        });
        json!({
            "changes": [change],
            "contracts": [contract],
            "decisions": [],
            "results": [result],
            "evidence": [evidence],
            "repository": {
                "revision": "revision-2",
                "artifacts": [{
                    "ref": "code.test",
                    "digest": code_digest,
                    "applies_to": ["operation.test"]
                }]
            }
        })
    }

    fn replace_evidence_digest(references: &mut Value, evidence_id: &str, evidence_digest: &str) {
        let references = references.as_object_mut().unwrap();
        references.remove("evidence.test");
        references.insert(
            evidence_id.to_owned(),
            Value::String(evidence_digest.to_owned()),
        );
    }

    #[test]
    fn reports_verified_stale_and_failed_without_mutating_contracts() {
        let verified = build_contract_health_report(
            &project_with_evidence("passed", "satisfied"),
            &registry(),
        )
        .unwrap();
        assert_eq!(verified.clauses[0].status, "verified");

        let mut stale = project_with_evidence("passed", "satisfied");
        stale["repository"]["artifacts"][0]["digest"] = json!(format!("sha256:{}", "d".repeat(64)));
        let stale = build_contract_health_report(&stale, &registry()).unwrap();
        assert_eq!(stale.clauses[0].status, "stale");
        assert_eq!(stale.clauses[0].stale_refs, ["code.test"]);

        let failed = build_contract_health_report(
            &project_with_evidence("failed", "unsatisfied"),
            &registry(),
        )
        .unwrap();
        assert_eq!(failed.clauses[0].status, "failed");

        let mut shallow = project_with_evidence("passed", "satisfied");
        shallow["evidence"][0]["artifact"] = json!({});
        let shallow_digest = digest_value(&shallow["evidence"][0]).unwrap();
        shallow["results"][0]["input_refs"]["evidence.test"] =
            Value::String(shallow_digest.clone());
        shallow["results"][0]["freshness_refs"]["evidence.test"] =
            Value::String(shallow_digest.clone());
        shallow["results"][0]["payload"]["outcomes"][0]["input_refs"]["evidence.test"] =
            Value::String(shallow_digest.clone());
        shallow["results"][0]["payload"]["outcomes"][0]["freshness_refs"]["evidence.test"] =
            Value::String(shallow_digest);
        let shallow = build_contract_health_report(&shallow, &registry()).unwrap();
        assert_eq!(shallow.clauses[0].status, "unverified");
    }

    #[test]
    fn reports_a_clause_without_evidence_as_unverified() {
        let mut project = project_with_evidence("passed", "satisfied");
        project["results"] = json!([]);
        project["evidence"] = json!([]);
        let report = build_contract_health_report(&project, &registry()).unwrap();
        assert_eq!(report.clauses[0].status, "unverified");
        assert_eq!(report.summary.unverified, 1);
    }

    #[test]
    fn selective_modes_keep_review_out_of_the_evidence_gate_and_require_direct_claims() {
        let mut review = project_with_evidence("passed", "satisfied");
        review["contracts"][0]["evidence_mode"] = Value::String("review".to_owned());
        review["results"] = json!([]);
        review["evidence"] = json!([]);
        let report = build_contract_health_report(&review, &registry()).unwrap();
        assert_eq!(report.clauses[0].status, "review");
        assert_eq!(report.summary.review, 1);
        assert_eq!(report.summary.unverified, 0);

        let mut direct = project_with_evidence("passed", "satisfied");
        direct["contracts"][0]["evidence_mode"] = Value::String("direct".to_owned());
        let contract_digest = digest_value(&direct["contracts"][0]).unwrap();
        direct["results"][0]["input_refs"]["contract.test"] =
            Value::String(contract_digest.clone());
        direct["results"][0]["freshness_refs"]["contract.test"] =
            Value::String(contract_digest.clone());
        direct["results"][0]["payload"]["outcomes"][0]["input_refs"]["contract.test"] =
            Value::String(contract_digest.clone());
        direct["results"][0]["payload"]["outcomes"][0]["freshness_refs"]["contract.test"] =
            Value::String(contract_digest);
        let without_claim = build_contract_health_report(&direct, &registry()).unwrap();
        assert_eq!(without_claim.clauses[0].status, "unverified");

        direct["evidence"][0]["claims"] = json!([{
            "contract_clause_ref": "contract.test#stable-output",
            "assertion": "The report demonstrates stable output."
        }]);
        let evidence_digest = digest_value(&direct["evidence"][0]).unwrap();
        direct["results"][0]["input_refs"]["evidence.test"] =
            Value::String(evidence_digest.clone());
        direct["results"][0]["freshness_refs"]["evidence.test"] =
            Value::String(evidence_digest.clone());
        direct["results"][0]["payload"]["outcomes"][0]["input_refs"]["evidence.test"] =
            Value::String(evidence_digest.clone());
        direct["results"][0]["payload"]["outcomes"][0]["freshness_refs"]["evidence.test"] =
            Value::String(evidence_digest);
        let with_claim = build_contract_health_report(&direct, &registry()).unwrap();
        assert_eq!(with_claim.clauses[0].status, "verified");
    }

    #[test]
    fn reports_the_effective_clause_scope() {
        let mut project = project_with_evidence("passed", "satisfied");
        project["contracts"][0]["applies_to"] = json!(["operation.test", "operation.unrelated"]);
        project["contracts"][0]["clauses"][0]["applies_to"] = json!(["operation.test"]);
        project["contracts"][0]["clauses"]
            .as_array_mut()
            .unwrap()
            .push(json!({
                "id": "unrelated-output",
                "text": "unrelated output remains stable",
                "applies_to": ["operation.unrelated"]
            }));

        let report = build_contract_health_report(&project, &registry()).unwrap();
        assert_eq!(report.clauses[0].applies_to, ["operation.test"]);
        assert_eq!(report.clauses[1].applies_to, ["operation.unrelated"]);
    }

    #[test]
    fn current_success_resolves_an_earlier_current_failure() {
        let mut project = project_with_evidence("failed", "unsatisfied");
        let mut retry_evidence = project["evidence"][0].clone();
        retry_evidence["id"] = json!("evidence.retry");
        retry_evidence["outcome"] = json!("passed");
        let retry_evidence_digest = digest_value(&retry_evidence).unwrap();

        let mut retry_result = project["results"][0].clone();
        retry_result["id"] = json!("result.retry");
        retry_result["output_refs"] = json!(["evidence.retry"]);
        retry_result["payload"]["outcomes"][0]["status"] = json!("satisfied");
        retry_result["payload"]["outcomes"][0]["basis_refs"] = json!(["evidence.retry"]);
        replace_evidence_digest(
            &mut retry_result["input_refs"],
            "evidence.retry",
            &retry_evidence_digest,
        );
        replace_evidence_digest(
            &mut retry_result["freshness_refs"],
            "evidence.retry",
            &retry_evidence_digest,
        );
        replace_evidence_digest(
            &mut retry_result["payload"]["outcomes"][0]["input_refs"],
            "evidence.retry",
            &retry_evidence_digest,
        );
        replace_evidence_digest(
            &mut retry_result["payload"]["outcomes"][0]["freshness_refs"],
            "evidence.retry",
            &retry_evidence_digest,
        );
        project["evidence"]
            .as_array_mut()
            .unwrap()
            .push(retry_evidence);
        project["results"]
            .as_array_mut()
            .unwrap()
            .push(retry_result);

        let report = build_contract_health_report(&project, &registry()).unwrap();
        assert_eq!(report.clauses[0].status, "verified");
        assert_eq!(
            report.clauses[0].verification_result_ids,
            ["result.retry", "result.test"]
        );
    }
}
