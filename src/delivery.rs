//! Verify, install, and activate Framework Releases.
//!
//! A signed Release is an offline directory bundle. Its manifest signs the
//! complete file inventory, while the project-owned Framework lock pins the
//! artifact digest, logical source, and signer. Network transports can later
//! download into a temporary directory and reuse this same trust boundary.

use crate::canonical_digest;
use crate::framework_detection::FrameworkCatalog;
use crate::framework_lock::validate_framework_lock;
use crate::framework_lock::{FRAMEWORK_LOCK_SCHEMA_VERSION, SIGNED_FRAMEWORK_LOCK_SCHEMA_VERSION};
use crate::rules::compile_rule_index;
use crate::schema::{SCHEMA_BUNDLE_VERSION, SchemaRegistry};
use ed25519_dalek::{Signature, VerifyingKey};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

pub const LEGACY_RELEASE_MANIFEST_SCHEMA_VERSION: &str = "1";
pub const SIGNED_RELEASE_MANIFEST_SCHEMA_VERSION: &str = "2";
pub const LEGACY_TRUST_STORE_SCHEMA_VERSION: &str = "1";
pub const TRUST_STORE_SCHEMA_VERSION: &str = "2";
pub const TRUST_STORE_PATH: &str = ".adf/trusted-release-keys.yaml";

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub struct VerifiedRelease {
    pub release_id: String,
    pub root: PathBuf,
    pub rule_source: Value,
    pub schema_registry: SchemaRegistry,
    pub framework_catalog: FrameworkCatalog,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallReceipt {
    pub release_id: String,
    pub installed_root: PathBuf,
    pub already_installed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SwitchReceipt {
    pub release_id: String,
    pub backup_path: PathBuf,
}

struct VerifiedManifest {
    rules: String,
    schemas: String,
    framework_catalog: Option<String>,
}

#[derive(Clone, Copy)]
enum TrustUse {
    Runtime,
    NewActivation,
    Rollback,
}

/// Resolve either an explicitly supplied offline Release or the project cache.
pub fn resolve_verified_release(
    project_root: &Path,
    framework_lock: &Value,
    explicit_root: Option<&Path>,
) -> Result<VerifiedRelease, DeliveryError> {
    resolve_verified_release_for(
        project_root,
        framework_lock,
        explicit_root,
        TrustUse::Runtime,
    )
}

/// Verify a Release selected for a new Project activation. Unlike runtime
/// resolution, retired signing keys are rejected.
pub fn resolve_verified_release_for_activation(
    project_root: &Path,
    framework_lock: &Value,
    explicit_root: Option<&Path>,
) -> Result<VerifiedRelease, DeliveryError> {
    resolve_verified_release_for(
        project_root,
        framework_lock,
        explicit_root,
        TrustUse::NewActivation,
    )
}

fn resolve_verified_release_for(
    project_root: &Path,
    framework_lock: &Value,
    explicit_root: Option<&Path>,
    trust_use: TrustUse,
) -> Result<VerifiedRelease, DeliveryError> {
    let release_id = required_lock_string(framework_lock, &["framework_release"])?;
    assert_safe_release_id(release_id)?;
    let release_root = match explicit_root {
        Some(path) if path.is_absolute() => path.to_path_buf(),
        Some(path) => project_root.join(path),
        None => project_root.join(".adf/cache/releases").join(release_id),
    };
    let release_root = release_root.canonicalize().map_err(|error| {
        delivery_error(format!(
            "Framework Release {release_id:?} is not installed at {}: {error}",
            release_root.display()
        ))
    })?;
    if !release_root.is_dir() {
        return Err(delivery_error(format!(
            "Framework Release root is not a directory: {}",
            release_root.display()
        )));
    }

    let manifest = verify_release_manifest(project_root, framework_lock, &release_root, trust_use)?;
    let rules_path = release_asset_path(&release_root, &manifest.rules)?;
    let schemas_path = release_asset_path(&release_root, &manifest.schemas)?;
    let framework_catalog_path = manifest
        .framework_catalog
        .as_deref()
        .map(|path| release_asset_path(&release_root, path))
        .transpose()?;
    if !rules_path.is_file() {
        return Err(delivery_error(format!(
            "Framework Rule source is not a file: {}",
            rules_path.display()
        )));
    }
    if !schemas_path.is_dir() {
        return Err(delivery_error(format!(
            "Framework Schema root is not a directory: {}",
            schemas_path.display()
        )));
    }
    if let Some(path) = &framework_catalog_path
        && !path.is_file()
    {
        return Err(delivery_error(format!(
            "Framework Catalog is not a file: {}",
            path.display()
        )));
    }

    let rule_source = read_yaml(&rules_path)?;
    let actual_rule_digest =
        canonical_digest(&rule_source).map_err(|error| delivery_error(error.to_string()))?;
    let expected_rule_digest =
        required_lock_string(framework_lock, &["rule_set", "source_digest"])?;
    if actual_rule_digest != expected_rule_digest {
        return Err(delivery_error(format!(
            "Framework Rule source digest mismatch: expected {expected_rule_digest}, got {actual_rule_digest}"
        )));
    }

    let expected_schema_version =
        required_lock_string(framework_lock, &["schema_bundle", "version"])?;
    if expected_schema_version != SCHEMA_BUNDLE_VERSION {
        return Err(delivery_error(format!(
            "Framework Schema bundle version mismatch: runtime={}, lock={expected_schema_version}",
            SCHEMA_BUNDLE_VERSION
        )));
    }
    let schema_registry =
        SchemaRegistry::load(&schemas_path).map_err(|error| delivery_error(error.to_string()))?;
    let expected_schema_digest =
        required_lock_string(framework_lock, &["schema_bundle", "digest"])?;
    if schema_registry.digest() != expected_schema_digest {
        return Err(delivery_error(format!(
            "Framework Schema bundle digest mismatch: expected {expected_schema_digest}, got {}",
            schema_registry.digest()
        )));
    }
    let framework_catalog = match framework_catalog_path {
        Some(path) => {
            let source = read_yaml(&path)?;
            FrameworkCatalog::with_release_source(&source)
                .map_err(|error| delivery_error(error.to_string()))?
        }
        None => FrameworkCatalog::default(),
    };

    Ok(VerifiedRelease {
        release_id: release_id.to_owned(),
        root: release_root,
        rule_source,
        schema_registry,
        framework_catalog,
    })
}

/// Verify an offline bundle and atomically place it in the immutable cache.
pub fn install_release(
    project_root: &Path,
    framework_lock: &Value,
    bundle_root: &Path,
) -> Result<InstallReceipt, DeliveryError> {
    let project_root = canonical_project_root(project_root)?;
    let verified = resolve_verified_release_for(
        &project_root,
        framework_lock,
        Some(bundle_root),
        TrustUse::NewActivation,
    )?;
    let releases_root = project_root.join(".adf/cache/releases");
    fs::create_dir_all(&releases_root).map_err(|error| {
        delivery_error(format!(
            "cannot create {}: {error}",
            releases_root.display()
        ))
    })?;
    let releases_root = releases_root.canonicalize().map_err(|error| {
        delivery_error(format!(
            "cannot resolve {}: {error}",
            releases_root.display()
        ))
    })?;
    if !releases_root.starts_with(&project_root) {
        return Err(delivery_error(
            "Framework Release cache resolves outside the project root",
        ));
    }
    let target = releases_root.join(&verified.release_id);
    if target.exists() {
        if fs::symlink_metadata(&target)
            .map_err(|error| delivery_error(format!("{}: {error}", target.display())))?
            .file_type()
            .is_symlink()
        {
            return Err(delivery_error(format!(
                "installed Framework Release must not be a symlink: {}",
                target.display()
            )));
        }
        resolve_verified_release_for(
            &project_root,
            framework_lock,
            Some(&target),
            TrustUse::NewActivation,
        )?;
        return Ok(InstallReceipt {
            release_id: verified.release_id,
            installed_root: target,
            already_installed: true,
        });
    }

    let staging = releases_root.join(format!(
        ".{}.install-{}-{}",
        verified.release_id,
        std::process::id(),
        TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    let result = (|| {
        copy_release_tree(&verified.root, &staging)?;
        // Verify the copied bytes before the atomic directory rename.
        resolve_verified_release_for(
            &project_root,
            framework_lock,
            Some(&staging),
            TrustUse::NewActivation,
        )?;
        fs::rename(&staging, &target).map_err(|error| {
            delivery_error(format!(
                "cannot atomically install Framework Release at {}: {error}",
                target.display()
            ))
        })?;
        Ok(InstallReceipt {
            release_id: verified.release_id,
            installed_root: target,
            already_installed: false,
        })
    })();
    if result.is_err() && staging.exists() {
        let _ = fs::remove_dir_all(&staging);
    }
    result
}

/// Validate an installed Release before atomically replacing Framework lock.
pub fn switch_framework_lock(
    project_root: &Path,
    candidate_lock_path: &Path,
) -> Result<SwitchReceipt, DeliveryError> {
    switch_framework_lock_for(project_root, candidate_lock_path, TrustUse::NewActivation)
}

fn switch_framework_lock_for(
    project_root: &Path,
    candidate_lock_path: &Path,
    trust_use: TrustUse,
) -> Result<SwitchReceipt, DeliveryError> {
    let project_root = canonical_project_root(project_root)?;
    let candidate_path = absolute_from(&project_root, candidate_lock_path);
    let candidate_text = fs::read_to_string(&candidate_path)
        .map_err(|error| delivery_error(format!("{}: {error}", candidate_path.display())))?;
    let candidate_lock: Value = serde_yaml::from_str(&candidate_text)
        .map_err(|error| delivery_error(format!("{}: {error}", candidate_path.display())))?;
    let verified = resolve_verified_release_for(&project_root, &candidate_lock, None, trust_use)?;
    let rule_index = compile_rule_index(&verified.rule_source, &verified.schema_registry)
        .map_err(|error| delivery_error(error.to_string()))?;
    validate_framework_lock(
        &candidate_lock,
        &verified.rule_source,
        &rule_index,
        &verified.schema_registry,
    )
    .map_err(|error| delivery_error(error.to_string()))?;

    let active_path = project_root.join(".adf/framework.lock");
    let active_text = fs::read_to_string(&active_path)
        .map_err(|error| delivery_error(format!("{}: {error}", active_path.display())))?;
    let active_lock: Value = serde_yaml::from_str(&active_text)
        .map_err(|error| delivery_error(format!("{}: {error}", active_path.display())))?;
    let active_digest =
        canonical_digest(&active_lock).map_err(|error| delivery_error(error.to_string()))?;
    let backups_root = project_root.join(".adf/cache/framework-lock-backups");
    fs::create_dir_all(&backups_root).map_err(|error| {
        delivery_error(format!("cannot create {}: {error}", backups_root.display()))
    })?;
    let backup_path = backups_root.join(format!(
        "{}.yaml",
        active_digest
            .strip_prefix("sha256:")
            .expect("canonical digests always use sha256")
    ));
    if backup_path.exists() {
        let existing = fs::read_to_string(&backup_path)
            .map_err(|error| delivery_error(format!("{}: {error}", backup_path.display())))?;
        if existing != active_text {
            return Err(delivery_error(format!(
                "Framework lock backup collision at {}",
                backup_path.display()
            )));
        }
    } else {
        atomic_write(&backup_path, active_text.as_bytes())?;
    }
    atomic_write(&active_path, candidate_text.as_bytes())?;
    Ok(SwitchReceipt {
        release_id: verified.release_id,
        backup_path,
    })
}

/// Restore a backup created by `switch_framework_lock`.
pub fn rollback_framework_lock(
    project_root: &Path,
    backup_lock_path: &Path,
) -> Result<SwitchReceipt, DeliveryError> {
    let project_root = canonical_project_root(project_root)?;
    let backups_root = project_root.join(".adf/cache/framework-lock-backups");
    let backup_path = absolute_from(&project_root, backup_lock_path)
        .canonicalize()
        .map_err(|error| delivery_error(format!("{}: {error}", backup_lock_path.display())))?;
    let canonical_backups = backups_root
        .canonicalize()
        .map_err(|error| delivery_error(format!("{}: {error}", backups_root.display())))?;
    if !backup_path.starts_with(&canonical_backups) || !backup_path.is_file() {
        return Err(delivery_error(
            "rollback lock must be a Framework lock backup created by this project",
        ));
    }
    switch_framework_lock_for(&project_root, &backup_path, TrustUse::Rollback)
}

pub fn read_framework_lock(path: &Path) -> Result<Value, DeliveryError> {
    read_yaml(path)
}

fn verify_release_manifest(
    project_root: &Path,
    framework_lock: &Value,
    release_root: &Path,
    trust_use: TrustUse,
) -> Result<VerifiedManifest, DeliveryError> {
    let release_id = required_lock_string(framework_lock, &["framework_release"])?;
    let manifest_path = release_root.join("release.yaml");
    let manifest = read_yaml(&manifest_path)?;
    let object = manifest
        .as_object()
        .ok_or_else(|| delivery_error("Framework Release manifest must be a mapping"))?;
    let schema_version = required_string(object, "schema_version", "Framework Release manifest")?;
    if object.get("release_id").and_then(Value::as_str) != Some(release_id) {
        return Err(delivery_error(format!(
            "Framework Release ID mismatch: lock={release_id:?}, manifest={}",
            object.get("release_id").unwrap_or(&Value::Null)
        )));
    }
    let assets = object
        .get("assets")
        .and_then(Value::as_object)
        .ok_or_else(|| delivery_error("Framework Release assets must be a mapping"))?;
    match schema_version {
        LEGACY_RELEASE_MANIFEST_SCHEMA_VERSION => {
            if framework_lock.get("schema_version").and_then(Value::as_str)
                != Some(FRAMEWORK_LOCK_SCHEMA_VERSION)
            {
                return Err(delivery_error(
                    "signed Framework lock requires a signed Release manifest",
                ));
            }
            assert_exact_fields(
                object,
                &["schema_version", "release_id", "assets"],
                "Framework Release manifest",
            )?;
            assert_exact_fields(assets, &["rules", "schemas"], "Framework Release assets")?;
        }
        SIGNED_RELEASE_MANIFEST_SCHEMA_VERSION => {
            assert_optional_framework_catalog_asset(assets)?;
            verify_signed_manifest(
                project_root,
                framework_lock,
                release_root,
                object,
                trust_use,
            )?;
        }
        other => {
            return Err(delivery_error(format!(
                "unsupported Framework Release manifest Schema: {other}"
            )));
        }
    }
    Ok(VerifiedManifest {
        rules: required_string(assets, "rules", "Framework Release assets")?.to_owned(),
        schemas: required_string(assets, "schemas", "Framework Release assets")?.to_owned(),
        framework_catalog: assets
            .get("framework_catalog")
            .map(|_| {
                required_string(assets, "framework_catalog", "Framework Release assets")
                    .map(str::to_owned)
            })
            .transpose()?,
    })
}

fn assert_optional_framework_catalog_asset(
    assets: &Map<String, Value>,
) -> Result<(), DeliveryError> {
    let allowed = ["framework_catalog", "rules", "schemas"]
        .into_iter()
        .collect::<BTreeSet<_>>();
    let actual = assets.keys().map(String::as_str).collect::<BTreeSet<_>>();
    if !actual.contains("rules") || !actual.contains("schemas") || !actual.is_subset(&allowed) {
        return Err(delivery_error(format!(
            "signed Framework Release assets fields must be rules, schemas, and optional framework_catalog; got {actual:?}"
        )));
    }
    Ok(())
}

fn verify_signed_manifest(
    project_root: &Path,
    framework_lock: &Value,
    release_root: &Path,
    manifest: &Map<String, Value>,
    trust_use: TrustUse,
) -> Result<(), DeliveryError> {
    assert_exact_fields(
        manifest,
        &[
            "schema_version",
            "release_id",
            "source_id",
            "assets",
            "files",
            "signer",
            "signature",
        ],
        "signed Framework Release manifest",
    )?;
    let source_id = required_string(manifest, "source_id", "Framework Release manifest")?;
    let signer = manifest
        .get("signer")
        .and_then(Value::as_object)
        .ok_or_else(|| delivery_error("Framework Release signer must be a mapping"))?;
    assert_exact_fields(signer, &["algorithm", "key_id"], "Framework Release signer")?;
    if required_string(signer, "algorithm", "Framework Release signer")? != "ed25519" {
        return Err(delivery_error(
            "Framework Release signer.algorithm must be ed25519",
        ));
    }
    let key_id = required_string(signer, "key_id", "Framework Release signer")?;

    let mut payload = Value::Object(manifest.clone());
    payload
        .as_object_mut()
        .expect("payload was created as an object")
        .remove("signature");
    let artifact_digest =
        canonical_digest(&payload).map_err(|error| delivery_error(error.to_string()))?;
    if framework_lock.get("schema_version").and_then(Value::as_str)
        == Some(SIGNED_FRAMEWORK_LOCK_SCHEMA_VERSION)
    {
        let expected_digest =
            required_lock_string(framework_lock, &["release_artifact", "artifact_digest"])?;
        let expected_source =
            required_lock_string(framework_lock, &["release_artifact", "source_id"])?;
        let expected_signer =
            required_lock_string(framework_lock, &["release_artifact", "signer_key_id"])?;
        if artifact_digest != expected_digest {
            return Err(delivery_error(format!(
                "Framework Release artifact digest mismatch: expected {expected_digest}, got {artifact_digest}"
            )));
        }
        if source_id != expected_source {
            return Err(delivery_error(format!(
                "Framework Release source mismatch: expected {expected_source:?}, got {source_id:?}"
            )));
        }
        if key_id != expected_signer {
            return Err(delivery_error(format!(
                "Framework Release signer mismatch: expected {expected_signer:?}, got {key_id:?}"
            )));
        }
    }

    let trust = trusted_key(project_root, key_id, source_id, trust_use)?;
    let signature_text = required_string(manifest, "signature", "Framework Release manifest")?;
    let signature_hex = signature_text
        .strip_prefix("ed25519:")
        .ok_or_else(|| delivery_error("Framework Release signature must start with ed25519:"))?;
    let signature_bytes = decode_hex::<64>(signature_hex, "Framework Release signature")?;
    let signature = Signature::from_bytes(&signature_bytes);
    let payload_bytes =
        crate::canonical_json(&payload).map_err(|error| delivery_error(error.to_string()))?;
    trust
        .verify_strict(payload_bytes.as_bytes(), &signature)
        .map_err(|_| delivery_error("Framework Release signature verification failed"))?;

    verify_file_inventory(release_root, manifest)?;
    Ok(())
}

fn trusted_key(
    project_root: &Path,
    key_id: &str,
    source_id: &str,
    trust_use: TrustUse,
) -> Result<VerifyingKey, DeliveryError> {
    let path = project_root.join(TRUST_STORE_PATH);
    let store = read_yaml(&path)?;
    let object = store
        .as_object()
        .ok_or_else(|| delivery_error("trusted Release key store must be a mapping"))?;
    assert_exact_fields(
        object,
        &["schema_version", "keys"],
        "trusted Release key store",
    )?;
    let schema_version = object
        .get("schema_version")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            delivery_error("trusted Release key store schema_version must be a string")
        })?;
    if !matches!(
        schema_version,
        LEGACY_TRUST_STORE_SCHEMA_VERSION | TRUST_STORE_SCHEMA_VERSION
    ) {
        return Err(delivery_error(format!(
            "unsupported trusted Release key store Schema: {schema_version}"
        )));
    }
    let keys = object
        .get("keys")
        .and_then(Value::as_array)
        .ok_or_else(|| delivery_error("trusted Release key store keys must be an array"))?;
    let mut seen_ids = BTreeSet::new();
    let mut selected = None;
    for key in keys {
        let key = key
            .as_object()
            .ok_or_else(|| delivery_error("trusted Release key entry must be a mapping"))?;
        let fields = if schema_version == TRUST_STORE_SCHEMA_VERSION {
            &["id", "algorithm", "public_key", "allowed_sources", "status"][..]
        } else {
            &["id", "algorithm", "public_key", "allowed_sources"][..]
        };
        assert_exact_fields(key, fields, "trusted Release key entry")?;
        let id = required_string(key, "id", "trusted Release key entry")?;
        if !seen_ids.insert(id.to_owned()) {
            return Err(delivery_error(format!(
                "duplicate trusted Release key ID: {id:?}"
            )));
        }
        if key.get("algorithm").and_then(Value::as_str) != Some("ed25519") {
            return Err(delivery_error(format!(
                "trusted Release key {id:?} does not use ed25519"
            )));
        }
        let allowed = key
            .get("allowed_sources")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                delivery_error("trusted Release key allowed_sources must be an array")
            })?;
        if !allowed.iter().all(Value::is_string) {
            return Err(delivery_error(format!(
                "trusted Release key {id:?} allowed_sources must contain only strings"
            )));
        }
        let unique_sources = allowed
            .iter()
            .filter_map(Value::as_str)
            .collect::<BTreeSet<_>>();
        if unique_sources.is_empty() || unique_sources.len() != allowed.len() {
            return Err(delivery_error(format!(
                "trusted Release key {id:?} allowed_sources must be non-empty and unique"
            )));
        }
        let public_key = key
            .get("public_key")
            .and_then(Value::as_str)
            .ok_or_else(|| delivery_error("trusted Release key public_key must be a string"))?;
        decode_hex::<32>(public_key, "trusted Release public key")?;
        let status = if schema_version == TRUST_STORE_SCHEMA_VERSION {
            required_string(key, "status", "trusted Release key entry")?
        } else {
            "active"
        };
        if !matches!(status, "active" | "retired" | "revoked") {
            return Err(delivery_error(format!(
                "trusted Release key {id:?} has unsupported status {status:?}"
            )));
        }
        if id == key_id {
            selected = Some((
                public_key.to_owned(),
                allowed
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect::<Vec<_>>(),
                status.to_owned(),
            ));
        }
    }
    let Some((public_key, allowed_sources, status)) = selected else {
        return Err(delivery_error(format!(
            "Framework Release signer is not trusted: {key_id:?}"
        )));
    };
    if !allowed_sources.iter().any(|allowed| allowed == source_id) {
        return Err(delivery_error(format!(
            "trusted Release key {key_id:?} does not allow source {source_id:?}"
        )));
    }
    match (status.as_str(), trust_use) {
        ("revoked", _) => {
            return Err(delivery_error(format!(
                "Framework Release signer has been revoked: {key_id:?}"
            )));
        }
        ("retired", TrustUse::NewActivation) => {
            return Err(delivery_error(format!(
                "retired Framework Release signer cannot authorize a new install or switch: {key_id:?}"
            )));
        }
        ("active" | "retired", TrustUse::Runtime | TrustUse::Rollback) | ("active", _) => {}
        _ => unreachable!("key status and trust use were exhaustively validated"),
    }
    let bytes = decode_hex::<32>(&public_key, "trusted Release public key")?;
    VerifyingKey::from_bytes(&bytes)
        .map_err(|_| delivery_error(format!("invalid trusted Release key {key_id:?}")))
}

