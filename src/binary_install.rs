//! Verify and atomically activate prebuilt CLI binaries.
//!
//! Release assets are immutable once copied into `releases/<tag>/`. Updates
//! change only one small activation file containing the current and previous
//! tags, so the running executable is never overwritten. Keeping both tags in
//! one file also makes update and rollback a single filesystem replacement.

use crate::distribution_trust::{DISTRIBUTION_TRUST_FILE, validate_distribution_binding};
use crate::project_setup::{
    CANDIDATE_LOCK_FILE, FRAMEWORK_ARCHIVE_FILE, PUBLICATION_RECORD_FILE, PUBLISH_RECEIPT_FILE,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

const POINTER_SCHEMA_VERSION: &str = "1";
const LAUNCHER_MARKER: &str = "adf-managed-launcher";
const ACTIVATION_FILE: &str = "active";
static INSTALL_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BinaryCandidate {
    pub release_tag: String,
    pub source_revision: String,
    pub target: String,
    pub binary_name: String,
    pub binary_digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BinaryInstallReceipt {
    pub release_tag: String,
    pub current: String,
    pub previous: Option<String>,
    pub already_installed: bool,
    pub launcher: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BinaryInstallStatus {
    pub schema_version: &'static str,
    pub target: String,
    pub current: Option<String>,
    pub previous: Option<String>,
    pub launcher: PathBuf,
}

#[derive(Debug, Default)]
struct Activation {
    current: Option<String>,
    previous: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PublicationRecord {
    schema_version: String,
    release_id: String,
    release_tag: String,
    source_revision: String,
    candidate_workflow_run_id: String,
    source_id: String,
    signer_key_id: String,
    artifact_digest: String,
    archive_digest: String,
    signer_public_key: String,
    asset_digests: BTreeMap<String, String>,
    binary_asset_digests: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct BinaryBuildRecord {
    schema_version: String,
    binary_name: String,
    target: String,
    source_revision: String,
    sha256: String,
    size: u64,
    rustc_version: String,
}

/// Return the exact native target supported by the published binary matrix.
pub fn current_platform_target() -> Result<&'static str, BinaryInstallError> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => Ok("x86_64-unknown-linux-gnu"),
        ("linux", "aarch64") => Ok("aarch64-unknown-linux-gnu"),
        ("macos", "x86_64") => Ok("x86_64-apple-darwin"),
        ("macos", "aarch64") => Ok("aarch64-apple-darwin"),
        ("windows", "x86_64") => Ok("x86_64-pc-windows-msvc"),
        (os, architecture) => Err(binary_error(format!(
            "no published Agentic binary for {os}/{architecture}"
        ))),
    }
}

pub fn published_binary_name(target: &str) -> String {
    let suffix = if target.ends_with("-windows-msvc") {
        ".exe"
    } else {
        ""
    };
    format!("adf-{target}{suffix}")
}

/// Verify the native binary and the complete project bootstrap candidate.
pub fn inspect_binary_candidate(
    candidate_root: &Path,
    expected_tag: &str,
    expected_revision: &str,
) -> Result<BinaryCandidate, BinaryInstallError> {
    validate_tag(expected_tag)?;
    validate_revision(expected_revision)?;
    require_directory(candidate_root)?;
    let root = candidate_root
        .canonicalize()
        .map_err(|error| binary_error(format!("{}: {error}", candidate_root.display())))?;
    if !root.is_dir() {
        return Err(binary_error("binary candidate root must be a directory"));
    }
    let target = current_platform_target()?.to_owned();
    let binary_name = published_binary_name(&target);
    let record_name = format!("{binary_name}.build.json");
    let required = [
        binary_name.as_str(),
        record_name.as_str(),
        "SHA256SUMS",
        PUBLICATION_RECORD_FILE,
        DISTRIBUTION_TRUST_FILE,
        CANDIDATE_LOCK_FILE,
        FRAMEWORK_ARCHIVE_FILE,
        PUBLISH_RECEIPT_FILE,
    ];
    for name in required {
        require_regular_file(&root.join(name))?;
    }

    let publication: PublicationRecord = read_json(&root.join(PUBLICATION_RECORD_FILE))?;
    if publication.schema_version != "1"
        || publication.release_tag != expected_tag
        || publication.source_revision != expected_revision
    {
        return Err(binary_error(
            "Publication Record does not match the requested tag and source revision",
        ));
    }
    if format!("framework-{}", publication.release_id) != expected_tag
        || !publication
            .candidate_workflow_run_id
            .bytes()
            .all(|value| value.is_ascii_digit())
        || publication.candidate_workflow_run_id.is_empty()
        || publication.source_id.is_empty()
        || publication.signer_key_id.is_empty()
        || !is_digest(&publication.artifact_digest)
        || !is_digest(&publication.archive_digest)
        || publication.signer_public_key.len() != 64
        || !publication
            .signer_public_key
            .bytes()
            .all(|value| value.is_ascii_hexdigit() && !value.is_ascii_uppercase())
        || publication.asset_digests.len() != 4
        || publication
            .asset_digests
            .keys()
            .map(String::as_str)
            .collect::<BTreeSet<_>>()
            != BTreeSet::from([
                "candidate-framework.lock",
                "distribution-trust.json",
                "framework-release.tar",
                "publish-receipt.json",
            ])
        || publication
            .asset_digests
            .values()
            .any(|digest| !is_digest(digest))
        || publication
            .binary_asset_digests
            .values()
            .any(|digest| !is_digest(digest))
    {
        return Err(binary_error(
            "Publication Record contains invalid Framework provenance fields",
        ));
    }
    validate_distribution_binding(
        &root.join(DISTRIBUTION_TRUST_FILE),
        &publication.release_id,
        &publication.signer_key_id,
        &publication.source_id,
        &publication.signer_public_key,
    )
    .map_err(|error| binary_error(error.to_string()))?;
    for name in [
        DISTRIBUTION_TRUST_FILE,
        CANDIDATE_LOCK_FILE,
        FRAMEWORK_ARCHIVE_FILE,
        PUBLISH_RECEIPT_FILE,
    ] {
        let actual = raw_digest(&root.join(name))?;
        let expected = publication
            .asset_digests
            .get(name)
            .ok_or_else(|| binary_error(format!("Publication Record does not list {name}")))?;
        if expected != &actual {
            return Err(binary_error(format!(
                "Publication Record digest mismatch for {name}"
            )));
        }
    }

    let binary_path = root.join(&binary_name);
    let record_path = root.join(&record_name);
    let binary_digest = raw_digest(&binary_path)?;
    let record_digest = raw_digest(&record_path)?;
    let sums_digest = raw_digest(&root.join("SHA256SUMS"))?;
    for (name, actual) in [
        (binary_name.as_str(), binary_digest.as_str()),
        (record_name.as_str(), record_digest.as_str()),
        ("SHA256SUMS", sums_digest.as_str()),
    ] {
        let expected = publication
            .binary_asset_digests
            .get(name)
            .ok_or_else(|| binary_error(format!("Publication Record does not list {name}")))?;
        if expected != actual {
            return Err(binary_error(format!(
                "Publication Record digest mismatch for {name}"
            )));
        }
    }

    let build: BinaryBuildRecord = read_json(&record_path)?;
    if build.schema_version != "1"
        || build.binary_name != binary_name
        || build.target != target
        || build.source_revision != expected_revision
        || build.sha256 != binary_digest
        || build.size != file_size(&binary_path)?
        || !build.rustc_version.starts_with("rustc 1.89.0 ")
    {
        return Err(binary_error(
            "Binary Build Record does not match the selected binary",
        ));
    }
    verify_checksum_file(&root.join("SHA256SUMS"), &binary_name, &binary_digest)?;

    Ok(BinaryCandidate {
        release_tag: expected_tag.to_owned(),
        source_revision: expected_revision.to_owned(),
        target,
        binary_name,
        binary_digest,
    })
}

pub fn install_binary_candidate(
    candidate_root: &Path,
    expected_tag: &str,
    expected_revision: &str,
    install_root: &Path,
) -> Result<BinaryInstallReceipt, BinaryInstallError> {
    let candidate = inspect_binary_candidate(candidate_root, expected_tag, expected_revision)?;
    fs::create_dir_all(install_root)
        .map_err(|error| binary_error(format!("{}: {error}", install_root.display())))?;
    let install_root = install_root
        .canonicalize()
        .map_err(|error| binary_error(format!("{}: {error}", install_root.display())))?;
    reject_filesystem_root(&install_root)?;
    // Hold one OS-managed lock across immutable Release installation, launcher
    // validation, and activation so concurrent updater processes cannot race.
    let _install_lock = acquire_install_lock(&install_root)?;
    let candidate_root = candidate_root
        .canonicalize()
        .map_err(|error| binary_error(format!("{}: {error}", candidate_root.display())))?;
    if candidate_root.starts_with(&install_root) {
        return Err(binary_error(
            "binary candidate staging must be outside the installation root",
        ));
    }

    let releases_root = install_root.join("releases");
    fs::create_dir_all(&releases_root)
        .map_err(|error| binary_error(format!("{}: {error}", releases_root.display())))?;
    require_directory(&releases_root)?;
    let target = releases_root.join(&candidate.release_tag);
    let already_installed = target.exists();
    if already_installed {
        inspect_binary_candidate(&target, &candidate.release_tag, &candidate.source_revision)?;
    } else {
        let staging = releases_root.join(format!(
            ".{}.install-{}-{}",
            candidate.release_tag,
            std::process::id(),
            INSTALL_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        let result = (|| {
            fs::create_dir(&staging)
                .map_err(|error| binary_error(format!("{}: {error}", staging.display())))?;
            let record_name = format!("{}.build.json", candidate.binary_name);
            for name in [
                candidate.binary_name.as_str(),
                record_name.as_str(),
                "SHA256SUMS",
                PUBLICATION_RECORD_FILE,
                DISTRIBUTION_TRUST_FILE,
                CANDIDATE_LOCK_FILE,
                FRAMEWORK_ARCHIVE_FILE,
                PUBLISH_RECEIPT_FILE,
            ] {
                copy_new_file(&candidate_root.join(name), &staging.join(name))?;
            }
            inspect_binary_candidate(&staging, &candidate.release_tag, &candidate.source_revision)?;
            fs::rename(&staging, &target).map_err(|error| {
                binary_error(format!(
                    "cannot atomically install binary Release at {}: {error}",
                    target.display()
                ))
            })?;
            sync_directory(&releases_root)
        })();
        if staging.exists() {
            let _ = fs::remove_dir_all(&staging);
        }
        result?;
    }

    let launcher = write_managed_launcher(&install_root, &candidate.binary_name)?;
    let activation_path = install_root.join(ACTIVATION_FILE);
    let activation = read_activation(&activation_path)?;
    let previous = if activation.current.as_deref() != Some(&candidate.release_tag) {
        publish_activation(
            &activation_path,
            &candidate.release_tag,
            activation.current.as_deref(),
        )?;
        activation.current
    } else {
        activation.previous
    };
    Ok(BinaryInstallReceipt {
        release_tag: candidate.release_tag.clone(),
        current: candidate.release_tag,
        previous,
        already_installed,
        launcher,
    })
}

pub fn rollback_binary_install(
    install_root: &Path,
) -> Result<BinaryInstallReceipt, BinaryInstallError> {
    let install_root = install_root
        .canonicalize()
        .map_err(|error| binary_error(format!("{}: {error}", install_root.display())))?;
    reject_filesystem_root(&install_root)?;
    let _install_lock = acquire_install_lock(&install_root)?;
    let activation_path = install_root.join(ACTIVATION_FILE);
    let activation = read_activation(&activation_path)?;
    let current = activation
        .current
        .ok_or_else(|| binary_error("active binary Release is missing"))?;
    let previous = activation
        .previous
        .ok_or_else(|| binary_error("binary rollback point is missing"))?;
    if current == previous {
        return Err(binary_error("binary rollback point is not distinct"));
    }
    let previous_root = install_root.join("releases").join(&previous);
    let publication: PublicationRecord = read_json(&previous_root.join(PUBLICATION_RECORD_FILE))?;
    let verified =
        inspect_binary_candidate(&previous_root, &previous, &publication.source_revision)?;
    publish_activation(&activation_path, &previous, Some(&current))?;
    let launcher = launcher_path(&install_root);
    Ok(BinaryInstallReceipt {
        release_tag: verified.release_tag,
        current: previous,
        previous: Some(current),
        already_installed: true,
        launcher,
    })
}

pub fn binary_install_status(
    install_root: &Path,
) -> Result<BinaryInstallStatus, BinaryInstallError> {
    let target = current_platform_target()?.to_owned();
    let activation = read_activation(&install_root.join(ACTIVATION_FILE))?;
    let current = activation.current;
    let previous = activation.previous;
    if let Some(tag) = current.as_deref() {
        let root = install_root.join("releases").join(tag);
        let publication: PublicationRecord = read_json(&root.join(PUBLICATION_RECORD_FILE))?;
        inspect_binary_candidate(&root, tag, &publication.source_revision)?;
    }
    Ok(BinaryInstallStatus {
        schema_version: POINTER_SCHEMA_VERSION,
        target,
        current,
        previous,
        launcher: launcher_path(install_root),
    })
}

fn write_managed_launcher(
    install_root: &Path,
    binary_name: &str,
) -> Result<PathBuf, BinaryInstallError> {
    let bin = install_root.join("bin");
    fs::create_dir_all(&bin)
        .map_err(|error| binary_error(format!("{}: {error}", bin.display())))?;
    require_directory(&bin)?;
    let path = launcher_path(install_root);
    let content = if cfg!(windows) {
        format!(
            "@echo off\r\nrem {LAUNCHER_MARKER}\r\nset /p ADF_VERSION=<\"%~dp0..\\{ACTIVATION_FILE}\"\r\n\"%~dp0..\\releases\\%ADF_VERSION%\\{binary_name}\" %*\r\n"
        )
    } else {
        format!(
            "#!/bin/sh\n# {LAUNCHER_MARKER}\nset -eu\nBIN_DIR=$(CDPATH= cd -- \"$(dirname -- \"$0\")\" && pwd)\nINSTALL_ROOT=$(CDPATH= cd -- \"$BIN_DIR/..\" && pwd)\nIFS= read -r ADF_VERSION < \"$INSTALL_ROOT/{ACTIVATION_FILE}\"\nexec \"$INSTALL_ROOT/releases/$ADF_VERSION/{binary_name}\" \"$@\"\n"
        )
    };
    if path.exists() {
        let existing = fs::read_to_string(&path)
            .map_err(|error| binary_error(format!("{}: {error}", path.display())))?;
        let managed_prefix = if cfg!(windows) {
            format!("@echo off\r\nrem {LAUNCHER_MARKER}\r\n")
        } else {
            format!("#!/bin/sh\n# {LAUNCHER_MARKER}\n")
        };
        if !existing.starts_with(&managed_prefix) {
            return Err(binary_error(format!(
                "refusing to overwrite an unmanaged launcher: {}",
                path.display()
            )));
        }
        if existing.as_bytes() == content.as_bytes() {
            return Ok(path);
        }
    }
    publish_bytes(&path, content.as_bytes())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755))
            .map_err(|error| binary_error(format!("{}: {error}", path.display())))?;
    }
    Ok(path)
}

fn launcher_path(install_root: &Path) -> PathBuf {
    install_root
        .join("bin")
        .join(if cfg!(windows) { "adf.cmd" } else { "adf" })
}

fn reject_filesystem_root(path: &Path) -> Result<(), BinaryInstallError> {
    if path.parent().is_none() {
        Err(binary_error(
            "installation root must be a dedicated directory, not a filesystem root",
        ))
    } else {
        Ok(())
    }
}

fn verify_checksum_file(
    path: &Path,
    binary_name: &str,
    binary_digest: &str,
) -> Result<(), BinaryInstallError> {
    let text = fs::read_to_string(path)
        .map_err(|error| binary_error(format!("{}: {error}", path.display())))?;
    let mut names = BTreeSet::new();
    let mut selected = None;
    for line in text.lines() {
        let (digest, name) = line
            .split_once("  ")
            .ok_or_else(|| binary_error("SHA256SUMS contains an invalid line"))?;
        if digest.len() != 64 || !digest.bytes().all(|value| value.is_ascii_hexdigit()) {
            return Err(binary_error("SHA256SUMS contains an invalid digest"));
        }
        if !names.insert(name.to_owned()) {
            return Err(binary_error("SHA256SUMS contains a duplicate file"));
        }
        if name == binary_name {
            selected = Some(format!("sha256:{}", digest.to_ascii_lowercase()));
        }
    }
    if selected.as_deref() != Some(binary_digest) {
        return Err(binary_error(
            "SHA256SUMS does not match the selected binary",
        ));
    }
    Ok(())
}

fn read_activation(path: &Path) -> Result<Activation, BinaryInstallError> {
    if !path.exists() {
        return Ok(Activation::default());
    }
    require_regular_file(path)?;
    let value = fs::read_to_string(path)
        .map_err(|error| binary_error(format!("{}: {error}", path.display())))?;
    let mut lines = value.lines();
    let current = lines
        .next()
        .ok_or_else(|| binary_error("binary activation file is empty"))?;
    let previous = lines.next().unwrap_or("");
    if lines.next().is_some() {
        return Err(binary_error("binary activation file has extra lines"));
    }
    validate_tag(current)?;
    if !previous.is_empty() {
        validate_tag(previous)?;
    }
    Ok(Activation {
        current: Some(current.to_owned()),
        previous: (!previous.is_empty()).then(|| previous.to_owned()),
    })
}

fn publish_activation(
    path: &Path,
    current: &str,
    previous: Option<&str>,
) -> Result<(), BinaryInstallError> {
    validate_tag(current)?;
    if let Some(value) = previous {
        validate_tag(value)?;
    }
    publish_bytes(
        path,
        format!("{current}\n{}\n", previous.unwrap_or("")).as_bytes(),
    )
}

fn publish_bytes(path: &Path, bytes: &[u8]) -> Result<(), BinaryInstallError> {
    let parent = path
        .parent()
        .ok_or_else(|| binary_error("managed output has no parent directory"))?;
    fs::create_dir_all(parent)
        .map_err(|error| binary_error(format!("{}: {error}", parent.display())))?;
    let temporary = parent.join(format!(
        ".binary-install-{}-{}",
        std::process::id(),
        INSTALL_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    let result = (|| {
        let mut output = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)
            .map_err(|error| binary_error(format!("{}: {error}", temporary.display())))?;
        output
            .write_all(bytes)
            .and_then(|_| output.sync_all())
            .map_err(|error| binary_error(format!("{}: {error}", temporary.display())))?;
        replace_file(&temporary, path)?;
        sync_directory(parent)
    })();
    if temporary.exists() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn acquire_install_lock(install_root: &Path) -> Result<File, BinaryInstallError> {
    let path = install_root.join(".binary-install.lock");
    if path.exists() {
        require_regular_file(&path)?;
    }
    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&path)
        .map_err(|error| binary_error(format!("{}: {error}", path.display())))?;
    file.try_lock().map_err(|error| {
        binary_error(format!(
            "another binary install or rollback is running for {}: {error}",
            install_root.display()
        ))
    })?;
    Ok(file)
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> Result<(), BinaryInstallError> {
    File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| binary_error(format!("{}: {error}", path.display())))
}

#[cfg(windows)]
fn sync_directory(_path: &Path) -> Result<(), BinaryInstallError> {
    // MoveFileExW with MOVEFILE_WRITE_THROUGH flushes file replacement.
    Ok(())
}

#[cfg(not(windows))]
fn replace_file(source: &Path, target: &Path) -> Result<(), BinaryInstallError> {
    fs::rename(source, target)
        .map_err(|error| binary_error(format!("{}: {error}", target.display())))
}

#[cfg(windows)]
fn replace_file(source: &Path, target: &Path) -> Result<(), BinaryInstallError> {
    use std::os::windows::ffi::OsStrExt;

    // std::fs::rename does not replace an existing destination on Windows.
    // MoveFileExW keeps activation changes to one replace operation and asks
    // Windows to flush the move before returning.
    const MOVEFILE_REPLACE_EXISTING: u32 = 0x1;
    const MOVEFILE_WRITE_THROUGH: u32 = 0x8;
    #[link(name = "Kernel32")]
    unsafe extern "system" {
        fn MoveFileExW(
            existing_file_name: *const u16,
            new_file_name: *const u16,
            flags: u32,
        ) -> i32;
    }

    let source_wide: Vec<u16> = source
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let target_wide: Vec<u16> = target
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    // SAFETY: both pointers reference NUL-terminated buffers that remain alive
    // for the call, and the flags are documented MoveFileExW values.
    let moved = unsafe {
        MoveFileExW(
            source_wide.as_ptr(),
            target_wide.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if moved == 0 {
        Err(binary_error(format!(
            "{}: {}",
            target.display(),
            std::io::Error::last_os_error()
        )))
    } else {
        Ok(())
    }
}

fn copy_new_file(source: &Path, target: &Path) -> Result<(), BinaryInstallError> {
    require_regular_file(source)?;
    let bytes =
        fs::read(source).map_err(|error| binary_error(format!("{}: {error}", source.display())))?;
    let mut output = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(target)
        .map_err(|error| binary_error(format!("{}: {error}", target.display())))?;
    output
        .write_all(&bytes)
        .and_then(|_| output.sync_all())
        .map_err(|error| binary_error(format!("{}: {error}", target.display())))?;
    #[cfg(unix)]
    if source
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with("adf-") && !name.ends_with(".json"))
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(target, fs::Permissions::from_mode(0o755))
            .map_err(|error| binary_error(format!("{}: {error}", target.display())))?;
    }
    Ok(())
}

fn require_regular_file(path: &Path) -> Result<(), BinaryInstallError> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| binary_error(format!("{}: {error}", path.display())))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        Err(binary_error(format!(
            "binary Release asset must be a regular file: {}",
            path.display()
        )))
    } else {
        Ok(())
    }
}

