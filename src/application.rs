//! Storage-independent orchestration of the technical modules.
//!
//! The Application owns call order and issued Context lookup. Semantic state
//! remains a Thin Kernel output, while physical persistence remains a Project
//! Store responsibility.

use crate::canonical_digest;
use crate::context::{ContextCompiler, GeneratedContext};
use crate::contract_health::{ContractHealthReport, build_contract_health_report};
use crate::detection::detect_typed_facts_with_registry;
use crate::explain::{ExplainReport, ExplanationBuilder};
use crate::framework_lock::{FrameworkLock, validate_framework_lock};
use crate::kernel::{KernelDecision, ProjectSnapshot, ThinKernel};
use crate::project::build_project_snapshot;
use crate::rules::{RuleIndex, compile_rule_index_with_registry};
use crate::schema::SchemaRegistry;
use crate::signal_catalog::SignalCatalogRegistry;
use crate::submission::{ResultSubmission, prepare_result};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub struct ApplicationResponse {
    pub decision: KernelDecision,
    pub context: Option<GeneratedContext>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ApplicationSubmission {
    pub result: Value,
    pub response: ApplicationResponse,
}

/// Persistence operations required by `next` and `submit`.
///
/// Implementations own physical layout and concurrency. The Application sees
/// only validated Snapshots and append/upsert operations.
pub trait ProjectStore {
    fn snapshot(&self, change_id: &str) -> Result<ProjectSnapshot, ProjectStoreError>;
    fn contract_health(&self) -> Result<ContractHealthReport, ProjectStoreError>;
    fn append_result(&mut self, result: &Value) -> Result<(), ProjectStoreError>;
    fn add_evidence(&mut self, evidence: &Value) -> Result<(), ProjectStoreError>;
    fn upsert_decision(
        &mut self,
        decision: &Value,
        expected_digest: Option<&str>,
    ) -> Result<(), ProjectStoreError>;
    fn upsert_contract(
        &mut self,
        contract: &Value,
        expected_digest: Option<&str>,
    ) -> Result<(), ProjectStoreError>;
    fn upsert_contract_clauses(
        &mut self,
        contract: &Value,
        expected_clause_digests: &BTreeMap<String, String>,
    ) -> Result<(), ProjectStoreError>;
    fn update_repository(&mut self, repository: Value) -> Result<(), ProjectStoreError>;
}

pub struct Application<'a, Store: ProjectStore> {
    store: Store,
    rule_index: RuleIndex,
    framework_lock: FrameworkLock,
    schema_registry: &'a SchemaRegistry,
    signal_registry: SignalCatalogRegistry,
    issued: BTreeMap<(String, String), GeneratedContext>,
}

impl<'a, Store: ProjectStore> Application<'a, Store> {
    pub fn with_store(
        store: Store,
        rule_source: &Value,
        framework_lock_source: &Value,
        schema_registry: &'a SchemaRegistry,
    ) -> Result<Self, ApplicationError> {
        let signal_registry = SignalCatalogRegistry::built_in()
            .map_err(|error| application_error(error.to_string()))?;
        Self::with_store_and_signal_registry(
            store,
            rule_source,
            framework_lock_source,
            schema_registry,
            signal_registry,
        )
    }

    pub fn with_store_and_signal_registry(
        store: Store,
        rule_source: &Value,
        framework_lock_source: &Value,
        schema_registry: &'a SchemaRegistry,
        signal_registry: SignalCatalogRegistry,
    ) -> Result<Self, ApplicationError> {
        let rule_index =
            compile_rule_index_with_registry(rule_source, schema_registry, &signal_registry)
                .map_err(|error| application_error(error.to_string()))?;
        // A partial Framework upgrade must stop before any Project evaluation.
        let framework_lock = validate_framework_lock(
            framework_lock_source,
            rule_source,
            &rule_index,
            schema_registry,
        )
        .map_err(|error| application_error(error.to_string()))?;
        Ok(Self {
            store,
            rule_index,
            framework_lock,
            schema_registry,
            signal_registry,
            issued: BTreeMap::new(),
        })
    }