fn verify_file_inventory(
    release_root: &Path,
    manifest: &Map<String, Value>,
) -> Result<(), DeliveryError> {
    let files = manifest
        .get("files")
        .and_then(Value::as_array)
        .ok_or_else(|| delivery_error("Framework Release files must be an array"))?;
    let mut expected = BTreeMap::new();
    for file in files {
        let file = file
            .as_object()
            .ok_or_else(|| delivery_error("Framework Release file entry must be a mapping"))?;
        assert_exact_fields(file, &["path", "digest"], "Framework Release file entry")?;
        let relative = required_string(file, "path", "Framework Release file entry")?;
        if relative == "release.yaml" {
            return Err(delivery_error(
                "release.yaml must not appear in the signed file inventory",
            ));
        }
        validate_relative_path(relative)?;
        let digest = required_string(file, "digest", "Framework Release file entry")?;
        validate_sha256(digest, "Framework Release file digest")?;
        if expected
            .insert(relative.to_owned(), digest.to_owned())
            .is_some()
        {
            return Err(delivery_error(format!(
                "duplicate Framework Release file path: {relative}"
            )));
        }
    }

    let actual = collect_release_files(release_root)?;
    let actual_paths = actual.keys().cloned().collect::<BTreeSet<_>>();
    let expected_paths = expected.keys().cloned().collect::<BTreeSet<_>>();
    if actual_paths != expected_paths {
        let missing = expected_paths
            .difference(&actual_paths)
            .cloned()
            .collect::<Vec<_>>();
        let unexpected = actual_paths
            .difference(&expected_paths)
            .cloned()
            .collect::<Vec<_>>();
        return Err(delivery_error(format!(
            "Framework Release file inventory mismatch: missing={missing:?}, unexpected={unexpected:?}"
        )));
    }
    for (relative, path) in actual {
        let actual_digest = raw_file_digest(&path)?;
        let expected_digest = &expected[&relative];
        if &actual_digest != expected_digest {
            return Err(delivery_error(format!(
                "Framework Release file digest mismatch for {relative}: expected {expected_digest}, got {actual_digest}"
            )));
        }
    }
    Ok(())
}

