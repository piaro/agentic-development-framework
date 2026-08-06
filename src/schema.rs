//! JSON Schema subset shared with the Python prototype.
//!
//! The Framework bundle currently uses only a deliberately small set of
//! keywords. Implementing that set locally keeps validation behavior explicit
//! while the public Schema extension contract is still being designed.

use crate::canonical_digest;
use regex::Regex;
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;
use std::path::Path;

pub const SCHEMA_BUNDLE_VERSION: &str = "3";
const RECORD_KINDS: [&str; 5] = ["change", "contract", "decision", "evidence", "result"];
const RESULT_SCHEMA_IDS: [&str; 7] = [
    "result.analysis",
    "result.build",
    "result.challenge",
    "result.evidence",
    "result.human-answer",
    "result.impact-assessment",
    "result.risk-signal-review",
];

#[derive(Debug)]
pub struct SchemaRegistry {
    record_schemas: BTreeMap<String, Value>,
    result_payload_schemas: BTreeMap<String, Value>,
    digest: String,
}

impl SchemaRegistry {
    pub fn load(schema_root: impl AsRef<Path>) -> Result<Self, SchemaRegistryError> {
        let root = schema_root.as_ref();
        let mut record_schemas = BTreeMap::new();
        for record_kind in RECORD_KINDS {
            let path = root.join(format!("{record_kind}.schema.json"));
            let schema = read_json(&path)?;
            require_object(&schema, &path)?;
            record_schemas.insert(record_kind.to_owned(), schema);
        }

        let payload_root = root.join("result-payloads");
        let mut payload_paths = fs::read_dir(&payload_root)
            .map_err(|error| {
                SchemaRegistryError::Io(format!("{}: {error}", payload_root.display()))
            })?
            .map(|entry| {
                entry
                    .map(|value| value.path())
                    .map_err(|error| SchemaRegistryError::Io(error.to_string()))
            })
            .collect::<Result<Vec<_>, _>>()?;
        payload_paths.retain(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.ends_with(".schema.json"))
        });
        payload_paths.sort();

        let mut result_payload_schemas = BTreeMap::new();
        for path in payload_paths {
            let schema = read_json(&path)?;
            let object = require_object(&schema, &path)?;
            let result_schema = object
                .get("x-result-schema")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    SchemaRegistryError::Bundle(format!(
                        "Result Schema identity is missing: {}",
                        path.display()
                    ))
                })?
                .to_owned();
            let allowed_roles = object
                .get("x-allowed-roles")
                .and_then(Value::as_array)
                .filter(|roles| !roles.is_empty() && roles.iter().all(Value::is_string))
                .ok_or_else(|| {
                    SchemaRegistryError::Bundle(format!(
                        "Result Schema allowed roles are invalid: {}",
                        path.display()
                    ))
                })?;
            if allowed_roles.is_empty() {
                return Err(SchemaRegistryError::Bundle(format!(
                    "Result Schema has no allowed role: {}",
                    path.display()
                )));
            }
            if result_payload_schemas
                .insert(result_schema.clone(), schema)
                .is_some()
            {
                return Err(SchemaRegistryError::Bundle(format!(
                    "duplicate Result Schema: {result_schema}"
                )));
            }
        }

        assert_exact_keys(
            "Record Schema",
            record_schemas.keys().map(String::as_str),
            RECORD_KINDS,
        )?;
        assert_exact_keys(
            "Result payload Schema",
            result_payload_schemas.keys().map(String::as_str),
            RESULT_SCHEMA_IDS,
        )?;

        let bundle = json!({
            "bundle_version": SCHEMA_BUNDLE_VERSION,
            "record_schemas": record_schemas,
            "result_payload_schemas": result_payload_schemas,
        });
        let digest = canonical_digest(&bundle)
            .map_err(|error| SchemaRegistryError::Bundle(error.to_string()))?;
        let record_schemas = object_to_btree(
            bundle["record_schemas"]
                .as_object()
                .expect("bundle record schemas are an object"),
        );
        let result_payload_schemas = object_to_btree(
            bundle["result_payload_schemas"]
                .as_object()
                .expect("bundle Result schemas are an object"),
        );
        Ok(Self {
            record_schemas,
            result_payload_schemas,
            digest,
        })
    }

    pub fn digest(&self) -> &str {
        &self.digest
    }

    pub fn supports_result_schema(&self, result_schema: &str) -> bool {
        self.result_payload_schemas.contains_key(result_schema)
    }

    pub fn supports_result_role(&self, result_schema: &str, role: &str) -> bool {
        self.result_payload_schemas
            .get(result_schema)
            .and_then(|schema| schema["x-allowed-roles"].as_array())
            .is_some_and(|roles| {
                roles
                    .iter()
                    .any(|candidate| candidate.as_str() == Some(role))
            })
    }

    pub fn validate(&self, record_kind: &str, value: &Value) -> Result<(), SchemaValidationError> {
        let schema = self.record_schemas.get(record_kind).ok_or_else(|| {
            SchemaValidationError::new("$", format!("unknown record kind {record_kind:?}"))
        })?;
        validate_value(value, schema, "$")?;
        if record_kind == "result" {
            let result_schema = value["result_schema"]
                .as_str()
                .expect("base Result Schema guarantees a string");
            let payload_schema =
                self.result_payload_schemas
                    .get(result_schema)
                    .ok_or_else(|| {
                        SchemaValidationError::new(
                            "$.result_schema",
                            format!("unsupported Result schema {result_schema:?}"),
                        )
                    })?;
            let role = value["role"]
                .as_str()
                .expect("base Result Schema guarantees a string");
            let role_allowed = payload_schema["x-allowed-roles"]
                .as_array()
                .expect("bundle loading validates allowed roles")
                .iter()
                .any(|candidate| candidate.as_str() == Some(role));
            if !role_allowed {
                return Err(SchemaValidationError::new(
                    "$.role",
                    format!("role {role:?} is not allowed for {result_schema:?}"),
                ));
            }
            validate_value(&value["payload"], payload_schema, "$.payload")?;
        }
        Ok(())
    }

    /// Validate an Agent/Human payload before subtype-specific submission logic.
    pub fn validate_result_payload(
        &self,
        result_schema: &str,
        payload: &Value,
    ) -> Result<(), SchemaValidationError> {
        let schema = self
            .result_payload_schemas
            .get(result_schema)
            .ok_or_else(|| {
                SchemaValidationError::new(
                    "$.result_schema",
                    format!("unsupported Result schema {result_schema:?}"),
                )
            })?;
        validate_value(payload, schema, "$")
    }
}