    pub fn next(&mut self, change_id: &str) -> Result<ApplicationResponse, ApplicationError> {
        // State is deliberately recomputed instead of restored from process memory.
        let snapshot = self.snapshot(change_id)?;
        let detection = detect_typed_facts_with_registry(
            change_id,
            snapshot.repository["facts"]
                .as_array()
                .map(Vec::as_slice)
                .unwrap_or(&[]),
            &snapshot.repository["coverage"],
            &snapshot.artifact_digests,
            &self.signal_registry,
        )
        .map_err(|error| application_error(error.to_string()))?;
        let contract_health = self
            .store
            .contract_health()
            .map_err(|error| application_error(error.to_string()))?;
        let decision = ThinKernel.evaluate_with_health(
            &snapshot,
            &self.rule_index,
            &detection,
            Some(&contract_health),
        );
        let context = ContextCompiler.compile_with_health(
            &decision,
            &snapshot,
            &detection,
            Some(&contract_health),
        );
        if let Some(context) = &context {
            self.issued.insert(
                (context.action_id.clone(), context.digest.clone()),
                context.clone(),
            );
        }
        Ok(ApplicationResponse { decision, context })
    }

    pub fn submit(
        &mut self,
        submission: &ResultSubmission,
    ) -> Result<ApplicationResponse, ApplicationError> {
        let context = self
            .issued
            .get(&(
                submission.action_id.clone(),
                submission.context_digest.clone(),
            ))
            .cloned()
            .ok_or_else(|| {
                application_error(format!(
                    "unknown action or Context: {} {}",
                    submission.action_id, submission.context_digest
                ))
            })?;
        self.submit_issued(&context, submission)
            .map(|submission| submission.response)
    }

    /// Submit against a Context retained by a trusted outer Application session.
    ///
    /// Long-running adapters reload the current Project for each call so code and
    /// Records changed after `next` are observed. They retain the issued Context
    /// separately and pass it back through this Application-owned validation path.
    pub fn submit_issued(
        &mut self,
        context: &GeneratedContext,
        submission: &ResultSubmission,
    ) -> Result<ApplicationSubmission, ApplicationError> {
        let snapshot = self.snapshot(&submission.change_id)?;
        self.submit_issued_with_snapshot(context, submission, &snapshot)
    }

    /// Submit against a Snapshot that the caller already loaded for validation.
    ///
    /// The post-submit evaluation still loads a new Snapshot so the response
    /// includes the Result that was just persisted.
    pub(crate) fn submit_issued_with_snapshot(
        &mut self,
        context: &GeneratedContext,
        submission: &ResultSubmission,
        snapshot: &ProjectSnapshot,
    ) -> Result<ApplicationSubmission, ApplicationError> {
        let result = prepare_result(context, snapshot, submission, self.schema_registry)
            .map_err(|error| application_error(error.to_string()))?;
        // The append completes before consuming the issued Context. A failed
        // persistence attempt can therefore be retried without recreating state.
        self.store
            .append_result(&result)
            .map_err(|error| application_error(error.to_string()))?;
        self.issued.remove(&(
            submission.action_id.clone(),
            submission.context_digest.clone(),
        ));
        let response = self.next(&submission.change_id)?;
        Ok(ApplicationSubmission { result, response })
    }

    /// Recompute the current decision and its trace without issuing an Action.
    pub fn explain(&self, change_id: &str) -> Result<ExplainReport, ApplicationError> {
        let snapshot = self.snapshot(change_id)?;
        let detection = detect_typed_facts_with_registry(
            change_id,
            snapshot.repository["facts"]
                .as_array()
                .map(Vec::as_slice)
                .unwrap_or(&[]),
            &snapshot.repository["coverage"],
            &snapshot.artifact_digests,
            &self.signal_registry,
        )
        .map_err(|error| application_error(error.to_string()))?;
        let contract_health = self
            .store
            .contract_health()
            .map_err(|error| application_error(error.to_string()))?;
        let decision = ThinKernel.evaluate_with_health(
            &snapshot,
            &self.rule_index,
            &detection,
            Some(&contract_health),
        );
        Ok(ExplanationBuilder.build(&snapshot, &self.rule_index, &detection, &decision))
    }