fn collect_release_files(root: &Path) -> Result<BTreeMap<String, PathBuf>, DeliveryError> {
    fn visit(
        root: &Path,
        directory: &Path,
        output: &mut BTreeMap<String, PathBuf>,
    ) -> Result<(), DeliveryError> {
        let mut entries = fs::read_dir(directory)
            .map_err(|error| delivery_error(format!("{}: {error}", directory.display())))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| delivery_error(error.to_string()))?;
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let file_type = entry
                .file_type()
                .map_err(|error| delivery_error(error.to_string()))?;
            if file_type.is_symlink() {
                return Err(delivery_error(format!(
                    "Framework Release must not contain symlinks: {}",
                    entry.path().display()
                )));
            }
            if file_type.is_dir() {
                visit(root, &entry.path(), output)?;
            } else if file_type.is_file() {
                let relative = entry
                    .path()
                    .strip_prefix(root)
                    .expect("walked paths are beneath root")
                    .to_string_lossy()
                    .replace('\\', "/");
                if relative != "release.yaml" {
                    output.insert(relative, entry.path());
                }
            } else {
                return Err(delivery_error(format!(
                    "Framework Release contains an unsupported filesystem entry: {}",
                    entry.path().display()
                )));
            }
        }
        Ok(())
    }

    let mut output = BTreeMap::new();
    visit(root, root, &mut output)?;
    Ok(output)
}

