//! Download remote Release archives into the verified offline install boundary.
//!
//! This module owns transport limits and archive safety only. It never trusts
//! downloaded bytes: the extracted directory must still pass `delivery`'s
//! signature, source, artifact, Rule, and Schema verification.

use crate::delivery::{InstallReceipt, install_release};
use http::Uri;
use serde_json::{Map, Value};
use std::collections::BTreeSet;
use std::fmt;
use std::fs;
use std::fs::OpenOptions;
use std::io::{Cursor, Write};
use std::path::{Component, Path};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

pub const RELEASE_SOURCES_PATH: &str = ".agentic/release-sources.yaml";
pub const RELEASE_SOURCES_SCHEMA_VERSION: &str = "1";
pub const MAX_ARCHIVE_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_EXTRACTED_BYTES: u64 = 256 * 1024 * 1024;
pub const MAX_ARCHIVE_ENTRIES: usize = 4096;

static FETCH_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchReceipt {
    pub artifact_url: String,
    pub install: InstallReceipt,
}

/// Fetch `<base_url>/<release-id>.tar` and reuse the offline installer.
pub fn fetch_release(
    project_root: &Path,
    framework_lock: &Value,
) -> Result<FetchReceipt, RemoteDeliveryError> {
    let project_root = project_root
        .canonicalize()
        .map_err(|error| remote_error(format!("{}: {error}", project_root.display())))?;
    let release_id = required_lock_string(framework_lock, &["framework_release"])?;
    assert_safe_release_id(release_id)?;
    let source_id = required_lock_string(framework_lock, &["release_artifact", "source_id"])?;
    let base_url = configured_base_url(&project_root, source_id)?;
    let artifact_url = artifact_url(&base_url, release_id)?;
    let archive = download_archive(&artifact_url)?;
    let install = install_archive_bytes(&project_root, framework_lock, &archive)?;
    Ok(FetchReceipt {
        artifact_url,
        install,
    })
}

/// Safely extract and install a local tar archive through the same boundary as
/// a remote download. This is used by Release CI and offline distribution.
pub fn install_release_archive(
    project_root: &Path,
    framework_lock: &Value,
    archive_path: &Path,
) -> Result<InstallReceipt, RemoteDeliveryError> {
    let metadata = fs::metadata(archive_path)
        .map_err(|error| remote_error(format!("{}: {error}", archive_path.display())))?;
    if !metadata.is_file() {
        return Err(remote_error(format!(
            "Framework Release archive is not a regular file: {}",
            archive_path.display()
        )));
    }
    if metadata.len() > MAX_ARCHIVE_BYTES as u64 {
        return Err(remote_error(format!(
            "Framework Release archive exceeds the {} byte input limit",
            MAX_ARCHIVE_BYTES
        )));
    }
    let archive = fs::read(archive_path)
        .map_err(|error| remote_error(format!("{}: {error}", archive_path.display())))?;
    install_archive_bytes(project_root, framework_lock, &archive)
}

