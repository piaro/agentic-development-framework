//! Normalize authoritative Project records into one immutable Snapshot.
//!
//! Storage layout and file order belong to Project Adapters. Downstream modules
//! receive sorted records and content digests through this language-neutral type.

use crate::canonical_digest;
use crate::kernel::ProjectSnapshot;
use crate::schema::SchemaRegistry;
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;
use std::fmt;

pub const PROJECT_SNAPSHOT_PROTOCOL_VERSION: &str = "1";

pub fn build_project_snapshot(
    project: &Value,
    change_id: &str,
    schema_registry: &SchemaRegistry,
) -> Result<ProjectSnapshot, ProjectSnapshotError> {
    let project = project
        .as_object()
        .ok_or_else(|| ProjectSnapshotError::new("project must be an object"))?;

    for (collection, record_kind) in [
        ("changes", "change"),
        ("contracts", "contract"),
        ("decisions", "decision"),
        ("results", "result"),
        ("evidence", "evidence"),
    ] {
        for item in record_array(project, collection)? {
            schema_registry
                .validate(record_kind, item)
                .map_err(|error| {
                    ProjectSnapshotError::new(format!("invalid {record_kind} record: {error}"))
                })?;
        }
    }

    let changes: BTreeMap<String, Value> = record_array(project, "changes")?
        .iter()
        .map(|item| {
            (
                item["id"]
                    .as_str()
                    .expect("Change Schema guarantees a string ID")
                    .to_owned(),
                item.clone(),
            )
        })
        .collect();
    let change = changes
        .get(change_id)
        .cloned()
        .ok_or_else(|| ProjectSnapshotError::new(format!("unknown change: {change_id}")))?;
    let contracts = records_for_change(project, "contracts", change_id)?;
    let decisions = decision_records_for_change(project, change_id)?;
    let results = records_for_change(project, "results", change_id)?;
    let evidence = records_for_change(project, "evidence", change_id)?;
    let repository = project
        .get("repository")
        .cloned()
        .unwrap_or_else(|| json!({}));
    validate_binding_authorities(&repository, &decisions)?;

    let mut artifact_digests = BTreeMap::new();
    artifact_digests.insert(change_id.to_owned(), digest_value(&change)?);
    for contract in &contracts {
        let contract_id = required_id(contract, "Contract")?;
        artifact_digests.insert(contract_id.to_owned(), digest_value(contract)?);
        for clause in value_array(contract, "clauses", "Contract clauses")? {
            let clause_id = clause["id"]
                .as_str()
                .expect("Contract Schema guarantees a string clause ID");
            artifact_digests.insert(format!("{contract_id}#{clause_id}"), digest_value(clause)?);
        }
    }
    for collection in [&decisions, &results, &evidence] {
        for item in collection {
            let item_id = required_id(item, "Project record")?;
            artifact_digests.insert(item_id.to_owned(), digest_value(item)?);
        }
    }
    for artifact in repository
        .get("artifacts")
        .map(|value| value_array_value(value, "Repository artifacts"))
        .transpose()?
        .unwrap_or(&[])
    {
        let reference = artifact["ref"]
            .as_str()
            .ok_or_else(|| ProjectSnapshotError::new("Repository artifact ref must be a string"))?;
        let digest = artifact["digest"]
            .as_str()
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
            .unwrap_or(digest_value(artifact)?);
        artifact_digests.insert(reference.to_owned(), digest);
    }

    let snapshot_body = json!({
        "change": change,
        "contracts": contracts,
        "decisions": decisions,
        "results": results,
        "evidence": evidence,
        "repository": repository,
        "artifact_digests": artifact_digests,
    });
    let digest = digest_value(&snapshot_body)?;
    Ok(ProjectSnapshot {
        change_id: change_id.to_owned(),
        change: snapshot_body["change"].clone(),
        contracts: array_clone(&snapshot_body["contracts"]),
        decisions: array_clone(&snapshot_body["decisions"]),
        results: array_clone(&snapshot_body["results"]),
        evidence: array_clone(&snapshot_body["evidence"]),
        repository: snapshot_body["repository"].clone(),
        artifact_digests: object_string_map(&snapshot_body["artifact_digests"]),
        digest,
    })
}