fn copy_release_tree(source: &Path, target: &Path) -> Result<(), DeliveryError> {
    if target.exists() {
        return Err(delivery_error(format!(
            "Framework Release staging path already exists: {}",
            target.display()
        )));
    }
    fs::create_dir(target)
        .map_err(|error| delivery_error(format!("{}: {error}", target.display())))?;
    fn copy(source: &Path, target: &Path) -> Result<(), DeliveryError> {
        let mut entries = fs::read_dir(source)
            .map_err(|error| delivery_error(format!("{}: {error}", source.display())))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| delivery_error(error.to_string()))?;
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let kind = entry
                .file_type()
                .map_err(|error| delivery_error(error.to_string()))?;
            let destination = target.join(entry.file_name());
            if kind.is_symlink() {
                return Err(delivery_error(format!(
                    "Framework Release must not contain symlinks: {}",
                    entry.path().display()
                )));
            }
            if kind.is_dir() {
                fs::create_dir(&destination).map_err(|error| {
                    delivery_error(format!("{}: {error}", destination.display()))
                })?;
                copy(&entry.path(), &destination)?;
            } else if kind.is_file() {
                fs::copy(entry.path(), &destination).map_err(|error| {
                    delivery_error(format!("{}: {error}", destination.display()))
                })?;
            } else {
                return Err(delivery_error(format!(
                    "Framework Release contains an unsupported filesystem entry: {}",
                    entry.path().display()
                )));
            }
        }
        Ok(())
    }
    copy(source, target)
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), DeliveryError> {
    let parent = path
        .parent()
        .ok_or_else(|| delivery_error("atomic write target has no parent directory"))?;
    fs::create_dir_all(parent)
        .map_err(|error| delivery_error(format!("{}: {error}", parent.display())))?;
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| delivery_error("atomic write target has an invalid file name"))?;
    let temporary = parent.join(format!(
        ".{file_name}.tmp-{}-{}",
        std::process::id(),
        TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|error| delivery_error(format!("{}: {error}", temporary.display())))?;
        file.write_all(bytes)
            .and_then(|_| file.sync_all())
            .map_err(|error| delivery_error(format!("{}: {error}", temporary.display())))?;
        fs::rename(&temporary, path).map_err(|error| {
            delivery_error(format!(
                "cannot atomically replace {}: {error}",
                path.display()
            ))
        })
    })();
    if result.is_err() && temporary.exists() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn release_asset_path(root: &Path, relative: &str) -> Result<PathBuf, DeliveryError> {
    validate_relative_path(relative)?;
    let candidate = root.join(relative).canonicalize().map_err(|error| {
        delivery_error(format!(
            "cannot resolve Framework Release asset {relative}: {error}"
        ))
    })?;
    if !candidate.starts_with(root) {
        return Err(delivery_error(format!(
            "Framework Release asset path escapes Release root: {relative}"
        )));
    }
    Ok(candidate)
}