/// Validate a generated, non-Record document with the supported Schema subset.
pub fn validate_json_document(
    value: &Value,
    document_schema: &Value,
) -> Result<(), SchemaValidationError> {
    validate_value(value, document_schema, "$")
}

fn validate_value(value: &Value, schema: &Value, path: &str) -> Result<(), SchemaValidationError> {
    let schema = schema
        .as_object()
        .ok_or_else(|| SchemaValidationError::new(path, "Schema node must be an object"))?;

    if let Some(alternatives) = schema.get("anyOf").and_then(Value::as_array)
        && !alternatives
            .iter()
            .any(|alternative| validate_value(value, alternative, path).is_ok())
    {
        return Err(SchemaValidationError::new(
            path,
            "must match at least one anyOf alternative",
        ));
    }

    if let Some(expected) = schema.get("const")
        && value != expected
    {
        return Err(SchemaValidationError::new(
            path,
            format!("must equal {expected}"),
        ));
    }
    if let Some(allowed) = schema.get("enum").and_then(Value::as_array)
        && !allowed.contains(value)
    {
        return Err(SchemaValidationError::new(path, "must match enum"));
    }
    if let Some(expected_type) = schema.get("type").and_then(Value::as_str)
        && !matches_type(value, expected_type)
    {
        return Err(SchemaValidationError::new(
            path,
            format!("must be {expected_type}"),
        ));
    }

    if let Some(text) = value.as_str() {
        if let Some(minimum) = schema.get("minLength").and_then(Value::as_u64)
            && text.chars().count() < minimum as usize
        {
            return Err(SchemaValidationError::new(
                path,
                format!("must contain at least {minimum} characters"),
            ));
        }
        if let Some(pattern) = schema.get("pattern").and_then(Value::as_str) {
            let expression = Regex::new(pattern).map_err(|error| {
                SchemaValidationError::new(path, format!("invalid pattern: {error}"))
            })?;
            if !expression.is_match(text) {
                return Err(SchemaValidationError::new(
                    path,
                    format!("must match pattern {pattern:?}"),
                ));
            }
        }
    }

    if let Some(items) = value.as_array() {
        if let Some(minimum) = schema.get("minItems").and_then(Value::as_u64)
            && items.len() < minimum as usize
        {
            return Err(SchemaValidationError::new(
                path,
                format!("must contain at least {minimum} items"),
            ));
        }
        if schema
            .get("uniqueItems")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            let normalized: BTreeSet<String> = items.iter().map(Value::to_string).collect();
            if normalized.len() != items.len() {
                return Err(SchemaValidationError::new(
                    path,
                    "must contain unique items",
                ));
            }
        }
        if let Some(item_schema) = schema.get("items") {
            for (index, item) in items.iter().enumerate() {
                validate_value(item, item_schema, &format!("{path}[{index}]"))?;
            }
        }
    }

    if let Some(object) = value.as_object() {
        if let Some(required) = schema.get("required").and_then(Value::as_array) {
            for field in required.iter().filter_map(Value::as_str) {
                if !object.contains_key(field) {
                    return Err(SchemaValidationError::new(
                        path,
                        format!("missing required field {field:?}"),
                    ));
                }
            }
        }
        let properties = schema
            .get("properties")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        for (field, item) in object {
            let child_path = format!("{path}.{field}");
            if let Some(property_schema) = properties.get(field) {
                validate_value(item, property_schema, &child_path)?;
                continue;
            }
            match schema.get("additionalProperties") {
                Some(Value::Bool(false)) => {
                    return Err(SchemaValidationError::new(child_path, "unexpected field"));
                }
                Some(additional @ Value::Object(_)) => {
                    validate_value(item, additional, &child_path)?;
                }
                _ => {}
            }
        }
    }
    Ok(())
}