    pub fn snapshot(&self, change_id: &str) -> Result<ProjectSnapshot, ApplicationError> {
        self.store
            .snapshot(change_id)
            .map_err(|error| application_error(error.to_string()))
    }

    pub fn contract_health(&self) -> Result<ContractHealthReport, ApplicationError> {
        self.store
            .contract_health()
            .map_err(|error| application_error(error.to_string()))
    }

    pub fn upsert_decision(
        &mut self,
        decision: Value,
        expected_digest: Option<&str>,
    ) -> Result<(), ApplicationError> {
        self.store
            .upsert_decision(&decision, expected_digest)
            .map_err(|error| application_error(error.to_string()))
    }

    pub fn add_evidence(&mut self, evidence: Value) -> Result<(), ApplicationError> {
        self.store
            .add_evidence(&evidence)
            .map_err(|error| application_error(error.to_string()))
    }

    pub fn upsert_contract(
        &mut self,
        contract: Value,
        expected_digest: Option<&str>,
    ) -> Result<(), ApplicationError> {
        self.store
            .upsert_contract(&contract, expected_digest)
            .map_err(|error| application_error(error.to_string()))
    }

    pub fn upsert_contract_clauses(
        &mut self,
        contract: Value,
        expected_clause_digests: &BTreeMap<String, String>,
    ) -> Result<(), ApplicationError> {
        self.store
            .upsert_contract_clauses(&contract, expected_clause_digests)
            .map_err(|error| application_error(error.to_string()))
    }

    pub fn update_repository(&mut self, repository: Value) -> Result<(), ApplicationError> {
        self.store
            .update_repository(repository)
            .map_err(|error| application_error(error.to_string()))
    }

    pub fn rule_index_digest(&self) -> &str {
        &self.rule_index.digest
    }

    pub fn framework_lock_digest(&self) -> &str {
        &self.framework_lock.digest
    }

    pub fn signal_catalog_digest(&self) -> &str {
        self.signal_registry.digest()
    }
}

/// Test adapter that keeps authoritative Records in one JSON value.
pub struct InMemoryProjectStore<'a> {
    project: Value,
    schema_registry: &'a SchemaRegistry,
}

impl<'a> InMemoryProjectStore<'a> {
    pub fn new(project: Value, schema_registry: &'a SchemaRegistry) -> Self {
        Self {
            project,
            schema_registry,
        }
    }

    fn upsert_record(
        &mut self,
        collection: &str,
        record_kind: &str,
        record: &Value,
    ) -> Result<(), ProjectStoreError> {
        self.schema_registry
            .validate(record_kind, record)
            .map_err(|error| project_store_error(error.to_string()))?;
        let record_id = record["id"]
            .as_str()
            .ok_or_else(|| project_store_error("Record ID must be a string"))?;
        let records = self.record_collection_mut(collection)?;
        if let Some(existing) = records
            .iter_mut()
            .find(|candidate| candidate["id"].as_str() == Some(record_id))
        {
            *existing = record.clone();
        } else {
            records.push(record.clone());
        }
        Ok(())
    }

    fn record_collection_mut(
        &mut self,
        collection: &str,
    ) -> Result<&mut Vec<Value>, ProjectStoreError> {
        let project = self
            .project
            .as_object_mut()
            .ok_or_else(|| project_store_error("Project must be an object"))?;
        let records = project
            .entry(collection)
            .or_insert_with(|| Value::Array(Vec::new()));
        records
            .as_array_mut()
            .ok_or_else(|| project_store_error(format!("{collection} must be an array")))
    }
}