fn validate_relative_path(relative: &str) -> Result<(), DeliveryError> {
    let path = Path::new(relative);
    if relative.is_empty() || path.is_absolute() {
        return Err(delivery_error(format!(
            "Framework Release path must be a non-empty relative path: {relative:?}"
        )));
    }
    for component in path.components() {
        if !matches!(component, Component::Normal(_)) {
            return Err(delivery_error(format!(
                "Framework Release asset path escapes Release root: {relative}"
            )));
        }
    }
    Ok(())
}

fn required_lock_string<'a>(value: &'a Value, path: &[&str]) -> Result<&'a str, DeliveryError> {
    let mut current = value;
    for field in path {
        current = current.get(*field).ok_or_else(|| {
            delivery_error(format!(
                "Framework lock field is missing: {}",
                path.join(".")
            ))
        })?;
    }
    current.as_str().ok_or_else(|| {
        delivery_error(format!(
            "Framework lock field must be a string: {}",
            path.join(".")
        ))
    })
}

fn assert_exact_fields(
    object: &Map<String, Value>,
    expected: &[&str],
    label: &str,
) -> Result<(), DeliveryError> {
    let actual: BTreeSet<&str> = object.keys().map(String::as_str).collect();
    let expected: BTreeSet<&str> = expected.iter().copied().collect();
    if actual == expected {
        Ok(())
    } else {
        let missing = expected.difference(&actual).copied().collect::<Vec<_>>();
        let unexpected = actual.difference(&expected).copied().collect::<Vec<_>>();
        Err(delivery_error(format!(
            "invalid {label}: missing={missing:?}, unexpected={unexpected:?}"
        )))
    }
}

