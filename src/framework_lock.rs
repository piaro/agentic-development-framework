//! Build and validate the complete technical identity of one Framework runtime.
//!
//! Lock generation records versions and content digests. It does not decide
//! whether a Rule or Schema change is semantically acceptable.

use crate::canonical_digest;
use crate::context::CONTEXT_COMPILER_VERSION;
use crate::detection::{DETECTOR_ID, DETECTOR_VERSION};
use crate::kernel::KERNEL_VERSION;
use crate::project::PROJECT_SNAPSHOT_PROTOCOL_VERSION;
use crate::rules::{RULE_COMPILER_VERSION, RuleIndex};
use crate::schema::{SCHEMA_BUNDLE_VERSION, SchemaRegistry};
use crate::{APPLICATION_PROTOCOL_VERSION, CANONICALIZATION_VERSION};
use serde_json::{Value, json};
use std::fmt;

pub const DATA_MODEL_VERSION: &str = "3";
pub const EXPLANATION_VERSION: &str = "2";
pub const FRAMEWORK_LOCK_SCHEMA_VERSION: &str = "1";
pub const SIGNED_FRAMEWORK_LOCK_SCHEMA_VERSION: &str = "2";
pub const FRAMEWORK_RELEASE: &str = "adf-dev";

#[derive(Debug, Clone, PartialEq)]
pub struct FrameworkLock {
    pub manifest: Value,
    pub digest: String,
}

pub fn build_framework_lock(
    rule_source: &Value,
    rule_index: &RuleIndex,
    schema_registry: &SchemaRegistry,
) -> Result<Value, FrameworkLockError> {
    let source_digest = canonical_digest(rule_source)
        .map_err(|error| FrameworkLockError::new(error.to_string()))?;
    Ok(json!({
        "schema_version": FRAMEWORK_LOCK_SCHEMA_VERSION,
        "framework_release": FRAMEWORK_RELEASE,
        "protocols": {
            "application": APPLICATION_PROTOCOL_VERSION,
            "canonicalization": CANONICALIZATION_VERSION,
            "context_compiler": CONTEXT_COMPILER_VERSION,
            "data_model": DATA_MODEL_VERSION,
            "explanation": EXPLANATION_VERSION,
            "kernel": KERNEL_VERSION,
            "project_snapshot": PROJECT_SNAPSHOT_PROTOCOL_VERSION,
            "rule_compiler": RULE_COMPILER_VERSION,
        },
        "detectors": {
            DETECTOR_ID: DETECTOR_VERSION,
        },
        "schema_bundle": {
            "version": SCHEMA_BUNDLE_VERSION,
            "digest": schema_registry.digest(),
        },
        "rule_set": {
            "source_digest": source_digest,
            "index_digest": rule_index.digest,
        },
    }))
}

pub fn validate_framework_lock(
    lock_source: &Value,
    rule_source: &Value,
    rule_index: &RuleIndex,
    schema_registry: &SchemaRegistry,
) -> Result<FrameworkLock, FrameworkLockError> {
    let expected = build_framework_lock(rule_source, rule_index, schema_registry)?;
    let comparable = comparable_core_lock(lock_source)?;
    let differences = differences(&expected, &comparable, "");
    if !differences.is_empty() {
        return Err(FrameworkLockError::new(format!(
            "framework lock mismatch:\n- {}",
            differences.join("\n- ")
        )));
    }
    let digest = canonical_digest(lock_source)
        .map_err(|error| FrameworkLockError::new(error.to_string()))?;
    Ok(FrameworkLock {
        manifest: lock_source.clone(),
        digest,
    })
}

/// Normalize a signed-delivery v2 lock to the v1 runtime identity.
///
/// Delivery owns signature/source verification. The Application still compares
/// every runtime, Rule, and Schema field exactly, without coupling itself to a
/// transport implementation.
fn comparable_core_lock(lock_source: &Value) -> Result<Value, FrameworkLockError> {
    match lock_source.get("schema_version").and_then(Value::as_str) {
        Some(FRAMEWORK_LOCK_SCHEMA_VERSION) => Ok(lock_source.clone()),
        Some(SIGNED_FRAMEWORK_LOCK_SCHEMA_VERSION) => {
            validate_release_artifact_shape(lock_source)?;
            let mut comparable = lock_source.clone();
            let object = comparable
                .as_object_mut()
                .ok_or_else(|| FrameworkLockError::new("framework lock must be a mapping"))?;
            object.insert(
                "schema_version".to_owned(),
                Value::String(FRAMEWORK_LOCK_SCHEMA_VERSION.to_owned()),
            );
            object.remove("release_artifact");
            Ok(comparable)
        }
        Some(version) => Err(FrameworkLockError::new(format!(
            "unsupported framework lock Schema: {version}"
        ))),
        None => Err(FrameworkLockError::new(
            "framework lock schema_version must be a string",
        )),
    }
}

fn validate_release_artifact_shape(lock_source: &Value) -> Result<(), FrameworkLockError> {
    let artifact = lock_source
        .get("release_artifact")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            FrameworkLockError::new("framework lock v2 release_artifact must be a mapping")
        })?;
    let mut actual = artifact.keys().map(String::as_str).collect::<Vec<_>>();
    actual.sort_unstable();
    let expected = ["artifact_digest", "signer_key_id", "source_id"];
    if actual != expected {
        return Err(FrameworkLockError::new(format!(
            "framework lock v2 release_artifact fields must be exactly {expected:?}"
        )));
    }
    for field in expected {
        let value = artifact
            .get(field)
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                FrameworkLockError::new(format!(
                    "framework lock release_artifact.{field} must be a non-empty string"
                ))
            })?;
        if field == "artifact_digest"
            && !(value.starts_with("sha256:")
                && value.len() == "sha256:".len() + 64
                && value["sha256:".len()..]
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit()))
        {
            return Err(FrameworkLockError::new(
                "framework lock release_artifact.artifact_digest must be a SHA-256 digest",
            ));
        }
    }
    Ok(())
}

fn differences(expected: &Value, actual: &Value, path: &str) -> Vec<String> {
    if let (Some(expected), Some(actual)) = (expected.as_object(), actual.as_object()) {
        let mut keys = expected.keys().chain(actual.keys()).collect::<Vec<_>>();
        keys.sort();
        keys.dedup();
        let mut output = Vec::new();
        for key in keys {
            let child_path = if path.is_empty() {
                key.clone()
            } else {
                format!("{path}.{key}")
            };
            match (expected.get(key), actual.get(key)) {
                (None, Some(_)) => output.push(format!("{child_path}: unexpected field")),
                (Some(_), None) => output.push(format!("{child_path}: missing field")),
                (Some(expected), Some(actual)) => {
                    output.extend(differences(expected, actual, &child_path));
                }
                (None, None) => unreachable!("key came from one of the objects"),
            }
        }
        return output;
    }
    if expected == actual {
        Vec::new()
    } else {
        vec![format!("{path}: expected {expected}, got {actual}")]
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameworkLockError {
    message: String,
}

impl FrameworkLockError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for FrameworkLockError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for FrameworkLockError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_nested_missing_and_unexpected_fields_in_order() {
        assert_eq!(
            differences(
                &json!({"a": {"required": "1"}}),
                &json!({"a": {"unexpected": "1"}}),
                "",
            ),
            vec![
                "a.required: missing field",
                "a.unexpected: unexpected field",
            ]
        );
    }
}
