//! Append-only execution attempt events reported by an external runner.
//!
//! These records are operational observations. They never participate in
//! Kernel decisions, Result identity, freshness, or Evidence validation.

use crate::canonical_digest;
use crate::project_application::IssuedActionKey;
use crate::schema::validate_json_document;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

pub const EXECUTION_EVENT_SCHEMA_VERSION: &str = "1";
static EXECUTION_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RunnerIdentity {
    pub provider: String,
    pub surface: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BeginExecutionRequest {
    pub runner: RunnerIdentity,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CompleteExecutionRequest {
    pub change_id: String,
    pub status: ExecutionStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_creation_input_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cached_input_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_output_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retries: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum ExecutionStatus {
    Succeeded,
    Failed,
    Interrupted,
    Stale,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionStarted {
    pub schema_version: String,
    pub event: String,
    pub execution_id: String,
    pub change_id: String,
    pub action_id: String,
    pub context_digest: String,
    pub role: String,
    pub result_schema: String,
    pub context_bytes: u64,
    pub runner: RunnerIdentity,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionCompleted {
    pub schema_version: String,
    pub event: String,
    pub execution_id: String,
    #[serde(flatten)]
    pub completion: CompleteExecutionRequest,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event")]
pub enum ExecutionEvent {
    #[serde(rename = "started")]
    Started {
        schema_version: String,
        execution_id: String,
        change_id: String,
        action_id: String,
        context_digest: String,
        role: String,
        result_schema: String,
        context_bytes: u64,
        runner: RunnerIdentity,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        started_at: Option<String>,
    },
    #[serde(rename = "completed")]
    Completed {
        schema_version: String,
        execution_id: String,
        #[serde(flatten)]
        completion: CompleteExecutionRequest,
    },
}

impl ExecutionEvent {
    pub fn execution_id(&self) -> &str {
        match self {
            Self::Started { execution_id, .. } | Self::Completed { execution_id, .. } => {
                execution_id
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct ExecutionEventStore {
    project_root: PathBuf,
}

impl ExecutionEventStore {
    pub fn open(project_root: impl AsRef<Path>) -> Result<Self, String> {
        let project_root = project_root
            .as_ref()
            .canonicalize()
            .map_err(|error| format!("cannot resolve project root: {error}"))?;
        Ok(Self { project_root })
    }

    pub fn begin(
        &self,
        key: &IssuedActionKey,
        role: &str,
        result_schema: &str,
        context_bytes: u64,
        request: BeginExecutionRequest,
    ) -> Result<ExecutionStarted, String> {
        validate_runner(&request.runner)?;
        let execution_id = execution_id(key)?;
        let event = ExecutionStarted {
            schema_version: EXECUTION_EVENT_SCHEMA_VERSION.to_owned(),
            event: "started".to_owned(),
            execution_id,
            change_id: key.change_id.clone(),
            action_id: key.action_id.clone(),
            context_digest: key.context_digest.clone(),
            role: required_text(role, "role")?.to_owned(),
            result_schema: required_text(result_schema, "result_schema")?.to_owned(),
            context_bytes,
            runner: request.runner,
            started_at: optional_text(request.started_at, "started_at")?,
        };
        self.write_new(
            &event.change_id,
            &format!("{}.started.json", event.execution_id),
            &serde_json::to_value(&event).map_err(|error| error.to_string())?,
        )?;
        Ok(event)
    }

    pub fn complete(
        &self,
        execution_id: &str,
        request: CompleteExecutionRequest,
    ) -> Result<(ExecutionCompleted, bool), String> {
        validate_execution_id(execution_id)?;
        validate_completion(&request)?;
        let started = self.started(&request.change_id, execution_id)?;
        let event = ExecutionCompleted {
            schema_version: EXECUTION_EVENT_SCHEMA_VERSION.to_owned(),
            event: "completed".to_owned(),
            execution_id: execution_id.to_owned(),
            completion: request,
        };
        let value = serde_json::to_value(&event).map_err(|error| error.to_string())?;
        let filename = format!("{execution_id}.completed.json");
        match self.write_new(&started.change_id, &filename, &value) {
            Ok(()) => Ok((event, false)),
            Err(error) if error.starts_with("WRITE_CONFLICT:") => {
                let existing = self.read_event(&started.change_id, &filename)?;
                if existing == value {
                    Ok((event, true))
                } else {
                    Err(format!(
                        "a different completion already exists for {execution_id}"
                    ))
                }
            }
            Err(error) => Err(error),
        }
    }

    pub fn complete_bound(
        &self,
        execution_id: &str,
        request: CompleteExecutionRequest,
    ) -> Result<(ExecutionCompleted, bool), String> {
        let started = self.started(&request.change_id, execution_id)?;
        if let Some(result_id) = request.result_id.as_deref() {
            self.validate_result_binding(&started, result_id)?;
        }
        self.complete(execution_id, request)
    }

    pub fn events(&self, change_id: &str) -> Result<Vec<ExecutionEvent>, String> {
        validate_record_id(change_id, "change")?;
        let directory = self.validated_execution_root(change_id)?;
        if !directory.exists() {
            return Ok(Vec::new());
        }
        let mut paths = fs::read_dir(&directory)
            .map_err(|error| format!("{}: {error}", directory.display()))?
            .map(|entry| {
                entry
                    .map(|value| value.path())
                    .map_err(|error| error.to_string())
            })
            .collect::<Result<Vec<_>, _>>()?;
        paths.retain(|path| path.extension().and_then(|value| value.to_str()) == Some("json"));
        paths.sort();
        paths
            .into_iter()
            .map(|path| {
                let bytes = read_regular_file(&path)?;
                serde_json::from_slice(&bytes)
                    .map_err(|error| format!("{}: {error}", path.display()))
            })
            .collect()
    }

    pub fn started(&self, change_id: &str, execution_id: &str) -> Result<ExecutionStarted, String> {
        validate_record_id(change_id, "change")?;
        let filename = format!("{execution_id}.started.json");
        let candidate = self.validated_execution_root(change_id)?.join(filename);
        let bytes = read_regular_file(&candidate)?;
        serde_json::from_slice(&bytes).map_err(|error| format!("{}: {error}", candidate.display()))
    }

    fn validated_execution_root(&self, change_id: &str) -> Result<PathBuf, String> {
        self.validated_change_subdirectory(change_id, "executions", true)
    }

    fn validate_result_binding(
        &self,
        started: &ExecutionStarted,
        result_id: &str,
    ) -> Result<(), String> {
        validate_record_id(result_id, "result")?;
        let result_root =
            self.validated_change_subdirectory(&started.change_id, "results", false)?;
        let mut paths = fs::read_dir(&result_root)
            .map_err(|error| format!("{}: {error}", result_root.display()))?
            .map(|entry| {
                entry
                    .map(|value| value.path())
                    .map_err(|error| error.to_string())
            })
            .collect::<Result<Vec<_>, _>>()?;
        paths.retain(|path| path.extension().and_then(|value| value.to_str()) == Some("json"));
        paths.sort();
        for path in paths {
            let bytes = read_regular_file(&path)?;
            let result: Value = serde_json::from_slice(&bytes)
                .map_err(|error| format!("{}: {error}", path.display()))?;
            if result["id"].as_str() == Some(result_id) {
                if result["action_id"].as_str() == Some(started.action_id.as_str())
                    && result["context_digest"].as_str() == Some(started.context_digest.as_str())
                {
                    return Ok(());
                }
                return Err(format!(
                    "Result {result_id} belongs to a different Action or Context"
                ));
            }
        }
        Err(format!(
            "Result {result_id} does not exist for {}",
            started.change_id
        ))
    }

    fn validated_change_subdirectory(
        &self,
        change_id: &str,
        name: &str,
        allow_missing: bool,
    ) -> Result<PathBuf, String> {
        validate_record_id(change_id, "change")?;
        let change_root = self.project_root.join(".adf/changes").join(change_id);
        let metadata = fs::symlink_metadata(&change_root)
            .map_err(|error| format!("{}: {error}", change_root.display()))?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(format!(
                "Change directory is not a regular directory: {}",
                change_root.display()
            ));
        }
        let canonical = change_root
            .canonicalize()
            .map_err(|error| format!("{}: {error}", change_root.display()))?;
        if !canonical.starts_with(&self.project_root) {
            return Err("Change directory escapes the project root".to_owned());
        }
        let directory = change_root.join(name);
        if directory.exists() {
            let metadata = fs::symlink_metadata(&directory)
                .map_err(|error| format!("{}: {error}", directory.display()))?;
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Err(format!(
                    "{name} directory is not a regular directory: {}",
                    directory.display()
                ));
            }
        } else if !allow_missing {
            return Err(format!("{} does not exist", directory.display()));
        }
        Ok(directory)
    }

    fn write_new(&self, change_id: &str, filename: &str, value: &Value) -> Result<(), String> {
        validate_record_id(change_id, "change")?;
        validate_execution_event(value)?;
        let directory = self.validated_execution_root(change_id)?;
        fs::create_dir_all(&directory)
            .map_err(|error| format!("{}: {error}", directory.display()))?;
        let path = directory.join(filename);
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|error| {
                if error.kind() == std::io::ErrorKind::AlreadyExists {
                    format!("WRITE_CONFLICT: {} already exists", path.display())
                } else {
                    format!("{}: {error}", path.display())
                }
            })?;
        let mut bytes = serde_json::to_vec_pretty(value).map_err(|error| error.to_string())?;
        bytes.push(b'\n');
        file.write_all(&bytes)
            .and_then(|_| file.sync_all())
            .map_err(|error| format!("{}: {error}", path.display()))
    }

    fn read_event(&self, change_id: &str, filename: &str) -> Result<Value, String> {
        let path = self.validated_execution_root(change_id)?.join(filename);
        let bytes = read_regular_file(&path)?;
        serde_json::from_slice(&bytes).map_err(|error| format!("{}: {error}", path.display()))
    }
}

fn read_regular_file(path: &Path) -> Result<Vec<u8>, String> {
    let metadata =
        fs::symlink_metadata(path).map_err(|error| format!("{}: {error}", path.display()))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(format!(
            "Execution event is not a regular file: {}",
            path.display()
        ));
    }
    fs::read(path).map_err(|error| format!("{}: {error}", path.display()))
}

fn validate_execution_event(value: &Value) -> Result<(), String> {
    let schema: Value =
        serde_json::from_str(include_str!("../schemas/v1/execution-event.schema.json"))
            .map_err(|error| format!("invalid bundled execution event Schema: {error}"))?;
    validate_json_document(value, &schema).map_err(|error| error.to_string())
}

fn execution_id(key: &IssuedActionKey) -> Result<String, String> {
    let sequence = EXECUTION_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_nanos()
        .to_string();
    let digest = canonical_digest(&json!({
        "change_id": key.change_id,
        "action_id": key.action_id,
        "context_digest": key.context_digest,
        "process_id": std::process::id(),
        "sequence": sequence,
        "time": nanos,
    }))
    .map_err(|error| error.to_string())?;
    Ok(format!(
        "execution.{}",
        digest
            .strip_prefix("sha256:")
            .expect("canonical digest has a prefix")
            .chars()
            .take(20)
            .collect::<String>()
    ))
}

fn validate_runner(runner: &RunnerIdentity) -> Result<(), String> {
    required_text(&runner.provider, "runner.provider")?;
    required_text(&runner.surface, "runner.surface")?;
    optional_text(runner.version.clone(), "runner.version")?;
    Ok(())
}

fn validate_completion(request: &CompleteExecutionRequest) -> Result<(), String> {
    validate_record_id(&request.change_id, "change")?;
    if request.status == ExecutionStatus::Succeeded && request.result_id.is_none() {
        return Err("a succeeded execution requires result_id".to_owned());
    }
    for (value, field) in [
        (&request.result_id, "result_id"),
        (&request.completed_at, "completed_at"),
        (&request.model, "model"),
        (&request.thread_id, "thread_id"),
        (&request.error_code, "error_code"),
    ] {
        optional_text(value.clone(), field)?;
    }
    if let Some(result_id) = &request.result_id {
        validate_record_id(result_id, "result")?;
    }
    if request
        .cost_usd
        .is_some_and(|cost| !cost.is_finite() || cost < 0.0)
    {
        return Err("cost_usd must be a finite non-negative number".to_owned());
    }
    Ok(())
}

fn required_text<'a>(value: &'a str, field: &str) -> Result<&'a str, String> {
    if value.trim().is_empty() {
        Err(format!("{field} must not be empty"))
    } else {
        Ok(value)
    }
}

fn optional_text(value: Option<String>, field: &str) -> Result<Option<String>, String> {
    match value {
        Some(value) if value.trim().is_empty() => Err(format!("{field} must not be empty")),
        other => Ok(other),
    }
}

fn validate_execution_id(value: &str) -> Result<(), String> {
    validate_record_id(value, "execution")
}

fn validate_record_id(value: &str, prefix: &str) -> Result<(), String> {
    let expected = format!("{prefix}.");
    if !value.starts_with(&expected)
        || value.len() == expected.len()
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return Err(format!("invalid {prefix} id: {value}"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn completion(status: ExecutionStatus, result_id: Option<&str>) -> CompleteExecutionRequest {
        CompleteExecutionRequest {
            change_id: "change.test".to_owned(),
            status,
            result_id: result_id.map(str::to_owned),
            completed_at: None,
            duration_ms: Some(10),
            model: None,
            input_tokens: Some(5),
            cache_creation_input_tokens: Some(4),
            cached_input_tokens: Some(3),
            output_tokens: Some(2),
            reasoning_output_tokens: Some(1),
            cost_usd: Some(0.25),
            tool_calls: Some(1),
            retries: Some(0),
            thread_id: None,
            exit_code: Some(0),
            error_code: None,
        }
    }

    #[test]
    fn successful_completion_requires_a_result() {
        let request = completion(ExecutionStatus::Succeeded, None);
        assert_eq!(
            validate_completion(&request).unwrap_err(),
            "a succeeded execution requires result_id"
        );
    }

    #[test]
    fn completion_rejects_invalid_cost() {
        let mut request = completion(ExecutionStatus::Failed, None);
        request.cost_usd = Some(-0.01);
        assert_eq!(
            validate_completion(&request).unwrap_err(),
            "cost_usd must be a finite non-negative number"
        );
    }

    #[test]
    fn start_and_completion_are_append_only_and_idempotent() {
        let root = std::env::temp_dir().join(format!(
            "adf-execution-record-test-{}-{}",
            std::process::id(),
            EXECUTION_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(root.join(".adf/changes/change.test")).unwrap();
        let store = ExecutionEventStore::open(&root).unwrap();
        let key = IssuedActionKey {
            change_id: "change.test".to_owned(),
            action_id: "action.test".to_owned(),
            context_digest: format!("sha256:{}", "a".repeat(64)),
        };
        let started = store
            .begin(
                &key,
                "Challenger",
                "result.challenge",
                42,
                BeginExecutionRequest {
                    runner: RunnerIdentity {
                        provider: "openai".to_owned(),
                        surface: "codex-exec".to_owned(),
                        version: None,
                    },
                    started_at: None,
                },
            )
            .unwrap();
        let request = completion(ExecutionStatus::Succeeded, Some("result.test"));
        assert!(
            !store
                .complete(&started.execution_id, request.clone())
                .unwrap()
                .1
        );
        assert!(store.complete(&started.execution_id, request).unwrap().1);
        assert_eq!(store.events("change.test").unwrap().len(), 2);

        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn execution_directory_symlink_is_rejected() {
        use std::os::unix::fs::symlink;

        let root = std::env::temp_dir().join(format!(
            "adf-execution-symlink-test-{}-{}",
            std::process::id(),
            EXECUTION_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        let outside = root.with_extension("outside");
        fs::create_dir_all(root.join(".adf/changes/change.test")).unwrap();
        fs::create_dir_all(&outside).unwrap();
        symlink(&outside, root.join(".adf/changes/change.test/executions")).unwrap();
        let store = ExecutionEventStore::open(&root).unwrap();
        assert!(
            store
                .events("change.test")
                .unwrap_err()
                .contains("not a regular directory")
        );

        fs::remove_dir_all(root).unwrap();
        fs::remove_dir_all(outside).unwrap();
    }
}