fn required_string<'a>(
    object: &'a Map<String, Value>,
    field: &str,
    label: &str,
) -> Result<&'a str, DeliveryError> {
    object
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| delivery_error(format!("{label}.{field} must be a string")))
}

fn read_yaml(path: &Path) -> Result<Value, DeliveryError> {
    let text = fs::read_to_string(path)
        .map_err(|error| delivery_error(format!("{}: {error}", path.display())))?;
    serde_yaml::from_str(&text)
        .map_err(|error| delivery_error(format!("{}: {error}", path.display())))
}

fn canonical_project_root(path: &Path) -> Result<PathBuf, DeliveryError> {
    path.canonicalize()
        .map_err(|error| delivery_error(format!("{}: {error}", path.display())))
}

fn absolute_from(root: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    }
}

fn assert_safe_release_id(value: &str) -> Result<(), DeliveryError> {
    let mut characters = value.chars();
    let safe = characters
        .next()
        .is_some_and(|character| character.is_ascii_alphanumeric())
        && characters.all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-')
        });
    if safe {
        Ok(())
    } else {
        Err(delivery_error(format!(
            "unsafe Framework Release ID: {value:?}"
        )))
    }
}

fn raw_file_digest(path: &Path) -> Result<String, DeliveryError> {
    let bytes =
        fs::read(path).map_err(|error| delivery_error(format!("{}: {error}", path.display())))?;
    let digest = Sha256::digest(bytes);
    Ok(format!("sha256:{digest:x}"))
}