impl ProjectStore for InMemoryProjectStore<'_> {
    fn snapshot(&self, change_id: &str) -> Result<ProjectSnapshot, ProjectStoreError> {
        build_project_snapshot(&self.project, change_id, self.schema_registry)
            .map_err(|error| project_store_error(error.to_string()))
    }

    fn contract_health(&self) -> Result<ContractHealthReport, ProjectStoreError> {
        build_contract_health_report(&self.project, self.schema_registry)
            .map_err(|error| project_store_error(error.to_string()))
    }

    fn append_result(&mut self, result: &Value) -> Result<(), ProjectStoreError> {
        self.schema_registry
            .validate("result", result)
            .map_err(|error| project_store_error(error.to_string()))?;
        let result_id = result["id"]
            .as_str()
            .ok_or_else(|| project_store_error("Result ID must be a string"))?;
        let results = self.record_collection_mut("results")?;
        if results
            .iter()
            .any(|candidate| candidate["id"].as_str() == Some(result_id))
        {
            return Err(project_store_error(format!(
                "duplicate result: {result_id}"
            )));
        }
        results.push(result.clone());
        Ok(())
    }

    fn add_evidence(&mut self, evidence: &Value) -> Result<(), ProjectStoreError> {
        self.schema_registry
            .validate("evidence", evidence)
            .map_err(|error| project_store_error(error.to_string()))?;
        let evidence_id = evidence["id"]
            .as_str()
            .ok_or_else(|| project_store_error("Evidence ID must be a string"))?;
        let records = self.record_collection_mut("evidence")?;
        if records
            .iter()
            .any(|candidate| candidate["id"].as_str() == Some(evidence_id))
        {
            return Err(project_store_error(format!(
                "duplicate evidence: {evidence_id}"
            )));
        }
        records.push(evidence.clone());
        Ok(())
    }

    fn upsert_decision(
        &mut self,
        decision: &Value,
        expected_digest: Option<&str>,
    ) -> Result<(), ProjectStoreError> {
        let existing = self
            .project
            .get("decisions")
            .and_then(Value::as_array)
            .and_then(|decisions| {
                decisions
                    .iter()
                    .find(|candidate| candidate["id"] == decision["id"])
            });
        validate_record_update("Decision", existing, decision, expected_digest)?;
        self.upsert_record("decisions", "decision", decision)
    }

    fn upsert_contract(
        &mut self,
        contract: &Value,
        expected_digest: Option<&str>,
    ) -> Result<(), ProjectStoreError> {
        let existing = self
            .project
            .get("contracts")
            .and_then(Value::as_array)
            .and_then(|contracts| {
                contracts
                    .iter()
                    .find(|candidate| candidate["id"] == contract["id"])
            });
        validate_contract_update(existing, contract, expected_digest)?;
        self.upsert_record("contracts", "contract", contract)
    }

    fn upsert_contract_clauses(
        &mut self,
        contract: &Value,
        expected_clause_digests: &BTreeMap<String, String>,
    ) -> Result<(), ProjectStoreError> {
        self.schema_registry
            .validate("contract", contract)
            .map_err(|error| project_store_error(error.to_string()))?;
        let existing = self
            .project
            .get("contracts")
            .and_then(Value::as_array)
            .and_then(|contracts| {
                contracts
                    .iter()
                    .find(|candidate| candidate["id"] == contract["id"])
            });
        let merged = merge_contract_clause_update(existing, contract, expected_clause_digests)?;
        self.upsert_record("contracts", "contract", &merged)
    }

    fn update_repository(&mut self, repository: Value) -> Result<(), ProjectStoreError> {
        let project = self
            .project
            .as_object_mut()
            .ok_or_else(|| project_store_error("Project must be an object"))?;
        project.insert("repository".to_owned(), repository);
        Ok(())
    }
}

