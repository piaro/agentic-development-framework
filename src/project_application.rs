//! Long-running adapter session over a Project that is reloaded for every call.
//!
//! Issued Context is operational session state, not an authoritative Project
//! Record. Git, Records, and the verified Framework Release are re-read before
//! every evaluation or write.

use crate::application::{ApplicationResponse, ApplicationSubmission};
use crate::cli_output::next_response_value;
use crate::context::GeneratedContext;
use crate::execution_log::ExecutionLog;
use crate::execution_record::{
    BeginExecutionRequest, CompleteExecutionRequest, ExecutionEventStore,
};
use crate::kernel::ProjectSnapshot;
use crate::project_runtime::LoadedProject;
use crate::submission::ResultSubmission;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::{Path, PathBuf};

pub const MCP_APPLICATION_PROTOCOL_VERSION: &str = "1";

/// Describe a Record-shaped JSON value as a Schema object rather than the
/// boolean Schema `true` that `serde_json::Value` produces on its own.
///
/// `true` accepts every instance and is valid JSON Schema, but it carries no
/// type for a client to act on. MCP clients that convert tool Schemas into
/// their own validators reject a boolean where a Schema object is expected, and
/// a client that does accept it still has nothing telling it to send an object
/// rather than a string. Every value described here is a Record or a Report,
/// and those are always JSON objects.
pub(crate) fn json_object_schema(_generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
    schemars::json_schema!({"type": "object"})
}