fn validate_sha256(value: &str, label: &str) -> Result<(), DeliveryError> {
    let hex = value
        .strip_prefix("sha256:")
        .ok_or_else(|| delivery_error(format!("{label} must start with sha256:")))?;
    if hex.len() != 64 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(delivery_error(format!(
            "{label} must contain 64 hexadecimal characters"
        )));
    }
    Ok(())
}

fn decode_hex<const N: usize>(value: &str, label: &str) -> Result<[u8; N], DeliveryError> {
    if value.len() != N * 2 {
        return Err(delivery_error(format!(
            "{label} must contain {} hexadecimal characters",
            N * 2
        )));
    }
    let mut output = [0_u8; N];
    for (index, chunk) in value.as_bytes().chunks_exact(2).enumerate() {
        let high = hex_nibble(chunk[0])
            .ok_or_else(|| delivery_error(format!("{label} contains non-hexadecimal data")))?;
        let low = hex_nibble(chunk[1])
            .ok_or_else(|| delivery_error(format!("{label} contains non-hexadecimal data")))?;
        output[index] = (high << 4) | low;
    }
    Ok(output)
}

fn hex_nibble(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

fn delivery_error(message: impl Into<String>) -> DeliveryError {
    DeliveryError {
        message: message.into(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeliveryError {
    message: String,
}

impl fmt::Display for DeliveryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for DeliveryError {}