pub(crate) fn validate_contract_update(
    existing: Option<&Value>,
    proposed: &Value,
    expected_digest: Option<&str>,
) -> Result<(), ProjectStoreError> {
    let contract_id = proposed["id"]
        .as_str()
        .ok_or_else(|| project_store_error("Contract ID must be a string"))?;
    let Some(existing) = existing else {
        if let Some(expected) = expected_digest {
            return Err(project_store_error(format!(
                "stale Contract update: {contract_id}: expected {expected}, current record is missing"
            )));
        }
        return Ok(());
    };

    let shared_update = existing.get("change_id").is_none() || proposed.get("change_id").is_none();
    let Some(expected) = expected_digest else {
        if shared_update {
            return Err(project_store_error(format!(
                "Shared Contract update requires expected digest: {contract_id}"
            )));
        }
        return Ok(());
    };

    let current =
        canonical_digest(existing).map_err(|error| project_store_error(error.to_string()))?;
    if expected != current {
        return Err(project_store_error(format!(
            "stale Contract update: {contract_id}: expected {expected}, current {current}"
        )));
    }
    Ok(())
}

/// Mechanically merges a clause-scoped Contract update.
///
/// `proposed.clauses` is treated as a clause patch. Existing clauses included
/// in the patch require their base digest; an expected clause omitted from the
/// patch is deleted. Clauses outside the patch are preserved, so concurrent
/// updates to them do not conflict. An update to the same clause is rejected
/// as stale. Contract metadata requires the existing whole-record digest path
/// and is never merged here.
pub(crate) fn merge_contract_clause_update(
    existing: Option<&Value>,
    proposed: &Value,
    expected_clause_digests: &BTreeMap<String, String>,
) -> Result<Value, ProjectStoreError> {
    let contract_id = proposed["id"]
        .as_str()
        .ok_or_else(|| project_store_error("Contract ID must be a string"))?;
    if expected_clause_digests.is_empty() {
        return Err(project_store_error(format!(
            "clause-scoped Contract update requires expected clause digests: {contract_id}"
        )));
    }
    let existing = existing.ok_or_else(|| {
        project_store_error(format!(
            "stale Contract clause update: {contract_id}: current record is missing"
        ))
    })?;

    let existing_object = existing
        .as_object()
        .ok_or_else(|| project_store_error("Contract must be an object"))?;
    let proposed_object = proposed
        .as_object()
        .ok_or_else(|| project_store_error("Contract must be an object"))?;
    let mut existing_metadata = existing_object.clone();
    let mut proposed_metadata = proposed_object.clone();
    existing_metadata.remove("clauses");
    proposed_metadata.remove("clauses");
    if existing_metadata != proposed_metadata {
        return Err(project_store_error(format!(
            "clause-scoped Contract update cannot change metadata: {contract_id}"
        )));
    }

    let current_clauses = contract_clause_map(existing, contract_id)?;
    let proposed_clauses = contract_clause_map(proposed, contract_id)?;
    for clause_id in expected_clause_digests.keys() {
        if !current_clauses.contains_key(clause_id.as_str()) {
            return Err(project_store_error(format!(
                "stale Contract clause update: {contract_id}#{clause_id}: current clause is missing"
            )));
        }
    }

    let mut merged_clauses = Vec::new();
    for current_clause in existing["clauses"]
        .as_array()
        .expect("contract_clause_map validated clauses")
    {
        let clause_id = current_clause["id"]
            .as_str()
            .expect("contract_clause_map validated clause IDs");
        let Some(proposed_clause) = proposed_clauses.get(clause_id).copied() else {
            if let Some(expected) = expected_clause_digests.get(clause_id) {
                assert_current_clause_digest(contract_id, clause_id, current_clause, expected)?;
                continue;
            }
            merged_clauses.push(current_clause.clone());
            continue;
        };
        let proposed_digest = canonical_digest(proposed_clause)
            .map_err(|error| project_store_error(error.to_string()))?;
        let Some(expected) = expected_clause_digests.get(clause_id) else {
            return Err(project_store_error(format!(
                "Contract clause update requires expected digest: {contract_id}#{clause_id}"
            )));
        };

        if proposed_digest == *expected {
            // The caller did not edit this clause; preserve a concurrent update.
            merged_clauses.push(current_clause.clone());
        } else {
            assert_current_clause_digest(contract_id, clause_id, current_clause, expected)?;
            merged_clauses.push(proposed_clause.clone());
        }
    }

    for proposed_clause in proposed["clauses"]
        .as_array()
        .expect("contract_clause_map validated clauses")
    {
        let clause_id = proposed_clause["id"]
            .as_str()
            .expect("contract_clause_map validated clause IDs");
        if !current_clauses.contains_key(clause_id) {
            merged_clauses.push(proposed_clause.clone());
        }
    }

    let mut merged = existing.clone();
    merged["clauses"] = Value::Array(merged_clauses);
    Ok(merged)
}