fn matches_type(value: &Value, expected: &str) -> bool {
    match expected {
        "object" => value.is_object(),
        "array" => value.is_array(),
        "string" => value.is_string(),
        "integer" => value.as_i64().is_some() || value.as_u64().is_some(),
        "number" => value.is_number(),
        "boolean" => value.is_boolean(),
        "null" => value.is_null(),
        _ => false,
    }
}

fn read_json(path: &Path) -> Result<Value, SchemaRegistryError> {
    let bytes = fs::read(path)
        .map_err(|error| SchemaRegistryError::Io(format!("{}: {error}", path.display())))?;
    serde_json::from_slice(&bytes)
        .map_err(|error| SchemaRegistryError::Json(format!("{}: {error}", path.display())))
}

fn require_object<'a>(
    value: &'a Value,
    path: &Path,
) -> Result<&'a Map<String, Value>, SchemaRegistryError> {
    value.as_object().ok_or_else(|| {
        SchemaRegistryError::Bundle(format!("Schema must be an object: {}", path.display()))
    })
}

fn object_to_btree(object: &Map<String, Value>) -> BTreeMap<String, Value> {
    object
        .iter()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}

fn assert_exact_keys<'a>(
    label: &str,
    actual: impl Iterator<Item = &'a str>,
    expected: impl IntoIterator<Item = &'a str>,
) -> Result<(), SchemaRegistryError> {
    let actual: BTreeSet<&str> = actual.collect();
    let expected: BTreeSet<&str> = expected.into_iter().collect();
    if actual != expected {
        return Err(SchemaRegistryError::Bundle(format!(
            "invalid {label} bundle: expected {expected:?}, got {actual:?}"
        )));
    }
    Ok(())
}

#[derive(Debug)]
pub enum SchemaRegistryError {
    Io(String),
    Json(String),
    Bundle(String),
}

impl fmt::Display for SchemaRegistryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(message) => write!(formatter, "Schema I/O error: {message}"),
            Self::Json(message) => write!(formatter, "Schema JSON error: {message}"),
            Self::Bundle(message) => write!(formatter, "Schema bundle error: {message}"),
        }
    }
}

impl std::error::Error for SchemaRegistryError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaValidationError {
    pub path: String,
    pub reason: String,
}

impl SchemaValidationError {
    fn new(path: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            reason: reason.into(),
        }
    }
}

impl fmt::Display for SchemaValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "Schema validation failed at {}: {}",
            self.path, self.reason
        )
    }
}

impl std::error::Error for SchemaValidationError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_unexpected_object_field() {
        let value = json!({"known": true, "unknown": true});
        let schema = json!({
            "type": "object",
            "properties": {"known": {"type": "boolean"}},
            "additionalProperties": false
        });
        let error = validate_value(&value, &schema, "$").unwrap_err();
        assert_eq!(error.path, "$.unknown");
    }

    #[test]
    fn counts_unicode_characters_for_min_length() {
        let schema = json!({"type": "string", "minLength": 2});
        assert!(validate_value(&json!("注文"), &schema, "$").is_ok());
        assert!(validate_value(&json!("注"), &schema, "$").is_err());
    }
}