pub fn compatibility_checkpoint(snapshot: &ProjectSnapshot) -> Value {
    json!({
        "change_id": snapshot.change_id,
        "contract_ids": record_ids(&snapshot.contracts),
        "decision_ids": record_ids(&snapshot.decisions),
        "result_ids": record_ids(&snapshot.results),
        "evidence_ids": record_ids(&snapshot.evidence),
        "repository_phase": snapshot.repository["phase"]
            .as_str()
            .unwrap_or("pre-build"),
        "artifact_digests": snapshot.artifact_digests,
        "digest": snapshot.digest,
    })
}

fn records_for_change(
    project: &Map<String, Value>,
    collection: &str,
    change_id: &str,
) -> Result<Vec<Value>, ProjectSnapshotError> {
    let mut records: Vec<Value> = record_array(project, collection)?
        .iter()
        .filter(|item| {
            item.get("change_id")
                .and_then(Value::as_str)
                .is_none_or(|candidate| candidate == change_id)
        })
        .cloned()
        .collect();
    records.sort_by(|left, right| left["id"].as_str().cmp(&right["id"].as_str()));
    Ok(records)
}

fn decision_records_for_change(
    project: &Map<String, Value>,
    change_id: &str,
) -> Result<Vec<Value>, ProjectSnapshotError> {
    let mut records: Vec<Value> = record_array(project, "decisions")?
        .iter()
        .filter(|decision| {
            decision["status"].as_str() == Some("accepted")
                || decision
                    .get("change_id")
                    .and_then(Value::as_str)
                    .is_none_or(|candidate| candidate == change_id)
        })
        .cloned()
        .collect();
    records.sort_by(|left, right| left["id"].as_str().cmp(&right["id"].as_str()));
    Ok(records)
}

fn validate_binding_authorities(
    repository: &Value,
    decisions: &[Value],
) -> Result<(), ProjectSnapshotError> {
    let accepted_decisions: std::collections::BTreeSet<&str> = decisions
        .iter()
        .filter(|decision| decision["status"].as_str() == Some("accepted"))
        .filter_map(|decision| decision["id"].as_str())
        .collect();
    for (fact_index, fact) in repository
        .get("facts")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .enumerate()
    {
        let Some(authority_refs) = fact.get("binding_authority_refs") else {
            continue;
        };
        let authority_refs = authority_refs.as_array().ok_or_else(|| {
            ProjectSnapshotError::new(format!(
                "repository fact {fact_index} binding_authority_refs must be an array"
            ))
        })?;
        for authority_ref in authority_refs {
            let authority_ref = authority_ref.as_str().ok_or_else(|| {
                ProjectSnapshotError::new(format!(
                    "repository fact {fact_index} binding authority must be a string"
                ))
            })?;
            if !accepted_decisions.contains(authority_ref) {
                return Err(ProjectSnapshotError::new(format!(
                    "repository fact {fact_index} binding authority is not an accepted Decision: {authority_ref}"
                )));
            }
        }
    }
    Ok(())
}

fn record_array<'a>(
    project: &'a Map<String, Value>,
    collection: &str,
) -> Result<&'a [Value], ProjectSnapshotError> {
    match project.get(collection) {
        None => Ok(&[]),
        Some(value) => value_array_value(value, collection),
    }
}

fn value_array<'a>(
    value: &'a Value,
    field: &str,
    label: &str,
) -> Result<&'a [Value], ProjectSnapshotError> {
    value
        .get(field)
        .map(|child| value_array_value(child, label))
        .transpose()
        .map(|value| value.unwrap_or(&[]))
}

fn value_array_value<'a>(
    value: &'a Value,
    label: &str,
) -> Result<&'a [Value], ProjectSnapshotError> {
    value
        .as_array()
        .map(Vec::as_slice)
        .ok_or_else(|| ProjectSnapshotError::new(format!("{label} must be an array")))
}

fn required_id<'a>(value: &'a Value, label: &str) -> Result<&'a str, ProjectSnapshotError> {
    value["id"]
        .as_str()
        .ok_or_else(|| ProjectSnapshotError::new(format!("{label} ID must be a string")))
}

fn digest_value(value: &Value) -> Result<String, ProjectSnapshotError> {
    canonical_digest(value).map_err(|error| ProjectSnapshotError::new(error.to_string()))
}