/// Describe the optimistic concurrency digest, which is the digest of the
/// Record being replaced or `null` when the Record is being created.
pub(crate) fn expected_digest_schema(
    _generator: &mut schemars::SchemaGenerator,
) -> schemars::Schema {
    schemars::json_schema!({"type": ["string", "null"]})
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
pub struct IssuedActionKey {
    pub change_id: String,
    pub action_id: String,
    pub context_digest: String,
}

#[derive(Debug, Clone)]
struct IssuedActionEntry {
    context: GeneratedContext,
    framework_lock_digest: String,
    rule_index_digest: String,
    registered_output_refs: BTreeSet<String>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct NextServiceResponse {
    pub schema_version: String,
    #[schemars(schema_with = "json_object_schema")]
    pub next_response: Value,
    pub issued_action: Option<IssuedActionKey>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct SubmitServiceResponse {
    pub schema_version: String,
    pub result_id: String,
    pub already_completed: bool,
    #[schemars(schema_with = "json_object_schema")]
    pub next_response: Value,
    pub issued_action: Option<IssuedActionKey>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct RecordWriteResponse {
    pub schema_version: String,
    pub output_ref: String,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct AbandonActionResponse {
    pub schema_version: String,
    pub abandoned: bool,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct BeginExecutionResponse {
    pub schema_version: String,
    pub execution_id: String,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct CompleteExecutionResponse {
    pub schema_version: String,
    pub execution_id: String,
    pub already_completed: bool,
}

pub struct ProjectApplicationService {
    project_root: PathBuf,
    release_root: Option<PathBuf>,
    issued: BTreeMap<IssuedActionKey, IssuedActionEntry>,
}

impl ProjectApplicationService {
    pub fn new(
        project_root: impl AsRef<Path>,
        release_root: Option<PathBuf>,
    ) -> Result<Self, ServiceError> {
        let project_root = project_root
            .as_ref()
            .canonicalize()
            .map_err(|error| service_error("PROJECT_INVALID", error.to_string(), false))?;
        // Validate configuration and the pinned Release before accepting the
        // session. Every operation still reloads these inputs.
        LoadedProject::load(&project_root, release_root.as_deref(), false)
            .map_err(project_error)?;
        Ok(Self {
            project_root,
            release_root,
            issued: BTreeMap::new(),
        })
    }

    pub fn next(
        &mut self,
        change_id: &str,
        require_clean: bool,
    ) -> Result<NextServiceResponse, ServiceError> {
        let project = self.load(require_clean)?;
        if require_clean {
            project
                .assert_tracked_inputs(change_id)
                .map_err(project_error)?;
        }
        let mut application = project.application().map_err(application_error)?;
        let response = application.next(change_id).map_err(application_error)?;
        let rule_index_digest = application.rule_index_digest().to_owned();
        let framework_lock_digest = application.framework_lock_digest().to_owned();
        let next_response = next_response_value(change_id, &response);
        let issued_action = self.register_response(
            change_id,
            &response,
            &rule_index_digest,
            &framework_lock_digest,
        );
        Ok(NextServiceResponse {
            schema_version: MCP_APPLICATION_PROTOCOL_VERSION.to_owned(),
            next_response,
            issued_action,
        })
    }

    pub fn explain(&self, change_id: &str, require_clean: bool) -> Result<Value, ServiceError> {
        let project = self.load(require_clean)?;
        if require_clean {
            project
                .assert_tracked_inputs(change_id)
                .map_err(project_error)?;
        }
        let application = project.application().map_err(application_error)?;
        application
            .explain(change_id)
            .map(|report| report.as_value())
            .map_err(application_error)
    }

    pub fn contract_health(&self, require_clean: bool) -> Result<Value, ServiceError> {
        let project = self.load(require_clean)?;
        if require_clean {
            project
                .assert_tracked_project_inputs()
                .map_err(project_error)?;
        }
        let application = project.application().map_err(application_error)?;
        application
            .contract_health()
            .map(|report| report.as_value())
            .map_err(application_error)
    }

    pub fn execution_log(
        &self,
        change_id: &str,
        require_clean: bool,
    ) -> Result<Value, ServiceError> {
        let project = self.load(require_clean)?;
        if require_clean {
            project
                .assert_tracked_inputs(change_id)
                .map_err(project_error)?;
        }
        let application = project.application().map_err(application_error)?;
        let snapshot = application.snapshot(change_id).map_err(application_error)?;
        let events = ExecutionEventStore::open(project.root())
            .and_then(|store| store.events(change_id))
            .map_err(project_error)?;
        Ok(ExecutionLog::build_with_events(&snapshot, &events).as_value())
    }

    pub fn begin_execution(
        &mut self,
        key: &IssuedActionKey,
        request: BeginExecutionRequest,
    ) -> Result<BeginExecutionResponse, ServiceError> {
        let entry = self.issued_entry(key)?;
        let project = self.load(false)?;
        let context_bytes = serde_json::to_vec(&entry.context)
            .map_err(application_error)?
            .len() as u64;
        let role = required_context_string(&entry.context, &["action", "role"])?;
        let result_schema =
            required_context_string(&entry.context, &["action", "expected_result_schema"])?;
        let started = ExecutionEventStore::open(project.root())
            .and_then(|store| store.begin(key, role, result_schema, context_bytes, request))
            .map_err(project_error)?;
        Ok(BeginExecutionResponse {
            schema_version: MCP_APPLICATION_PROTOCOL_VERSION.to_owned(),
            execution_id: started.execution_id,
        })
    }

    pub fn complete_execution(
        &self,
        execution_id: &str,
        request: CompleteExecutionRequest,
    ) -> Result<CompleteExecutionResponse, ServiceError> {
        let store = ExecutionEventStore::open(&self.project_root).map_err(project_error)?;
        let (completed, already_completed) = store
            .complete_bound(execution_id, request)
            .map_err(project_error)?;
        Ok(CompleteExecutionResponse {
            schema_version: MCP_APPLICATION_PROTOCOL_VERSION.to_owned(),
            execution_id: completed.execution_id,
            already_completed,
        })
    }

    pub fn submit(
        &mut self,
        key: &IssuedActionKey,
        payload: Value,
        output_refs: Vec<String>,
        execution: Option<Value>,
    ) -> Result<SubmitServiceResponse, ServiceError> {
        let entry = match self.issued.get(key).cloned() {
            Some(entry) => entry,
            None => {
                // A Result already stored for this Action and Context means the
                // submission arrived twice; replay it rather than writing again.
                if let Some(response) = self.replay_submission(key, &payload, &output_refs)? {
                    return Ok(response);
                }
                self.resume(key)?
            }
        };

        let project = self.load(false)?;
        let mut application = project.application().map_err(application_error)?;
        let snapshot = application
            .snapshot(&key.change_id)
            .map_err(application_error)?;
        validate_output_refs(&entry, &output_refs, &snapshot)?;
        assert_framework_identity(&entry, &application)?;
        let role = required_context_string(&entry.context, &["action", "role"])?;
        let result_schema =
            required_context_string(&entry.context, &["action", "expected_result_schema"])?;
        let submission = ResultSubmission {
            change_id: key.change_id.clone(),
            action_id: key.action_id.clone(),
            context_digest: key.context_digest.clone(),
            role: role.to_owned(),
            result_schema: result_schema.to_owned(),
            payload,
            output_refs,
            execution,
        };
        let ApplicationSubmission { result, response } = application
            .submit_issued_with_snapshot(&entry.context, &submission, &snapshot)
            .map_err(application_error)?;
        let rule_index_digest = application.rule_index_digest().to_owned();
        let framework_lock_digest = application.framework_lock_digest().to_owned();
        let result_id = result["id"]
            .as_str()
            .expect("validated Result has an ID")
            .to_owned();
        let next_response = next_response_value(&key.change_id, &response);
        let next_issued = issued_entry(
            &key.change_id,
            &response,
            &rule_index_digest,
            &framework_lock_digest,
        );
        drop(application);
        drop(project);

        self.issued.remove(key);
        let issued_action = next_issued.map(|(next_key, next_entry)| {
            self.issued.insert(next_key.clone(), next_entry);
            next_key
        });
        Ok(SubmitServiceResponse {
            schema_version: MCP_APPLICATION_PROTOCOL_VERSION.to_owned(),
            result_id,
            already_completed: false,
            next_response,
            issued_action,
        })
    }

    /// Replays a submission whose Result is already stored, or `None` when this
    /// Action and Context produced no Result yet.
    fn replay_submission(
        &mut self,
        key: &IssuedActionKey,
        payload: &Value,
        output_refs: &[String],
    ) -> Result<Option<SubmitServiceResponse>, ServiceError> {
        let project = self.load(false)?;
        let mut application = project.application().map_err(application_error)?;
        let snapshot = application
            .snapshot(&key.change_id)
            .map_err(application_error)?;
        let Some(result) = snapshot.results.iter().find(|result| {
            result["action_id"].as_str() == Some(key.action_id.as_str())
                && result["context_digest"].as_str() == Some(key.context_digest.as_str())
        }) else {
            return Ok(None);
        };
        if !submitted_payload_matches(&result["payload"], payload)
            || result["output_refs"] != json!(output_refs)
        {
            return Err(service_error(
                "WRITE_CONFLICT",
                "A different Result already exists for this Action and Context",
                false,
            ));
        }
        let result_id = required_record_id(result, "Result")?.to_owned();
        let response = application
            .next(&key.change_id)
            .map_err(application_error)?;
        let rule_index_digest = application.rule_index_digest().to_owned();
        let framework_lock_digest = application.framework_lock_digest().to_owned();
        let next_response = next_response_value(&key.change_id, &response);
        let next_issued = issued_entry(
            &key.change_id,
            &response,
            &rule_index_digest,
            &framework_lock_digest,
        );
        drop(application);
        drop(project);
        let issued_action = next_issued.map(|(next_key, next_entry)| {
            self.issued.insert(next_key.clone(), next_entry);
            next_key
        });
        Ok(Some(SubmitServiceResponse {
            schema_version: MCP_APPLICATION_PROTOCOL_VERSION.to_owned(),
            result_id,
            already_completed: true,
            next_response,
            issued_action,
        }))
    }

    pub fn add_evidence(
        &mut self,
        key: &IssuedActionKey,
        mut evidence: Value,
    ) -> Result<RecordWriteResponse, ServiceError> {
        let entry = self.issued_entry(key)?;
        assert_action(&entry, Some("record-evidence"), Some("result.evidence"))?;
        assert_record_change(&evidence, key, "Evidence")?;
        let allowed_instances = context_instance_keys(&entry.context);
        let evidence_instances = string_set(&evidence["requirement_instances"]);
        if !evidence_instances.is_subset(&allowed_instances) {
            return Err(service_error(
                "ACTION_NOT_ALLOWED",
                "Evidence refers to a Requirement Instance outside the issued Action",
                false,
            ));
        }
        let input_refs = evidence_input_refs(&entry.context, &evidence_instances);
        evidence
            .as_object_mut()
            .expect("Evidence is validated as an object before persistence")
            .insert("input_refs".to_owned(), Value::Object(input_refs));
        let evidence_id = required_record_id(&evidence, "Evidence")?.to_owned();
        let project = self.load(false)?;
        let mut application = project.application().map_err(application_error)?;
        assert_framework_identity(&entry, &application)?;
        application
            .add_evidence(evidence)
            .map_err(application_error)?;
        self.register_output(key, &evidence_id)?;
        Ok(RecordWriteResponse {
            schema_version: MCP_APPLICATION_PROTOCOL_VERSION.to_owned(),
            output_ref: evidence_id,
        })
    }

    pub fn apply_decision(
        &mut self,
        key: &IssuedActionKey,
        decision: Value,
        expected_digest: Option<&str>,
    ) -> Result<RecordWriteResponse, ServiceError> {
        let entry = self.issued_entry(key)?;
        assert_action(&entry, Some("record-human-decision"), None)?;
        assert_record_change(&decision, key, "Decision")?;
        let decision_id = required_record_id(&decision, "Decision")?.to_owned();
        let project = self.load(false)?;
        let mut application = project.application().map_err(application_error)?;
        assert_framework_identity(&entry, &application)?;
        let snapshot = application
            .snapshot(&key.change_id)
            .map_err(application_error)?;
        assert_decision_resolves_human_answer(&decision, &snapshot.results)?;
        application
            .upsert_decision(decision, expected_digest)
            .map_err(application_error)?;
        self.register_output(key, &decision_id)?;
        Ok(RecordWriteResponse {
            schema_version: MCP_APPLICATION_PROTOCOL_VERSION.to_owned(),
            output_ref: decision_id,
        })
    }

    pub fn apply_contract(
        &mut self,
        key: &IssuedActionKey,
        contract: Value,
        expected_digest: Option<&str>,
        expected_clause_digests: Option<&BTreeMap<String, String>>,
    ) -> Result<RecordWriteResponse, ServiceError> {
        if expected_digest.is_some() && expected_clause_digests.is_some() {
            return Err(ServiceError::invalid_argument(
                "expected_digest and expected_clause_digests are mutually exclusive",
            ));
        }
        let entry = self.issued_entry(key)?;
        let action = required_context_string(&entry.context, &["action", "action"])?;
        if !matches!(
            action,
            "record-human-decision" | "establish-impact-governance"
        ) {
            return Err(service_error(
                "ACTION_NOT_ALLOWED",
                format!("Issued Action does not allow a Contract write: {action}"),
                false,
            ));
        }
        assert_contract_scope(&contract, key)?;
        let contract_id = required_record_id(&contract, "Contract")?.to_owned();
        let authority_refs = contract
            .get("clauses")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|clause| clause["authority_ref"].as_str())
            .collect::<BTreeSet<_>>();
        if action == "record-human-decision" && authority_refs.is_empty() {
            return Err(service_error(
                "ACTION_NOT_ALLOWED",
                "A Contract written from a Human decision must reference that Decision authority",
                false,
            ));
        }
        let project = self.load(false)?;
        let mut application = project.application().map_err(application_error)?;
        assert_framework_identity(&entry, &application)?;
        let snapshot = application
            .snapshot(&key.change_id)
            .map_err(application_error)?;
        let known_decisions = snapshot
            .decisions
            .iter()
            .filter_map(|decision| decision["id"].as_str())
            .collect::<BTreeSet<_>>();
        if !authority_refs.is_subset(&known_decisions) {
            return Err(service_error(
                "ACTION_NOT_ALLOWED",
                "Contract authority does not refer to a current Decision",
                false,
            ));
        }
        if let Some(expected_clause_digests) = expected_clause_digests {
            application
                .upsert_contract_clauses(contract, expected_clause_digests)
                .map_err(application_error)?;
        } else {
            application
                .upsert_contract(contract, expected_digest)
                .map_err(application_error)?;
        }
        self.register_output(key, &contract_id)?;
        Ok(RecordWriteResponse {
            schema_version: MCP_APPLICATION_PROTOCOL_VERSION.to_owned(),
            output_ref: contract_id,
        })
    }

    pub fn abandon(&mut self, key: &IssuedActionKey) -> AbandonActionResponse {
        AbandonActionResponse {
            schema_version: MCP_APPLICATION_PROTOCOL_VERSION.to_owned(),
            abandoned: self.issued.remove(key).is_some(),
        }
    }

    fn load(&self, require_clean: bool) -> Result<LoadedProject, ServiceError> {
        LoadedProject::load(
            &self.project_root,
            self.release_root.as_deref(),
            require_clean,
        )
        .map_err(project_error)
    }

    /// The entry for `key`, rebuilding it when the Action is still the current
    /// one for its change.
    ///
    /// Action identity is a digest of the Action body, and the Context digest
    /// binds the inputs it was built from, so reevaluating a change reproduces
    /// the same key whenever nothing it depends on has moved. That is what lets
    /// work survive a restart or a dropped connection: an Action the process no
    /// longer remembers is accepted precisely when the control plane would issue
    /// it again, and refused as superseded when it would not.
    fn issued_entry(&mut self, key: &IssuedActionKey) -> Result<IssuedActionEntry, ServiceError> {
        if let Some(entry) = self.issued.get(key) {
            return Ok(entry.clone());
        }
        self.resume(key)
    }

    fn resume(&mut self, key: &IssuedActionKey) -> Result<IssuedActionEntry, ServiceError> {
        let project = self.load(false)?;
        let mut application = project.application().map_err(application_error)?;
        let response = application
            .next(&key.change_id)
            .map_err(application_error)?;
        let rule_index_digest = application.rule_index_digest().to_owned();
        let framework_lock_digest = application.framework_lock_digest().to_owned();
        drop(application);
        drop(project);

        let Some((current_key, entry)) = issued_entry(
            &key.change_id,
            &response,
            &rule_index_digest,
            &framework_lock_digest,
        ) else {
            return Err(service_error(
                "ACTION_NOT_CURRENT",
                format!(
                    "Action {} is no longer current: {} has no Action to work on. \
                     Call next to see where it stands.",
                    key.action_id, key.change_id
                ),
                false,
            ));
        };
        if &current_key != key {
            return Err(service_error(
                "ACTION_NOT_CURRENT",
                format!(
                    "Action {} is no longer current: {} now expects {} against Context {}. \
                     Call next and redo the work against that Action.",
                    key.action_id, key.change_id, current_key.action_id, current_key.context_digest
                ),
                false,
            ));
        }
        self.issued.insert(key.clone(), entry.clone());
        Ok(entry)
    }

    fn register_output(
        &mut self,
        key: &IssuedActionKey,
        output_ref: &str,
    ) -> Result<(), ServiceError> {
        let entry = self.issued.get_mut(key).ok_or_else(|| {
            service_error(
                "INTERNAL",
                "Issued Action disappeared before its output was registered",
                true,
            )
        })?;
        entry.registered_output_refs.insert(output_ref.to_owned());
        Ok(())
    }

    fn register_response(
        &mut self,
        change_id: &str,
        response: &ApplicationResponse,
        rule_index_digest: &str,
        framework_lock_digest: &str,
    ) -> Option<IssuedActionKey> {
        issued_entry(
            change_id,
            response,
            rule_index_digest,
            framework_lock_digest,
        )
        .map(|(key, entry)| {
            self.issued.insert(key.clone(), entry);
            key
        })
    }
}

fn evidence_input_refs(
    context: &GeneratedContext,
    instance_keys: &BTreeSet<&str>,
) -> serde_json::Map<String, Value> {
    instance_keys
        .iter()
        .filter_map(|instance_key| context.instance_source_digests.get(*instance_key))
        .flat_map(|refs| refs.iter())
        .map(|(reference, digest)| (reference.clone(), Value::String(digest.clone())))
        .collect()
}

fn submitted_payload_matches(stored: &Value, submitted: &Value) -> bool {
    match (stored, submitted) {
        (Value::Object(stored), Value::Object(submitted)) => {
            if submitted
                .keys()
                .any(|key| matches!(key.as_str(), "input_refs" | "freshness_refs"))
            {
                return false;
            }
            stored.iter().all(|(key, value)| {
                matches!(key.as_str(), "input_refs" | "freshness_refs")
                    || submitted
                        .get(key)
                        .is_some_and(|submitted| submitted_payload_matches(value, submitted))
            }) && submitted.keys().all(|key| stored.contains_key(key))
        }
        (Value::Array(stored), Value::Array(submitted)) => {
            stored.len() == submitted.len()
                && stored
                    .iter()
                    .zip(submitted)
                    .all(|(stored, submitted)| submitted_payload_matches(stored, submitted))
        }
        _ => stored == submitted,
    }
}

fn issued_entry(
    change_id: &str,
    response: &ApplicationResponse,
    rule_index_digest: &str,
    framework_lock_digest: &str,
) -> Option<(IssuedActionKey, IssuedActionEntry)> {
    let context = response.context.clone()?;
    let key = IssuedActionKey {
        change_id: change_id.to_owned(),
        action_id: context.action_id.clone(),
        context_digest: context.digest.clone(),
    };
    Some((
        key,
        IssuedActionEntry {
            context,
            framework_lock_digest: framework_lock_digest.to_owned(),
            rule_index_digest: rule_index_digest.to_owned(),
            registered_output_refs: BTreeSet::new(),
        },
    ))
}

/// Whether the change's records hold a record of `kind` with this identifier.
fn change_holds_record(snapshot: &ProjectSnapshot, kind: &str, output_ref: &str) -> bool {
    let records = match kind {
        "contract" => &snapshot.contracts,
        "decision" => &snapshot.decisions,
        "evidence" => &snapshot.evidence,
        _ => return false,
    };
    records
        .iter()
        .any(|record| record["id"].as_str() == Some(output_ref))
}

fn validate_output_refs(
    entry: &IssuedActionEntry,
    output_refs: &[String],
    snapshot: &ProjectSnapshot,
) -> Result<(), ServiceError> {
    let unique = output_refs.iter().collect::<BTreeSet<_>>();
    if unique.len() != output_refs.len() {
        return Err(service_error(
            "OUTPUT_REF_INVALID",
            "output_refs must be unique",
            false,
        ));
    }
    let action = required_context_string(&entry.context, &["action", "action"])?;
    for output_ref in output_refs {
        let kind = output_ref
            .split_once('.')
            .map_or(output_ref.as_str(), |item| item.0);
        if matches!(kind, "contract" | "decision" | "evidence") {
            // A record written through this session is known immediately. After
            // reconnecting, `next` reissues the same Action and creates a fresh
            // registry entry, so it must also accept a record that is already
            // present in the current Change snapshot. Otherwise a caller that
            // correctly starts with `next` cannot submit work completed before
            // the reconnect.
            let written = entry.registered_output_refs.contains(output_ref)
                || change_holds_record(snapshot, kind, output_ref);
            if !written {
                return Err(service_error(
                    "OUTPUT_REF_INVALID",
                    format!("Record output was not written by this issued Action: {output_ref}"),
                    false,
                ));
            }
        } else if action != "implement-change" {
            return Err(service_error(
                "OUTPUT_REF_INVALID",
                format!("Repository output is not allowed for Action {action}: {output_ref}"),
                false,
            ));
        }
    }
    Ok(())
}

fn assert_framework_identity<Store: crate::application::ProjectStore>(
    entry: &IssuedActionEntry,
    application: &crate::application::Application<'_, Store>,
) -> Result<(), ServiceError> {
    if entry.framework_lock_digest != application.framework_lock_digest()
        || entry.rule_index_digest != application.rule_index_digest()
    {
        return Err(service_error(
            "RELEASE_MISMATCH",
            "Framework identity changed after the Action was issued",
            false,
        ));
    }
    Ok(())
}

fn assert_action(
    entry: &IssuedActionEntry,
    expected_action: Option<&str>,
    expected_schema: Option<&str>,
) -> Result<(), ServiceError> {
    let action = required_context_string(&entry.context, &["action", "action"])?;
    let schema = required_context_string(&entry.context, &["action", "expected_result_schema"])?;
    if expected_action.is_some_and(|expected| action != expected)
        || expected_schema.is_some_and(|expected| schema != expected)
    {
        return Err(service_error(
            "ACTION_NOT_ALLOWED",
            format!("Issued Action does not allow this write: {action} ({schema})"),
            false,
        ));
    }
    Ok(())
}

fn required_context_string<'a>(
    context: &'a GeneratedContext,
    path: &[&str],
) -> Result<&'a str, ServiceError> {
    let mut current = &context.payload;
    for key in path {
        current = &current[*key];
    }
    current.as_str().ok_or_else(|| {
        service_error(
            "INTERNAL",
            format!("Issued Context is missing {}", path.join(".")),
            false,
        )
    })
}

fn context_instance_keys(context: &GeneratedContext) -> BTreeSet<&str> {
    context.payload["requirement_instances"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|instance| instance["instance_key"].as_str())
        .collect()
}

fn string_set(value: &Value) -> BTreeSet<&str> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect()
}

fn assert_record_change(
    record: &Value,
    key: &IssuedActionKey,
    label: &str,
) -> Result<(), ServiceError> {
    if record["change_id"].as_str() != Some(key.change_id.as_str()) {
        return Err(service_error(
            "ACTION_NOT_ALLOWED",
            format!("{label} change_id does not match the issued Action"),
            false,
        ));
    }
    Ok(())
}

fn assert_contract_scope(contract: &Value, key: &IssuedActionKey) -> Result<(), ServiceError> {
    if contract
        .get("change_id")
        .is_some_and(|change_id| change_id.as_str() != Some(key.change_id.as_str()))
    {
        return Err(service_error(
            "ACTION_NOT_ALLOWED",
            "Contract change_id does not match the issued Action",
            false,
        ));
    }
    Ok(())
}

fn required_record_id<'a>(record: &'a Value, label: &str) -> Result<&'a str, ServiceError> {
    record["id"].as_str().ok_or_else(|| {
        service_error(
            "INVALID_ARGUMENT",
            format!("{label} id must be a string"),
            false,
        )
    })
}

fn assert_decision_resolves_human_answer(
    decision: &Value,
    results: &[Value],
) -> Result<(), ServiceError> {
    let resolves = string_set(&decision["resolves"]);
    let answered = results
        .iter()
        .filter(|result| result["result_schema"].as_str() == Some("result.human-answer"))
        .flat_map(|result| {
            result["payload"]["answers"]
                .as_array()
                .into_iter()
                .flatten()
        })
        .filter_map(|answer| answer["request_id"].as_str())
        .collect::<BTreeSet<_>>();
    if resolves.is_empty() || resolves.is_disjoint(&answered) {
        return Err(service_error(
            "ACTION_NOT_ALLOWED",
            "Decision does not resolve a current Human answer",
            false,
        ));
    }
    Ok(())
}

fn project_error(error: impl fmt::Display) -> ServiceError {
    service_error("PROJECT_INVALID", error.to_string(), false)
}

fn application_error(error: impl fmt::Display) -> ServiceError {
    let message = error.to_string();
    let code = if message.contains("stale") {
        "CONTEXT_STALE"
    } else if message.contains("duplicate") || message.contains("already exists") {
        "WRITE_CONFLICT"
    } else {
        "INVALID_ARGUMENT"
    };
    service_error(code, message, false)
}

fn service_error(code: &str, message: impl Into<String>, retryable: bool) -> ServiceError {
    ServiceError {
        code: code.to_owned(),
        message: message.into(),
        retryable,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct ServiceError {
    pub code: String,
    pub message: String,
    pub retryable: bool,
}

impl ServiceError {
    pub fn invalid_argument(message: impl Into<String>) -> Self {
        service_error("INVALID_ARGUMENT", message, false)
    }

    pub fn as_value(&self) -> Value {
        json!({
            "schema_version": MCP_APPLICATION_PROTOCOL_VERSION,
            "code": self.code,
            "message": self.message,
            "retryable": self.retryable,
            "details": {},
        })
    }
}

impl fmt::Display for ServiceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ServiceError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contract_scope_accepts_shared_and_matching_change_contracts() {
        let key = IssuedActionKey {
            change_id: "change.place-order".to_owned(),
            action_id: "action.test".to_owned(),
            context_digest: "sha256:test".to_owned(),
        };

        assert_contract_scope(&json!({"id": "contract.shared"}), &key).unwrap();
        assert_contract_scope(
            &json!({
                "id": "contract.local",
                "change_id": "change.place-order"
            }),
            &key,
        )
        .unwrap();
    }

    #[test]
    fn contract_scope_rejects_another_change_contract() {
        let key = IssuedActionKey {
            change_id: "change.place-order".to_owned(),
            action_id: "action.test".to_owned(),
            context_digest: "sha256:test".to_owned(),
        };

        let error = assert_contract_scope(
            &json!({
                "id": "contract.other",
                "change_id": "change.retry-order-events"
            }),
            &key,
        )
        .unwrap_err();
        assert_eq!(error.code, "ACTION_NOT_ALLOWED");
    }

    #[test]
    fn submit_accepts_persisted_evidence_after_next_reissues_the_action() {
        let entry = IssuedActionEntry {
            context: GeneratedContext {
                action_id: "action.record-evidence".to_owned(),
                role: "Builder".to_owned(),
                source_refs: Vec::new(),
                source_digests: BTreeMap::new(),
                instance_source_digests: BTreeMap::new(),
                contract_clause_projection_version: "1".to_owned(),
                contract_clauses: Vec::new(),
                contract_clauses_digest: "sha256:test".to_owned(),
                payload: json!({"action": {"action": "record-evidence"}}),
                digest: "sha256:test".to_owned(),
            },
            framework_lock_digest: "sha256:test".to_owned(),
            rule_index_digest: "sha256:test".to_owned(),
            registered_output_refs: BTreeSet::new(),
        };
        let snapshot = ProjectSnapshot {
            change_id: "change.place-order".to_owned(),
            change: json!({}),
            contracts: Vec::new(),
            decisions: Vec::new(),
            results: Vec::new(),
            evidence: vec![json!({"id": "evidence.persisted"})],
            repository: json!({}),
            artifact_digests: BTreeMap::new(),
            digest: "sha256:test".to_owned(),
        };

        validate_output_refs(&entry, &["evidence.persisted".to_owned()], &snapshot).unwrap();
    }

    #[test]
    fn evidence_inputs_are_derived_from_the_issued_requirement_instances() {
        let digest = |character: char| format!("sha256:{}", character.to_string().repeat(64));
        let context = GeneratedContext {
            action_id: "action.record-evidence".to_owned(),
            role: "Builder".to_owned(),
            source_refs: Vec::new(),
            source_digests: BTreeMap::new(),
            instance_source_digests: BTreeMap::from([
                (
                    "tests-passed|operation.one".to_owned(),
                    BTreeMap::from([("code.one".to_owned(), digest('1'))]),
                ),
                (
                    "tests-passed|operation.two".to_owned(),
                    BTreeMap::from([
                        ("code.two".to_owned(), digest('2')),
                        ("contract.shared#rule".to_owned(), digest('3')),
                    ]),
                ),
            ]),
            contract_clause_projection_version: "1".to_owned(),
            contract_clauses: Vec::new(),
            contract_clauses_digest: digest('4'),
            payload: json!({}),
            digest: digest('5'),
        };
        let instance_keys =
            BTreeSet::from(["tests-passed|operation.one", "tests-passed|operation.two"]);

        assert_eq!(
            evidence_input_refs(&context, &instance_keys),
            serde_json::Map::from_iter([
                ("code.one".to_owned(), Value::String(digest('1'))),
                ("code.two".to_owned(), Value::String(digest('2'))),
                (
                    "contract.shared#rule".to_owned(),
                    Value::String(digest('3')),
                ),
            ])
        );
    }
}