fn install_archive_bytes(
    project_root: &Path,
    framework_lock: &Value,
    archive: &[u8],
) -> Result<InstallReceipt, RemoteDeliveryError> {
    if archive.len() > MAX_ARCHIVE_BYTES {
        return Err(remote_error(format!(
            "Framework Release archive exceeds the {} byte input limit",
            MAX_ARCHIVE_BYTES
        )));
    }
    let project_root = project_root
        .canonicalize()
        .map_err(|error| remote_error(format!("{}: {error}", project_root.display())))?;
    let fetch_root = project_root.join(".agentic/cache/fetch");
    fs::create_dir_all(&fetch_root).map_err(|error| {
        remote_error(format!("cannot create {}: {error}", fetch_root.display()))
    })?;
    let fetch_root = fetch_root
        .canonicalize()
        .map_err(|error| remote_error(format!("{}: {error}", fetch_root.display())))?;
    if !fetch_root.starts_with(&project_root) {
        return Err(remote_error(
            "Framework Release fetch cache resolves outside the project root",
        ));
    }
    let staging = fetch_root.join(format!(
        ".extract-{}-{}",
        std::process::id(),
        FETCH_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    let result = (|| {
        fs::create_dir(&staging)
            .map_err(|error| remote_error(format!("{}: {error}", staging.display())))?;
        extract_archive(archive, &staging)?;
        install_release(&project_root, framework_lock, &staging)
            .map_err(|error| remote_error(error.to_string()))
    })();
    if staging.exists() {
        let _ = fs::remove_dir_all(&staging);
    }
    result
}

fn configured_base_url(
    project_root: &Path,
    expected_source_id: &str,
) -> Result<String, RemoteDeliveryError> {
    let path = project_root.join(RELEASE_SOURCES_PATH);
    let text = fs::read_to_string(&path)
        .map_err(|error| remote_error(format!("{}: {error}", path.display())))?;
    let value: Value = serde_yaml::from_str(&text)
        .map_err(|error| remote_error(format!("{}: {error}", path.display())))?;
    let object = value
        .as_object()
        .ok_or_else(|| remote_error("Framework Release sources must be a mapping"))?;
    assert_exact_fields(
        object,
        &["schema_version", "sources"],
        "Framework Release sources",
    )?;
    if object.get("schema_version").and_then(Value::as_str) != Some(RELEASE_SOURCES_SCHEMA_VERSION)
    {
        return Err(remote_error("unsupported Framework Release sources Schema"));
    }
    let sources = object
        .get("sources")
        .and_then(Value::as_array)
        .ok_or_else(|| remote_error("Framework Release sources must be an array"))?;
    let mut ids = BTreeSet::new();
    let mut selected = None;
    for source in sources {
        let source = source
            .as_object()
            .ok_or_else(|| remote_error("Framework Release source entry must be a mapping"))?;
        assert_exact_fields(
            source,
            &["id", "base_url"],
            "Framework Release source entry",
        )?;
        let id = required_string(source, "id", "Framework Release source entry")?;
        if !ids.insert(id.to_owned()) {
            return Err(remote_error(format!(
                "duplicate Framework Release source ID: {id:?}"
            )));
        }
        let base_url = required_string(source, "base_url", "Framework Release source entry")?;
        validate_base_url(base_url)?;
        if id == expected_source_id {
            selected = Some(base_url.to_owned());
        }
    }
    selected.ok_or_else(|| {
        remote_error(format!(
            "Framework Release source is not configured: {expected_source_id:?}"
        ))
    })
}

fn validate_base_url(value: &str) -> Result<(), RemoteDeliveryError> {
    let uri: Uri = value
        .parse()
        .map_err(|error| remote_error(format!("invalid Framework Release base URL: {error}")))?;
    if uri.query().is_some() {
        return Err(remote_error(
            "Framework Release base URL must not contain a query",
        ));
    }
    let authority = uri
        .authority()
        .ok_or_else(|| remote_error("Framework Release base URL must have an authority"))?;
    if authority.as_str().contains('@') {
        return Err(remote_error(
            "Framework Release base URL must not contain credentials",
        ));
    }
    let host = uri
        .host()
        .ok_or_else(|| remote_error("Framework Release base URL must have a host"))?;
    match uri.scheme_str() {
        Some("https") => {}
        Some("http") if is_loopback_host(host) => {}
        Some("http") => {
            return Err(remote_error(
                "unencrypted HTTP is allowed only for a loopback Release source",
            ));
        }
        _ => {
            return Err(remote_error("Framework Release base URL must use HTTPS"));
        }
    }
    Ok(())
}

fn artifact_url(base_url: &str, release_id: &str) -> Result<String, RemoteDeliveryError> {
    let value = format!("{}/{release_id}.tar", base_url.trim_end_matches('/'));
    validate_base_url(&value)?;
    Ok(value)
}

fn download_archive(url: &str) -> Result<Vec<u8>, RemoteDeliveryError> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(60)))
        .timeout_connect(Some(Duration::from_secs(10)))
        .timeout_recv_body(Some(Duration::from_secs(30)))
        // A redirect could silently change the reviewed source host or scheme.
        .max_redirects(0)
        .user_agent("agentic-vnext-rust/0.1")
        .build()
        .into();
    let mut response = agent
        .get(url)
        .call()
        .map_err(|error| remote_error(format!("Framework Release download failed: {error}")))?;
    if !response.status().is_success() {
        return Err(remote_error(format!(
            "Framework Release download returned HTTP {}",
            response.status()
        )));
    }
    if let Some(length) = response
        .headers()
        .get(http::header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<usize>().ok())
        && length > MAX_ARCHIVE_BYTES
    {
        return Err(remote_error(format!(
            "Framework Release archive exceeds the {} byte download limit",
            MAX_ARCHIVE_BYTES
        )));
    }
    let bytes = response
        .body_mut()
        .with_config()
        .limit((MAX_ARCHIVE_BYTES + 1) as u64)
        .read_to_vec()
        .map_err(|error| remote_error(format!("Framework Release download failed: {error}")))?;
    if bytes.len() > MAX_ARCHIVE_BYTES {
        return Err(remote_error(format!(
            "Framework Release archive exceeds the {} byte download limit",
            MAX_ARCHIVE_BYTES
        )));
    }
    Ok(bytes)
}

