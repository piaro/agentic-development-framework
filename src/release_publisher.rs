//! Build deterministic, signed Framework Release artifacts.
//!
//! Publishing is deliberately separate from delivery verification. The source
//! directory is never modified, existing outputs are never overwritten, and
//! signing key bytes are supplied by the caller rather than stored in Project
//! files or Release manifests.

use crate::canonical_digest;
use crate::framework_detection::FrameworkCatalog;
use crate::framework_lock::{SIGNED_FRAMEWORK_LOCK_SCHEMA_VERSION, validate_framework_lock};
use crate::remote_delivery::{MAX_ARCHIVE_BYTES, MAX_ARCHIVE_ENTRIES};
use crate::rules::compile_rule_index;
use crate::schema::SchemaRegistry;
use ed25519_dalek::{Signer, SigningKey};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::fs::OpenOptions;
use std::io::{Cursor, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

const SIGNING_KEY_ENV: &str = "ADF_RELEASE_SIGNING_KEY_HEX";
const MAX_PORTABLE_TAR_PATH_BYTES: usize = 100;
static PUBLISH_SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub struct PublishOptions<'a> {
    pub source_root: &'a Path,
    pub base_framework_lock: &'a Path,
    pub source_id: &'a str,
    pub signer_key_id: &'a str,
    /// Optional CI pin. A mismatched secret must fail before any output is written.
    pub expected_signer_public_key: Option<&'a str>,
    pub rules_path: &'a str,
    pub schemas_path: &'a str,
    pub framework_catalog_path: Option<&'a str>,
    pub archive_output: &'a Path,
    pub lock_output: &'a Path,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishReceipt {
    pub release_id: String,
    pub artifact_digest: String,
    pub archive_digest: String,
    pub signer_public_key: String,
    pub archive_output: PathBuf,
    pub lock_output: PathBuf,
}

/// Read the fixed CI secret variable without accepting key bytes in CLI args.
pub fn signing_seed_from_environment() -> Result<[u8; 32], PublishError> {
    let value = std::env::var(SIGNING_KEY_ENV).map_err(|_| {
        publish_error(format!(
            "{SIGNING_KEY_ENV} must contain a 64-character Ed25519 seed"
        ))
    })?;
    decode_hex::<32>(&value, SIGNING_KEY_ENV)
}

/// The public key a signing seed corresponds to, as 64 lowercase hex characters.
///
/// Publishing pins the expected public key so a wrong or rotated signing key
/// stops the build rather than producing a release nobody trusts. That pin has
/// to come from somewhere, and deriving it here keeps the seed from ever being
/// handled outside the process that signs with it.
pub fn signer_public_key(mut signing_seed: [u8; 32]) -> String {
    let signing_key = SigningKey::from_bytes(&signing_seed);
    signing_seed.fill(0);
    encode_hex(&signing_key.verifying_key().to_bytes())
}

