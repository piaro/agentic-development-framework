//! Validate an independently attested Framework distribution trust bundle.
//!
//! The JSON file is transported with a Release, but it is not trusted merely
//! because it is a Release asset. Bootstrap verifies its GitHub Artifact
//! Attestation before the binary persists it. These checks then bind its
//! signer key and source policy to the candidate Framework lock.

use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::fmt;
use std::fs;
use std::path::Path;

pub const DISTRIBUTION_TRUST_FILE: &str = "distribution-trust.json";
pub const DISTRIBUTION_TRUST_SCHEMA_VERSION: &str = "1";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DistributionTrust {
    schema_version: String,
    release_id: String,
    keys: Vec<DistributionTrustKey>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DistributionTrustKey {
    id: String,
    algorithm: String,
    public_key: String,
    allowed_sources: Vec<String>,
    status: String,
}

pub fn trust_store_for_lock(
    path: &Path,
    framework_lock: &Value,
) -> Result<Value, DistributionTrustError> {
    let release_id = lock_string(framework_lock, &["framework_release"])?;
    let signer_key_id = lock_string(framework_lock, &["release_artifact", "signer_key_id"])?;
    let source_id = lock_string(framework_lock, &["release_artifact", "source_id"])?;
    let trust = read_and_validate(path)?;
    validate_binding(&trust, release_id, signer_key_id, source_id, None)?;
    Ok(json!({
        "schema_version": "2",
        "keys": trust.keys.into_iter().map(|key| json!({
            "id": key.id,
            "algorithm": key.algorithm,
            "public_key": key.public_key,
            "allowed_sources": key.allowed_sources,
            "status": key.status,
        })).collect::<Vec<_>>(),
    }))
}

pub fn validate_distribution_binding(
    path: &Path,
    release_id: &str,
    signer_key_id: &str,
    source_id: &str,
    expected_public_key: &str,
) -> Result<(), DistributionTrustError> {
    let trust = read_and_validate(path)?;
    validate_binding(
        &trust,
        release_id,
        signer_key_id,
        source_id,
        Some(expected_public_key),
    )
}

fn read_and_validate(path: &Path) -> Result<DistributionTrust, DistributionTrustError> {
    let bytes =
        fs::read(path).map_err(|error| trust_error(format!("{}: {error}", path.display())))?;
    let trust: DistributionTrust = serde_json::from_slice(&bytes)
        .map_err(|error| trust_error(format!("{}: {error}", path.display())))?;
    if trust.schema_version != DISTRIBUTION_TRUST_SCHEMA_VERSION {
        return Err(trust_error("unsupported distribution trust Schema"));
    }
    if !safe_id(&trust.release_id) {
        return Err(trust_error("distribution trust release_id is invalid"));
    }
    if trust.keys.is_empty() {
        return Err(trust_error(
            "distribution trust must contain at least one key",
        ));
    }
    let mut key_ids = BTreeSet::new();
    for key in &trust.keys {
        if !key_ids.insert(key.id.as_str()) || key.id.is_empty() {
            return Err(trust_error(
                "distribution trust key IDs must be non-empty and unique",
            ));
        }
        if key.algorithm != "ed25519" {
            return Err(trust_error(
                "distribution trust key algorithm must be ed25519",
            ));
        }
        if key.public_key.len() != 64
            || !key
                .public_key
                .bytes()
                .all(|value| value.is_ascii_hexdigit() && !value.is_ascii_uppercase())
        {
            return Err(trust_error(
                "distribution trust public key must be 64 lowercase hexadecimal characters",
            ));
        }
        if key.allowed_sources.is_empty()
            || key.allowed_sources.iter().any(|source| source.is_empty())
            || key.allowed_sources.iter().collect::<BTreeSet<_>>().len()
                != key.allowed_sources.len()
        {
            return Err(trust_error(
                "distribution trust allowed_sources must be non-empty and unique",
            ));
        }
        if !matches!(key.status.as_str(), "active" | "retired" | "revoked") {
            return Err(trust_error("distribution trust key status is invalid"));
        }
    }
    Ok(trust)
}

fn validate_binding(
    trust: &DistributionTrust,
    release_id: &str,
    signer_key_id: &str,
    source_id: &str,
    expected_public_key: Option<&str>,
) -> Result<(), DistributionTrustError> {
    if trust.release_id != release_id {
        return Err(trust_error(format!(
            "distribution trust Release mismatch: expected {release_id:?}, got {:?}",
            trust.release_id
        )));
    }
    let key = trust
        .keys
        .iter()
        .find(|key| key.id == signer_key_id)
        .ok_or_else(|| {
            trust_error(format!(
                "distribution trust does not contain signer key {signer_key_id:?}"
            ))
        })?;
    if !key.allowed_sources.iter().any(|source| source == source_id) {
        return Err(trust_error(format!(
            "distribution trust key {signer_key_id:?} does not allow source {source_id:?}"
        )));
    }
    if key.status != "active" {
        return Err(trust_error(format!(
            "distribution trust signer key {signer_key_id:?} is not active"
        )));
    }
    if expected_public_key.is_some_and(|expected| key.public_key != expected) {
        return Err(trust_error(
            "distribution trust public key does not match Publication Record",
        ));
    }
    Ok(())
}

fn lock_string<'a>(value: &'a Value, path: &[&str]) -> Result<&'a str, DistributionTrustError> {
    let mut current = value;
    for field in path {
        current = current
            .get(*field)
            .ok_or_else(|| trust_error(format!("Framework lock field is missing: {field}")))?;
    }
    current
        .as_str()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| trust_error(format!("Framework lock field must be a string: {path:?}")))
}

fn safe_id(value: &str) -> bool {
    let mut bytes = value.bytes();
    bytes
        .next()
        .is_some_and(|value| value.is_ascii_alphanumeric())
        && bytes.all(|value| value.is_ascii_alphanumeric() || matches!(value, b'.' | b'_' | b'-'))
}

fn trust_error(message: impl Into<String>) -> DistributionTrustError {
    DistributionTrustError {
        message: message.into(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DistributionTrustError {
    message: String,
}

impl fmt::Display for DistributionTrustError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for DistributionTrustError {}