fn array_clone(value: &Value) -> Vec<Value> {
    value
        .as_array()
        .expect("Snapshot body field is an array")
        .clone()
}

fn object_string_map(value: &Value) -> BTreeMap<String, String> {
    value
        .as_object()
        .expect("Snapshot artifact digests are an object")
        .iter()
        .map(|(key, value)| {
            (
                key.clone(),
                value
                    .as_str()
                    .expect("Snapshot artifact digest is a string")
                    .to_owned(),
            )
        })
        .collect()
}

fn record_ids(records: &[Value]) -> Vec<&str> {
    records
        .iter()
        .filter_map(|record| record["id"].as_str())
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectSnapshotError {
    message: String,
}

impl ProjectSnapshotError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for ProjectSnapshotError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ProjectSnapshotError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::SchemaRegistry;
    use std::path::PathBuf;

    #[test]
    fn rejects_unknown_change() {
        let registry =
            SchemaRegistry::load(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("schemas/v1"))
                .unwrap();
        let error = build_project_snapshot(&json!({"changes": []}), "change.missing", &registry)
            .unwrap_err();
        assert_eq!(error.to_string(), "unknown change: change.missing");
    }

    #[test]
    fn binding_authority_must_be_an_accepted_decision() {
        let repository = json!({
            "facts": [{
                "binding_authority_refs": ["decision.binding"]
            }]
        });
        let error = validate_binding_authorities(
            &repository,
            &[json!({
                "id": "decision.binding",
                "status": "proposed"
            })],
        )
        .unwrap_err();

        assert_eq!(
            error.to_string(),
            "repository fact 0 binding authority is not an accepted Decision: decision.binding"
        );
    }

    #[test]
    fn accepted_decision_from_an_earlier_change_remains_available() {
        let registry =
            SchemaRegistry::load(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("schemas/v1"))
                .unwrap();
        let snapshot = build_project_snapshot(
            &json!({
                "changes": [{
                    "schema_version": "1",
                    "id": "change.current",
                    "title": "Current change",
                    "intent": "Use an existing repository binding"
                }],
                "decisions": [{
                    "schema_version": "1",
                    "id": "decision.binding",
                    "change_id": "change.previous",
                    "status": "accepted",
                    "title": "Bind the repository fact",
                    "resolves": ["contract.shared#binding"]
                }],
                "repository": {
                    "facts": [{
                        "binding_authority_refs": ["decision.binding"]
                    }]
                }
            }),
            "change.current",
            &registry,
        )
        .unwrap();

        assert_eq!(snapshot.decisions.len(), 1);
        assert_eq!(snapshot.decisions[0]["id"], "decision.binding");
        assert!(snapshot.artifact_digests.contains_key("decision.binding"));
    }

    #[test]
    fn non_accepted_decisions_from_other_changes_remain_out_of_scope() {
        let registry =
            SchemaRegistry::load(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("schemas/v1"))
                .unwrap();
        let snapshot = build_project_snapshot(
            &json!({
                "changes": [{
                    "schema_version": "1",
                    "id": "change.current",
                    "title": "Current change",
                    "intent": "Inspect decision scope"
                }],
                "decisions": [
                    {
                        "schema_version": "1",
                        "id": "decision.accepted-earlier",
                        "change_id": "change.previous",
                        "status": "accepted",
                        "title": "Accepted earlier decision",
                        "resolves": ["contract.shared#accepted"]
                    },
                    {
                        "schema_version": "1",
                        "id": "decision.proposed-earlier",
                        "change_id": "change.previous",
                        "status": "proposed",
                        "title": "Unresolved earlier proposal",
                        "resolves": ["contract.shared#proposed"]
                    },
                    {
                        "schema_version": "1",
                        "id": "decision.proposed-current",
                        "change_id": "change.current",
                        "status": "proposed",
                        "title": "Current proposal",
                        "resolves": ["contract.current#proposed"]
                    }
                ]
            }),
            "change.current",
            &registry,
        )
        .unwrap();

        let decision_ids = snapshot
            .decisions
            .iter()
            .filter_map(|decision| decision["id"].as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            decision_ids,
            ["decision.accepted-earlier", "decision.proposed-current"]
        );
    }
}