pub fn publish_release(
    options: &PublishOptions<'_>,
    mut signing_seed: [u8; 32],
) -> Result<PublishReceipt, PublishError> {
    validate_non_empty(options.source_id, "source ID")?;
    validate_non_empty(options.signer_key_id, "signer key ID")?;
    validate_relative_path(options.rules_path)?;
    validate_relative_path(options.schemas_path)?;
    if let Some(path) = options.framework_catalog_path {
        validate_relative_path(path)?;
    }

    let source_root = options
        .source_root
        .canonicalize()
        .map_err(|error| publish_error(format!("{}: {error}", options.source_root.display())))?;
    if !source_root.is_dir() {
        return Err(publish_error(
            "Framework Release source must be a directory",
        ));
    }
    reject_output_inside_source(&source_root, options.archive_output)?;
    reject_output_inside_source(&source_root, options.lock_output)?;
    assert_new_output(options.archive_output)?;
    assert_new_output(options.lock_output)?;

    let base_lock = read_yaml(options.base_framework_lock)?;
    let release_id = required_string(&base_lock, "framework_release", "Framework lock")?.to_owned();
    assert_safe_release_id(&release_id)?;

    let files = collect_source_files(&source_root)?;
    if files.len() + 1 > MAX_ARCHIVE_ENTRIES {
        return Err(publish_error(format!(
            "Framework Release contains more than {} publishable files",
            MAX_ARCHIVE_ENTRIES - 1
        )));
    }
    let rules_file = source_root.join(options.rules_path);
    if files.get(options.rules_path) != Some(&rules_file) {
        return Err(publish_error(format!(
            "Framework Rule source is not a regular source file: {}",
            options.rules_path
        )));
    }
    let schemas_root = source_root.join(options.schemas_path);
    if !schemas_root.is_dir() {
        return Err(publish_error(format!(
            "Framework Schema root is not a directory: {}",
            options.schemas_path
        )));
    }
    if let Some(relative) = options.framework_catalog_path {
        let path = source_root.join(relative);
        if files.get(relative) != Some(&path) {
            return Err(publish_error(format!(
                "Framework Catalog is not a regular source file: {relative}"
            )));
        }
        let source = read_yaml(&path)?;
        FrameworkCatalog::with_release_source(&source)
            .map_err(|error| publish_error(error.to_string()))?;
    }

    let rule_source = read_yaml(&rules_file)?;
    let schema_registry =
        SchemaRegistry::load(&schemas_root).map_err(|error| publish_error(error.to_string()))?;
    let rule_index = compile_rule_index(&rule_source, &schema_registry)
        .map_err(|error| publish_error(error.to_string()))?;
    validate_framework_lock(&base_lock, &rule_source, &rule_index, &schema_registry)
        .map_err(|error| publish_error(error.to_string()))?;

    let inventory = files
        .iter()
        .map(|(relative, path)| {
            Ok(json!({
                "path": relative,
                "digest": raw_file_digest(path)?,
            }))
        })
        .collect::<Result<Vec<_>, PublishError>>()?;
    let mut assets = json!({
        "rules": options.rules_path,
        "schemas": options.schemas_path,
    });
    if let Some(path) = options.framework_catalog_path {
        assets["framework_catalog"] = Value::String(path.to_owned());
    }
    let payload = json!({
        "schema_version": "2",
        "release_id": release_id,
        "source_id": options.source_id,
        "assets": assets,
        "files": inventory,
        "signer": {
            "algorithm": "ed25519",
            "key_id": options.signer_key_id,
        },
    });
    let artifact_digest =
        canonical_digest(&payload).map_err(|error| publish_error(error.to_string()))?;
    let signing_key = SigningKey::from_bytes(&signing_seed);
    signing_seed.fill(0);
    let signer_public_key = encode_hex(&signing_key.verifying_key().to_bytes());
    if let Some(expected) = options.expected_signer_public_key {
        // Decode first so malformed pins are reported separately from a valid
        // but unexpected signing key.
        let expected = encode_hex(&decode_hex::<32>(expected, "expected signer public key")?);
        if signer_public_key != expected {
            return Err(publish_error(
                "signing secret does not match the expected signer public key",
            ));
        }
    }
    let signature = signing_key.sign(
        crate::canonical_json(&payload)
            .map_err(|error| publish_error(error.to_string()))?
            .as_bytes(),
    );
    let mut manifest = payload;
    manifest["signature"] = Value::String(format!("ed25519:{}", encode_hex(&signature.to_bytes())));
    let manifest_bytes = serde_yaml::to_string(&manifest)
        .map_err(|error| publish_error(error.to_string()))?
        .into_bytes();
    let archive_bytes = build_archive(&files, &manifest_bytes)?;
    if archive_bytes.len() > MAX_ARCHIVE_BYTES {
        return Err(publish_error(format!(
            "Framework Release archive exceeds the {} byte fetch limit",
            MAX_ARCHIVE_BYTES
        )));
    }
    let archive_hash = Sha256::digest(&archive_bytes);
    let archive_digest = format!("sha256:{archive_hash:x}");

    let mut candidate_lock = base_lock;
    candidate_lock["schema_version"] =
        Value::String(SIGNED_FRAMEWORK_LOCK_SCHEMA_VERSION.to_owned());
    candidate_lock["release_artifact"] = json!({
        "artifact_digest": artifact_digest,
        "source_id": options.source_id,
        "signer_key_id": options.signer_key_id,
    });
    validate_framework_lock(&candidate_lock, &rule_source, &rule_index, &schema_registry)
        .map_err(|error| publish_error(error.to_string()))?;
    let lock_bytes = serde_yaml::to_string(&candidate_lock)
        .map_err(|error| publish_error(error.to_string()))?
        .into_bytes();

    publish_new_file(options.archive_output, &archive_bytes)?;
    if let Err(error) = publish_new_file(options.lock_output, &lock_bytes) {
        // This archive was created by this call and no lock points to it.
        let _ = fs::remove_file(options.archive_output);
        return Err(error);
    }
    Ok(PublishReceipt {
        release_id,
        artifact_digest,
        archive_digest,
        signer_public_key,
        archive_output: options.archive_output.to_path_buf(),
        lock_output: options.lock_output.to_path_buf(),
    })
}