fn extract_archive(bytes: &[u8], target: &Path) -> Result<(), RemoteDeliveryError> {
    let mut archive = tar::Archive::new(Cursor::new(bytes));
    let entries = archive
        .entries()
        .map_err(|error| remote_error(format!("invalid Framework Release archive: {error}")))?;
    let mut paths = BTreeSet::new();
    let mut entry_count = 0_usize;
    let mut extracted_bytes = 0_u64;
    for entry in entries {
        entry_count += 1;
        if entry_count > MAX_ARCHIVE_ENTRIES {
            return Err(remote_error(format!(
                "Framework Release archive exceeds the {} entry limit",
                MAX_ARCHIVE_ENTRIES
            )));
        }
        let mut entry = entry
            .map_err(|error| remote_error(format!("invalid Framework Release archive: {error}")))?;
        let entry_type = entry.header().entry_type();
        if !(entry_type.is_file() || entry_type.is_dir()) {
            return Err(remote_error(
                "Framework Release archive may contain only regular files and directories",
            ));
        }
        let path = entry
            .path()
            .map_err(|error| remote_error(format!("invalid archive path: {error}")))?;
        let relative = path
            .to_str()
            .ok_or_else(|| remote_error("Framework Release archive path must be UTF-8"))?
            .to_owned();
        validate_relative_path(&relative)?;
        if !paths.insert(relative.clone()) {
            return Err(remote_error(format!(
                "duplicate Framework Release archive path: {relative}"
            )));
        }
        let destination = target.join(&relative);
        if entry_type.is_dir() {
            fs::create_dir_all(&destination)
                .map_err(|error| remote_error(format!("{}: {error}", destination.display())))?;
            continue;
        }
        let size = entry
            .header()
            .size()
            .map_err(|error| remote_error(format!("invalid archive size: {error}")))?;
        extracted_bytes = extracted_bytes
            .checked_add(size)
            .ok_or_else(|| remote_error("Framework Release extracted size overflow"))?;
        if extracted_bytes > MAX_EXTRACTED_BYTES {
            return Err(remote_error(format!(
                "Framework Release archive exceeds the {} byte extraction limit",
                MAX_EXTRACTED_BYTES
            )));
        }
        let parent = destination
            .parent()
            .expect("validated relative file paths have a parent");
        fs::create_dir_all(parent)
            .map_err(|error| remote_error(format!("{}: {error}", parent.display())))?;
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&destination)
            .map_err(|error| remote_error(format!("{}: {error}", destination.display())))?;
        let copied = std::io::copy(&mut entry, &mut output)
            .map_err(|error| remote_error(format!("{}: {error}", destination.display())))?;
        if copied != size {
            return Err(remote_error(format!(
                "Framework Release archive file size mismatch for {relative}"
            )));
        }
        output
            .flush()
            .and_then(|_| output.sync_all())
            .map_err(|error| remote_error(format!("{}: {error}", destination.display())))?;
    }
    Ok(())
}

fn validate_relative_path(value: &str) -> Result<(), RemoteDeliveryError> {
    let path = Path::new(value);
    if value.is_empty() || path.is_absolute() {
        return Err(remote_error(
            "Framework Release archive paths must be non-empty and relative",
        ));
    }
    if !path
        .components()
        .all(|component| matches!(component, Component::Normal(_)))
    {
        return Err(remote_error(format!(
            "Framework Release archive path escapes extraction root: {value}"
        )));
    }
    Ok(())
}

fn is_loopback_host(value: &str) -> bool {
    matches!(value, "localhost" | "127.0.0.1" | "::1" | "[::1]")
}

fn assert_safe_release_id(value: &str) -> Result<(), RemoteDeliveryError> {
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
        Err(remote_error(format!(
            "unsafe Framework Release ID: {value:?}"
        )))
    }
}

fn required_lock_string<'a>(
    value: &'a Value,
    path: &[&str],
) -> Result<&'a str, RemoteDeliveryError> {
    let mut current = value;
    for field in path {
        current = current.get(*field).ok_or_else(|| {
            remote_error(format!(
                "Framework lock field is missing: {}",
                path.join(".")
            ))
        })?;
    }
    current.as_str().ok_or_else(|| {
        remote_error(format!(
            "Framework lock field must be a string: {}",
            path.join(".")
        ))
    })
}

fn required_string<'a>(
    object: &'a Map<String, Value>,
    field: &str,
    label: &str,
) -> Result<&'a str, RemoteDeliveryError> {
    object
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| remote_error(format!("{label}.{field} must be a non-empty string")))
}

fn assert_exact_fields(
    object: &Map<String, Value>,
    expected: &[&str],
    label: &str,
) -> Result<(), RemoteDeliveryError> {
    let actual = object.keys().map(String::as_str).collect::<BTreeSet<_>>();
    let expected = expected.iter().copied().collect::<BTreeSet<_>>();
    if actual == expected {
        Ok(())
    } else {
        let missing = expected.difference(&actual).copied().collect::<Vec<_>>();
        let unexpected = actual.difference(&expected).copied().collect::<Vec<_>>();
        Err(remote_error(format!(
            "invalid {label}: missing={missing:?}, unexpected={unexpected:?}"
        )))
    }
}

fn remote_error(message: impl Into<String>) -> RemoteDeliveryError {
    RemoteDeliveryError {
        message: message.into(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteDeliveryError {
    message: String,
}

impl fmt::Display for RemoteDeliveryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for RemoteDeliveryError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_sources_require_https_except_for_loopback_tests() {
        assert!(validate_base_url("https://releases.example.test/agentic").is_ok());
        assert!(validate_base_url("http://127.0.0.1:8080/releases").is_ok());
        assert!(validate_base_url("http://releases.example.test/agentic").is_err());
    }

    #[test]
    fn remote_sources_reject_credentials_and_queries() {
        assert!(validate_base_url("https://user@releases.example.test").is_err());
        assert!(validate_base_url("https://releases.example.test?channel=stable").is_err());
    }
}