fn contract_clause_map<'a>(
    contract: &'a Value,
    contract_id: &str,
) -> Result<BTreeMap<&'a str, &'a Value>, ProjectStoreError> {
    let clauses = contract["clauses"]
        .as_array()
        .ok_or_else(|| project_store_error("Contract clauses must be an array"))?;
    let mut by_id = BTreeMap::new();
    for clause in clauses {
        let clause_id = clause["id"]
            .as_str()
            .ok_or_else(|| project_store_error("Contract clause ID must be a string"))?;
        if by_id.insert(clause_id, clause).is_some() {
            return Err(project_store_error(format!(
                "duplicate Contract clause ID: {contract_id}#{clause_id}"
            )));
        }
    }
    Ok(by_id)
}

fn assert_current_clause_digest(
    contract_id: &str,
    clause_id: &str,
    current_clause: &Value,
    expected: &str,
) -> Result<(), ProjectStoreError> {
    let current =
        canonical_digest(current_clause).map_err(|error| project_store_error(error.to_string()))?;
    if current != expected {
        return Err(project_store_error(format!(
            "stale Contract clause update: {contract_id}#{clause_id}: expected {expected}, current {current}"
        )));
    }
    Ok(())
}

pub(crate) fn validate_decision_update(
    existing: Option<&Value>,
    proposed: &Value,
    expected_digest: Option<&str>,
) -> Result<(), ProjectStoreError> {
    validate_record_update("Decision", existing, proposed, expected_digest)
}

fn validate_record_update(
    label: &str,
    existing: Option<&Value>,
    proposed: &Value,
    expected_digest: Option<&str>,
) -> Result<(), ProjectStoreError> {
    let record_id = proposed["id"]
        .as_str()
        .ok_or_else(|| project_store_error(format!("{label} ID must be a string")))?;
    let Some(existing) = existing else {
        if let Some(expected) = expected_digest {
            return Err(project_store_error(format!(
                "stale {label} update: {record_id}: expected {expected}, current record is missing"
            )));
        }
        return Ok(());
    };
    let Some(expected) = expected_digest else {
        return Err(project_store_error(format!(
            "{label} update requires expected digest: {record_id}"
        )));
    };
    let current =
        canonical_digest(existing).map_err(|error| project_store_error(error.to_string()))?;
    if expected != current {
        return Err(project_store_error(format!(
            "stale {label} update: {record_id}: expected {expected}, current {current}"
        )));
    }
    Ok(())
}

pub type InMemoryApplication<'a> = Application<'a, InMemoryProjectStore<'a>>;

impl<'a> Application<'a, InMemoryProjectStore<'a>> {
    pub fn new(
        project: Value,
        rule_source: &Value,
        framework_lock_source: &Value,
        schema_registry: &'a SchemaRegistry,
    ) -> Result<Self, ApplicationError> {
        Self::with_store(
            InMemoryProjectStore::new(project, schema_registry),
            rule_source,
            framework_lock_source,
            schema_registry,
        )
    }
}