fn build_archive(
    files: &BTreeMap<String, PathBuf>,
    manifest_bytes: &[u8],
) -> Result<Vec<u8>, PublishError> {
    let mut entries = files
        .iter()
        .map(|(relative, path)| {
            fs::read(path)
                .map(|bytes| (relative.clone(), bytes))
                .map_err(|error| publish_error(format!("{}: {error}", path.display())))
        })
        .collect::<Result<BTreeMap<_, _>, _>>()?;
    entries.insert("release.yaml".to_owned(), manifest_bytes.to_vec());

    let mut output = Vec::new();
    {
        let mut archive = tar::Builder::new(&mut output);
        for (relative, bytes) in entries {
            if relative.len() > MAX_PORTABLE_TAR_PATH_BYTES {
                return Err(publish_error(format!(
                    "Framework Release path is too long for the portable tar profile: {relative}"
                )));
            }
            let mut header = tar::Header::new_gnu();
            header.set_size(bytes.len() as u64);
            header.set_mode(0o644);
            header.set_uid(0);
            header.set_gid(0);
            header.set_mtime(0);
            header.set_cksum();
            archive
                .append_data(&mut header, relative, Cursor::new(bytes))
                .map_err(|error| publish_error(format!("cannot build Release archive: {error}")))?;
        }
        archive
            .finish()
            .map_err(|error| publish_error(format!("cannot finish Release archive: {error}")))?;
    }
    Ok(output)
}