fn require_directory(path: &Path) -> Result<(), BinaryInstallError> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| binary_error(format!("{}: {error}", path.display())))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        Err(binary_error(format!(
            "binary candidate or installation directory must be a real directory: {}",
            path.display()
        )))
    } else {
        Ok(())
    }
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T, BinaryInstallError> {
    let bytes =
        fs::read(path).map_err(|error| binary_error(format!("{}: {error}", path.display())))?;
    serde_json::from_slice(&bytes)
        .map_err(|error| binary_error(format!("{}: {error}", path.display())))
}

fn raw_digest(path: &Path) -> Result<String, BinaryInstallError> {
    let bytes =
        fs::read(path).map_err(|error| binary_error(format!("{}: {error}", path.display())))?;
    Ok(format!("sha256:{:x}", Sha256::digest(bytes)))
}

fn is_digest(value: &str) -> bool {
    value.len() == 71
        && value.starts_with("sha256:")
        && value[7..]
            .bytes()
            .all(|character| character.is_ascii_hexdigit() && !character.is_ascii_uppercase())
}

fn file_size(path: &Path) -> Result<u64, BinaryInstallError> {
    fs::metadata(path)
        .map(|metadata| metadata.len())
        .map_err(|error| binary_error(format!("{}: {error}", path.display())))
}

fn validate_tag(value: &str) -> Result<(), BinaryInstallError> {
    let mut characters = value.chars();
    let safe = value.starts_with("framework-")
        && characters
            .next()
            .is_some_and(|character| character.is_ascii_alphanumeric())
        && characters.all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-')
        });
    if safe {
        Ok(())
    } else {
        Err(binary_error(format!(
            "unsafe binary Release tag: {value:?}"
        )))
    }
}

fn validate_revision(value: &str) -> Result<(), BinaryInstallError> {
    if value.len() == 40
        && value
            .bytes()
            .all(|character| character.is_ascii_hexdigit() && !character.is_ascii_uppercase())
    {
        Ok(())
    } else {
        Err(binary_error(
            "source revision must be a 40-character lowercase hexadecimal Git SHA",
        ))
    }
}

fn binary_error(message: impl Into<String>) -> BinaryInstallError {
    BinaryInstallError {
        message: message.into(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BinaryInstallError {
    message: String,
}

impl fmt::Display for BinaryInstallError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for BinaryInstallError {}