fn application_error(message: impl Into<String>) -> ApplicationError {
    ApplicationError {
        message: message.into(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplicationError {
    message: String,
}

impl fmt::Display for ApplicationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ApplicationError {}

impl ApplicationError {
    pub fn new(message: impl Into<String>) -> Self {
        application_error(message)
    }
}

fn project_store_error(message: impl Into<String>) -> ProjectStoreError {
    ProjectStoreError {
        message: message.into(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectStoreError {
    message: String,
}

impl ProjectStoreError {
    pub fn new(message: impl Into<String>) -> Self {
        project_store_error(message)
    }
}

impl fmt::Display for ProjectStoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ProjectStoreError {}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn shared_contract_update_requires_the_current_digest() {
        let initial = json!({
            "schema_version": "1",
            "id": "contract.shared-concurrency",
            "applies_to": ["data.orders"],
            "clauses": [{
                "id": "concurrency-policy",
                "text": "initial"
            }]
        });
        let update = json!({
            "schema_version": "1",
            "id": "contract.shared-concurrency",
            "applies_to": ["data.orders"],
            "clauses": [{
                "id": "concurrency-policy",
                "text": "updated"
            }]
        });
        let initial_digest = canonical_digest(&initial).unwrap();

        let missing = validate_contract_update(Some(&initial), &update, None).unwrap_err();
        assert!(
            missing
                .to_string()
                .contains("Shared Contract update requires expected digest")
        );
        validate_contract_update(Some(&initial), &update, Some(&initial_digest)).unwrap();
        let stale =
            validate_contract_update(Some(&update), &initial, Some(&initial_digest)).unwrap_err();
        assert!(stale.to_string().contains("stale Contract update"));
    }

    #[test]
    fn clause_scoped_updates_merge_different_clauses_and_reject_same_clause() {
        let initial = json!({
            "schema_version": "1",
            "id": "contract.shared-concurrency",
            "applies_to": ["data.orders"],
            "clauses": [
                {"id": "place-order", "text": "place initial"},
                {"id": "retry-order", "text": "retry initial"}
            ]
        });
        let place_digest = canonical_digest(&initial["clauses"][0]).unwrap();
        let retry_digest = canonical_digest(&initial["clauses"][1]).unwrap();
        let place_expected = BTreeMap::from([("place-order".to_owned(), place_digest)]);
        let retry_expected = BTreeMap::from([("retry-order".to_owned(), retry_digest)]);
        let place_update = json!({
            "schema_version": "1",
            "id": "contract.shared-concurrency",
            "applies_to": ["data.orders"],
            "clauses": [{"id": "place-order", "text": "place updated"}]
        });
        let retry_update = json!({
            "schema_version": "1",
            "id": "contract.shared-concurrency",
            "applies_to": ["data.orders"],
            "clauses": [{"id": "retry-order", "text": "retry updated"}]
        });

        let after_place =
            merge_contract_clause_update(Some(&initial), &place_update, &place_expected).unwrap();
        let after_both =
            merge_contract_clause_update(Some(&after_place), &retry_update, &retry_expected)
                .unwrap();
        assert_eq!(after_both["clauses"][0]["text"], "place updated");
        assert_eq!(after_both["clauses"][1]["text"], "retry updated");

        let stale =
            merge_contract_clause_update(Some(&after_place), &place_update, &place_expected)
                .unwrap_err();
        assert!(stale.to_string().contains("stale Contract clause update"));
    }

    #[test]
    fn clause_scoped_update_cannot_change_contract_metadata() {
        let initial = json!({
            "schema_version": "1",
            "id": "contract.shared-concurrency",
            "applies_to": ["data.orders"],
            "clauses": [{"id": "policy", "text": "initial"}]
        });
        let update = json!({
            "schema_version": "1",
            "id": "contract.shared-concurrency",
            "applies_to": ["data.orders", "messaging.order-events"],
            "clauses": [{"id": "policy", "text": "updated"}]
        });
        let expected = BTreeMap::from([(
            "policy".to_owned(),
            canonical_digest(&initial["clauses"][0]).unwrap(),
        )]);

        let error = merge_contract_clause_update(Some(&initial), &update, &expected).unwrap_err();
        assert!(error.to_string().contains("cannot change metadata"));
    }
}