fn collect_source_files(root: &Path) -> Result<BTreeMap<String, PathBuf>, PublishError> {
    fn visit(
        root: &Path,
        directory: &Path,
        output: &mut BTreeMap<String, PathBuf>,
    ) -> Result<(), PublishError> {
        let mut entries = fs::read_dir(directory)
            .map_err(|error| publish_error(format!("{}: {error}", directory.display())))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| publish_error(error.to_string()))?;
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let kind = entry
                .file_type()
                .map_err(|error| publish_error(error.to_string()))?;
            if kind.is_symlink() {
                return Err(publish_error(format!(
                    "Framework Release source must not contain symlinks: {}",
                    entry.path().display()
                )));
            }
            if kind.is_dir() {
                visit(root, &entry.path(), output)?;
            } else if kind.is_file() {
                let relative = entry
                    .path()
                    .strip_prefix(root)
                    .expect("walked source is beneath root")
                    .to_str()
                    .ok_or_else(|| publish_error("Framework Release paths must be UTF-8"))?
                    .replace('\\', "/");
                // A stale manifest is input metadata, not a publishable component.
                if relative != "release.yaml" {
                    validate_relative_path(&relative)?;
                    output.insert(relative, entry.path());
                }
            } else {
                return Err(publish_error(format!(
                    "Framework Release source contains an unsupported entry: {}",
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

fn publish_new_file(path: &Path, bytes: &[u8]) -> Result<(), PublishError> {
    let parent = path
        .parent()
        .ok_or_else(|| publish_error("publish output must have a parent directory"))?;
    fs::create_dir_all(parent)
        .map_err(|error| publish_error(format!("{}: {error}", parent.display())))?;
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| publish_error("publish output has an invalid file name"))?;
    let temporary = parent.join(format!(
        ".{name}.publish-{}-{}",
        std::process::id(),
        PUBLISH_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|error| publish_error(format!("{}: {error}", temporary.display())))?;
        file.write_all(bytes)
            .and_then(|_| file.sync_all())
            .map_err(|error| publish_error(format!("{}: {error}", temporary.display())))?;
        fs::hard_link(&temporary, path).map_err(|error| {
            publish_error(format!(
                "cannot publish new output {}: {error}",
                path.display()
            ))
        })?;
        // The target is complete once the hard link succeeds. Failure to remove
        // the temporary name must not turn a published pair into a half-rollback.
        let _ = fs::remove_file(&temporary);
        Ok(())
    })();
    if temporary.exists() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn assert_new_output(path: &Path) -> Result<(), PublishError> {
    if path.exists() {
        Err(publish_error(format!(
            "publish output already exists: {}",
            path.display()
        )))
    } else {
        Ok(())
    }
}

fn reject_output_inside_source(root: &Path, output: &Path) -> Result<(), PublishError> {
    let absolute = if output.is_absolute() {
        output.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|error| publish_error(error.to_string()))?
            .join(output)
    };
    if absolute.starts_with(root) {
        Err(publish_error(format!(
            "publish output must be outside the Release source: {}",
            output.display()
        )))
    } else {
        Ok(())
    }
}

fn read_yaml(path: &Path) -> Result<Value, PublishError> {
    let text = fs::read_to_string(path)
        .map_err(|error| publish_error(format!("{}: {error}", path.display())))?;
    serde_yaml::from_str(&text)
        .map_err(|error| publish_error(format!("{}: {error}", path.display())))
}

fn raw_file_digest(path: &Path) -> Result<String, PublishError> {
    let bytes =
        fs::read(path).map_err(|error| publish_error(format!("{}: {error}", path.display())))?;
    let digest = Sha256::digest(bytes);
    Ok(format!("sha256:{digest:x}"))
}

fn required_string<'a>(
    value: &'a Value,
    field: &str,
    label: &str,
) -> Result<&'a str, PublishError> {
    value
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| publish_error(format!("{label}.{field} must be a non-empty string")))
}

fn validate_non_empty(value: &str, label: &str) -> Result<(), PublishError> {
    if value.is_empty() {
        Err(publish_error(format!("{label} must not be empty")))
    } else {
        Ok(())
    }
}

fn validate_relative_path(value: &str) -> Result<(), PublishError> {
    let path = Path::new(value);
    if value.is_empty()
        || path.is_absolute()
        || !path
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
    {
        return Err(publish_error(format!(
            "Framework Release path must be relative and must not escape its root: {value:?}"
        )));
    }
    Ok(())
}

fn assert_safe_release_id(value: &str) -> Result<(), PublishError> {
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
        Err(publish_error(format!(
            "unsafe Framework Release ID: {value:?}"
        )))
    }
}

fn decode_hex<const N: usize>(value: &str, label: &str) -> Result<[u8; N], PublishError> {
    if value.len() != N * 2 {
        return Err(publish_error(format!(
            "{label} must contain {} hexadecimal characters",
            N * 2
        )));
    }
    let mut output = [0_u8; N];
    for (index, chunk) in value.as_bytes().chunks_exact(2).enumerate() {
        let high = hex_nibble(chunk[0])
            .ok_or_else(|| publish_error(format!("{label} contains non-hexadecimal data")))?;
        let low = hex_nibble(chunk[1])
            .ok_or_else(|| publish_error(format!("{label} contains non-hexadecimal data")))?;
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

fn encode_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn publish_error(message: impl Into<String>) -> PublishError {
    PublishError {
        message: message.into(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishError {
    message: String,
}

impl fmt::Display for PublishError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for PublishError {}
