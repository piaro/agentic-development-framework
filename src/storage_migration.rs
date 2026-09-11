//! Explicit, resumable changes of physical Record representation.
//! No Git index/history changes, implicit upgrades, or logical Record edits.

use crate::canonical_digest;
use crate::delivery::{read_framework_lock, resolve_verified_release};
use crate::project_config::load_project_config;
use crate::record_storage::{self, RecordKind, StoragePolicy};
use crate::schema::SchemaRegistry;
use crate::storage_io::{self, StorageGuard};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const JOURNAL: &str = ".adf/local/storage-migrations/current.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    path: String,
    kind: String,
    id: String,
    before_hash: String,
    after_hash: String,
    logical_digest: String,
    before_bytes: usize,
    after_bytes: usize,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    schema_version: String,
    target: String,
    complete: bool,
    entries: Vec<Entry>,
}

fn hash(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

fn kind(name: &str) -> Result<RecordKind, String> {
    match name {
        "result" => Ok(RecordKind::Result),
        "evidence" => Ok(RecordKind::Evidence),
        _ => Err("unsupported Record kind".to_owned()),
    }
}

fn regular_entries(root: &Path, directory: &Path) -> Result<Vec<PathBuf>, String> {
    storage_io::reject_symlinks(root, directory)?;
    if !directory.exists() {
        return Ok(Vec::new());
    }
    let mut paths = fs::read_dir(directory)
        .map_err(|e| e.to_string())?
        .map(|entry| entry.map(|e| e.path()).map_err(|e| e.to_string()))
        .collect::<Result<Vec<_>, _>>()?;
    paths.sort();
    Ok(paths)
}

fn inventory(root: &Path) -> Result<Vec<(PathBuf, RecordKind)>, String> {
    let mut records = Vec::new();
    for change in regular_entries(root, &root.join(".adf/changes"))? {
        storage_io::reject_symlinks(root, &change)?;
        if !change.is_dir() {
            continue;
        }
        for (name, record_kind) in [
            ("results", RecordKind::Result),
            ("evidence", RecordKind::Evidence),
        ] {
            for path in regular_entries(root, &change.join(name))? {
                storage_io::reject_symlinks(root, &path)?;
                if path.extension().is_some_and(|s| s == "json") {
                    if !path.is_file() {
                        return Err(format!("not a regular Record: {}", path.display()));
                    }
                    records.push((path, record_kind));
                } else if path.is_dir() {
                    return Err(format!(
                        "nested Record directory is not supported: {}",
                        path.display()
                    ));
                }
            }
        }
    }
    Ok(records)
}

fn skipped_files(root: &Path) -> Result<Vec<Value>, String> {
    let mut skipped = Vec::new();
    for change in regular_entries(root, &root.join(".adf/changes"))? {
        if !change.is_dir() {
            continue;
        }
        for name in ["results", "evidence"] {
            for path in regular_entries(root, &change.join(name))? {
                if path.extension().is_none_or(|s| s != "json") {
                    skipped.push(json!({"path":path.strip_prefix(root).map_err(|e| e.to_string())?.to_str().ok_or("non-UTF8 skipped file path")?,
                        "bytes":fs::symlink_metadata(&path).map_err(|e| e.to_string())?.len(),
                        "reason": if path.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with(".adf-storage-")) { "temporary-file-review-required" } else { "not-a-json-record" }}));
                }
            }
        }
    }
    Ok(skipped)
}

fn converted(
    raw: &[u8],
    record_kind: RecordKind,
    target: StoragePolicy,
) -> Result<(Value, Vec<u8>), String> {
    let value = record_storage::decode(raw, record_kind)?;
    let encoded = record_storage::encode(&value, record_kind, target)?;
    let bytes = if target == StoragePolicy::Adaptive && encoded.len() >= raw.len() {
        raw.to_vec()
    } else {
        encoded
    };
    if record_storage::decode(&bytes, record_kind)? != value {
        return Err("Record storage roundtrip mismatch".to_owned());
    }
    Ok((value, bytes))
}

fn scan(
    root: &Path,
    registry: &SchemaRegistry,
    target: StoragePolicy,
) -> Result<Vec<Entry>, String> {
    let mut entries = Vec::new();
    let mut ids = BTreeSet::new();
    for (path, record_kind) in inventory(root)? {
        let raw = storage_io::read_record(&path)?;
        let (value, bytes) =
            converted(&raw, record_kind, target).map_err(|e| format!("{}: {e}", path.display()))?;
        registry
            .validate(record_kind.as_str(), &value)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        let id = value["id"]
            .as_str()
            .ok_or("Record id is missing")?
            .to_owned();
        if !ids.insert(id.clone()) {
            return Err(format!("duplicate Record id: {id}"));
        }
        let change_id = path
            .parent()
            .and_then(Path::parent)
            .and_then(Path::file_name)
            .and_then(|v| v.to_str())
            .ok_or("invalid Record path")?;
        if value["change_id"].as_str() != Some(change_id) {
            return Err(format!(
                "Record change_id disagrees with path: {}",
                path.display()
            ));
        }
        entries.push(Entry {
            path: path
                .strip_prefix(root)
                .map_err(|e| e.to_string())?
                .to_str()
                .ok_or("non-UTF8 Record path")?
                .to_owned(),
            kind: record_kind.as_str().to_owned(),
            id,
            before_hash: hash(&raw),
            after_hash: hash(&bytes),
            logical_digest: canonical_digest(&value).map_err(|e| e.to_string())?,
            before_bytes: raw.len(),
            after_bytes: bytes.len(),
        });
    }
    Ok(entries)
}

fn ignored(root: &Path, relative: &str) -> bool {
    Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["check-ignore", "-q", "--", relative])
        .status()
        .is_ok_and(|status| status.success())
}

fn journal_path(root: &Path) -> Result<PathBuf, String> {
    let path = root.join(JOURNAL);
    storage_io::reject_symlinks(root, &path)?;
    Ok(path)
}

fn validate_pending(root: &Path, entries: &[Entry]) -> Result<(), String> {
    let path = journal_path(root)?;
    if !path.exists() {
        return Ok(());
    }
    let previous: Journal = serde_json::from_value(record_storage::parse_strict(
        &storage_io::read_record(&path)?,
    )?)
    .map_err(|e| e.to_string())?;
    if previous.schema_version != "1" {
        return Err("unsupported storage migration journal".to_owned());
    }
    if previous.complete {
        return Ok(());
    }
    let current: BTreeMap<_, _> = entries
        .iter()
        .map(|entry| (entry.path.as_str(), entry))
        .collect();
    if current.len() != previous.entries.len() {
        return Err(
            "Records changed during interrupted storage migration; inspect before resuming"
                .to_owned(),
        );
    }
    for prior in previous.entries {
        let entry = current
            .get(prior.path.as_str())
            .ok_or("Record paths changed during interrupted storage migration")?;
        if (entry.before_hash != prior.before_hash && entry.before_hash != prior.after_hash)
            || entry.logical_digest != prior.logical_digest
            || entry.id != prior.id
            || entry.kind != prior.kind
        {
            return Err(format!(
                "Record changed during interrupted storage migration: {}",
                prior.path
            ));
        }
    }
    Ok(())
}

fn save_journal(root: &Path, journal: &Journal) -> Result<(), String> {
    if !ignored(root, JOURNAL) {
        let ignore = root.join(".adf/local/storage-migrations/.gitignore");
        storage_io::reject_symlinks(root, &ignore)?;
        if ignore.exists() {
            return Err(
                "storage migration journal must be Git-ignored; review its existing .gitignore"
                    .to_owned(),
            );
        }
        storage_io::atomic_write(&ignore, b"*\n", true)?;
        if !ignored(root, JOURNAL) {
            return Err("storage migration journal must be Git-ignored".to_owned());
        }
    }
    storage_io::atomic_write(
        &journal_path(root)?,
        &serde_json::to_vec(journal).map_err(|e| e.to_string())?,
        false,
    )
}

fn switch_config(root: &Path, target: StoragePolicy) -> Result<(), String> {
    let path = root.join(".adf/config.yaml");
    storage_io::reject_symlinks(root, &path)?;
    let old = fs::read(&path).map_err(|e| e.to_string())?;
    let mut value: Value = serde_yaml::from_slice(&old).map_err(|e| e.to_string())?;
    let config = value.as_object_mut().ok_or("invalid project config")?;
    config.insert(
        "schema_version".to_owned(),
        json!(if target == StoragePolicy::Adaptive {
            "2"
        } else {
            "1"
        }),
    );
    if target == StoragePolicy::Adaptive {
        config.insert("record_storage".to_owned(), json!(target.as_str()));
    } else {
        config.remove("record_storage");
    }
    let bytes = serde_yaml::to_string(&value).map_err(|e| e.to_string())?;
    if fs::read(&path).map_err(|e| e.to_string())? != old {
        return Err("project config changed before migration".to_owned());
    }
    storage_io::atomic_write(&path, bytes.as_bytes(), false)
}

fn available_bytes(root: &Path) -> Result<u64, String> {
    let output = Command::new("df")
        .arg("-Pk")
        .arg(root)
        .output()
        .map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err("cannot determine free space before migration".to_owned());
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .last()
        .and_then(|line| line.split_whitespace().nth(3))
        .and_then(|v| v.parse::<u64>().ok())
        .and_then(|v| v.checked_mul(1024))
        .ok_or_else(|| "cannot parse free space before migration".to_owned())
}

fn apply(
    root: &Path,
    target: StoragePolicy,
    entries: Vec<Entry>,
    stop_after: Option<usize>,
) -> Result<(), String> {
    validate_pending(root, &entries)?;
    let mut journal = Journal {
        schema_version: "1".to_owned(),
        target: target.as_str().to_owned(),
        complete: false,
        entries,
    };
    let journal_bytes = serde_json::to_vec(&journal)
        .map_err(|e| e.to_string())?
        .len() as u64;
    let growth: u64 = journal
        .entries
        .iter()
        .map(|e| e.after_bytes.saturating_sub(e.before_bytes) as u64)
        .sum();
    let largest = journal
        .entries
        .iter()
        .map(|e| e.after_bytes as u64)
        .max()
        .unwrap_or(0);
    if available_bytes(root)? < growth + largest + journal_bytes * 2 + 65536 {
        return Err("insufficient free space for storage migration".to_owned());
    }
    save_journal(root, &journal)?;
    // Keep v2 enabled throughout either direction, including interrupted rollback.
    switch_config(root, StoragePolicy::Adaptive)?;
    if stop_after == Some(0) {
        return Err("simulated storage migration interruption".to_owned());
    }
    for (i, entry) in journal.entries.iter().enumerate() {
        let path = root.join(&entry.path);
        storage_io::reject_symlinks(root, &path)?;
        let raw = storage_io::read_record(&path)?;
        if hash(&raw) != entry.before_hash {
            return Err(format!("Record changed before replacement: {}", entry.path));
        }
        let (value, bytes) = converted(&raw, kind(&entry.kind)?, target)?;
        if hash(&bytes) != entry.after_hash
            || canonical_digest(&value).map_err(|e| e.to_string())? != entry.logical_digest
        {
            return Err("migration plan changed before replacement".to_owned());
        }
        if storage_io::read_record(&path)? != raw {
            return Err(format!("Record changed before replacement: {}", entry.path));
        }
        if raw != bytes {
            storage_io::atomic_write(&path, &bytes, false)?;
        }
        if stop_after == Some(i + 1) {
            return Err("simulated storage migration interruption".to_owned());
        }
    }
    // Verify all bytes after replacement before lowering the reader requirement.
    for entry in &journal.entries {
        let raw = storage_io::read_record(&root.join(&entry.path))?;
        if hash(&raw) != entry.after_hash {
            return Err("Record changed after migration".to_owned());
        }
        let restored = record_storage::decode(&raw, kind(&entry.kind)?)?;
        if canonical_digest(&restored).map_err(|e| e.to_string())? != entry.logical_digest {
            return Err("Record verification failed after migration".to_owned());
        }
    }
    if inventory(root)?.len() != journal.entries.len() {
        return Err("Record inventory changed during migration".to_owned());
    }
    switch_config(root, target)?;
    journal.complete = true;
    save_journal(root, &journal)
}

/// CLI-only explicit maintenance interface; ordinary project operations never migrate.
pub fn run_cli(arguments: &[String]) -> Result<Value, String> {
    let operation = arguments
        .first()
        .map(String::as_str)
        .ok_or("expected storage inspect, verify, export or migrate")?;
    if !matches!(operation, "inspect" | "verify" | "export" | "migrate") {
        return Err("unsupported storage operation".to_owned());
    }
    let mut options = BTreeMap::new();
    let mut dry_run = false;
    let mut i = 1;
    while i < arguments.len() {
        let flag = &arguments[i];
        if flag == "--dry-run" {
            if dry_run {
                return Err("duplicate --dry-run".to_owned());
            }
            dry_run = true;
            i += 1;
            continue;
        }
        if !matches!(
            flag.as_str(),
            "--project" | "--to" | "--release" | "--record" | "--format"
        ) {
            return Err(format!("unsupported storage argument: {flag}"));
        }
        let value = arguments
            .get(i + 1)
            .filter(|v| !v.starts_with("--"))
            .ok_or_else(|| format!("missing value for {flag}"))?;
        if options.insert(flag.as_str(), value.as_str()).is_some() {
            return Err(format!("duplicate argument: {flag}"));
        }
        i += 2;
    }
    if options.get("--format").is_some_and(|v| *v != "json") {
        return Err("storage commands support --format json".to_owned());
    }
    if operation != "migrate" && (dry_run || options.contains_key("--to")) {
        return Err("--to and --dry-run require storage migrate".to_owned());
    }
    if operation != "export" && options.contains_key("--record") {
        return Err("--record requires storage export".to_owned());
    }
    let root = Path::new(options.get("--project").copied().unwrap_or("."))
        .canonicalize()
        .map_err(|e| e.to_string())?;
    if !root.join(".adf/config.yaml").is_file() {
        return Err("project is not initialized".to_owned());
    }
    let _guard = if operation == "migrate" && !dry_run {
        StorageGuard::exclusive(&root)?
    } else {
        StorageGuard::shared(&root)?
    };
    let config = load_project_config(&root).map_err(|e| e.to_string())?;
    let lock = read_framework_lock(&root.join(".adf/framework.lock")).map_err(|e| e.to_string())?;
    let release = resolve_verified_release(&root, &lock, options.get("--release").map(Path::new))
        .map_err(|e| e.to_string())?;
    let target = match options
        .get("--to")
        .copied()
        .unwrap_or("adaptive-refmaps-v1")
    {
        "plain-json-v1" => StoragePolicy::Plain,
        "adaptive-refmaps-v1" => StoragePolicy::Adaptive,
        _ => return Err("unsupported target storage policy".to_owned()),
    };
    if operation == "migrate" && !options.contains_key("--to") {
        return Err("storage migrate requires --to".to_owned());
    }
    let entries = scan(&root, &release.schema_registry, target)?;
    validate_pending(&root, &entries)?;
    if operation == "export" {
        let id = options
            .get("--record")
            .ok_or("storage export requires --record")?;
        let entry = entries
            .iter()
            .find(|entry| entry.id == *id)
            .ok_or("Record does not exist")?;
        return record_storage::decode(
            &storage_io::read_record(&root.join(&entry.path))?,
            kind(&entry.kind)?,
        );
    }
    let before: usize = entries.iter().map(|e| e.before_bytes).sum();
    let after: usize = entries.iter().map(|e| e.after_bytes).sum();
    let changed = entries
        .iter()
        .filter(|e| e.before_hash != e.after_hash)
        .count();
    let skipped = skipped_files(&root)?;
    let growth: u64 = entries
        .iter()
        .map(|e| e.after_bytes.saturating_sub(e.before_bytes) as u64)
        .sum();
    let temporary_record_bytes = entries.iter().map(|e| e.after_bytes).max().unwrap_or(0);
    let journal_estimate = serde_json::to_vec(&entries)
        .map_err(|e| e.to_string())?
        .len() as u64
        + 1024;
    let required_free_bytes = growth + temporary_record_bytes as u64 + journal_estimate * 2 + 65536;
    let mut report = json!({"schema_version":"1", "operation":operation, "dry_run":dry_run,
        "current_policy":config.record_storage.as_str(), "target_policy":target.as_str(),
        "records":entries.len(), "changed_records":changed, "current_bytes":before, "proposed_bytes":after,
        "saved_bytes": before as i64 - after as i64, "logical_records_verified":true, "applied":false, "skipped_files":skipped,
        "temporary_record_bytes":temporary_record_bytes, "required_free_bytes":required_free_bytes});
    if operation == "migrate" && !dry_run {
        apply(&root, target, entries, None)?;
        report["applied"] = json!(true);
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interrupted_migration_can_resume_or_reverse_without_original_copies() {
        let root =
            std::env::temp_dir().join(format!("adf-storage-migration-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join(".adf/changes/change.example/results")).unwrap();
        let root = root.canonicalize().unwrap();
        assert!(
            Command::new("git")
                .args(["init", "-q"])
                .arg(&root)
                .status()
                .unwrap()
                .success()
        );
        fs::write(root.join(".gitignore"), ".adf/local/\n.adf/cache/\n").unwrap();
        fs::write(root.join(".adf/config.yaml"), "schema_version: '1'\nproject_sources:\n  contracts: contracts\n  decisions: decisions\nrepository_observation: .adf/repository-observation.yaml\n").unwrap();
        let refs: BTreeMap<_, _> = (0..20)
            .map(|i| (format!("code.{i}"), format!("sha256:{}", "a".repeat(64))))
            .collect();
        let mut entries = Vec::new();
        for i in 0..2 {
            let value = json!({"id":format!("result.{i}"), "change_id":"change.example", "input_refs":refs, "freshness_refs":refs});
            let raw = serde_json::to_vec_pretty(&value).unwrap();
            let (_, after) = converted(&raw, RecordKind::Result, StoragePolicy::Adaptive).unwrap();
            let path = format!(".adf/changes/change.example/results/{i}.json");
            fs::write(root.join(&path), &raw).unwrap();
            entries.push(Entry {
                path,
                kind: "result".to_owned(),
                id: format!("result.{i}"),
                before_hash: hash(&raw),
                after_hash: hash(&after),
                logical_digest: canonical_digest(&value).unwrap(),
                before_bytes: raw.len(),
                after_bytes: after.len(),
            });
        }
        let guard = StorageGuard::exclusive(&root).unwrap();
        assert!(StorageGuard::shared(&root).is_err());
        assert!(apply(&root, StoragePolicy::Adaptive, entries.clone(), Some(1)).is_err());
        assert_eq!(
            load_project_config(&root).unwrap().record_storage,
            StoragePolicy::Adaptive
        );
        let mut reverse = entries.clone();
        for entry in &mut reverse {
            let raw = storage_io::read_record(&root.join(&entry.path)).unwrap();
            let (_, bytes) = converted(&raw, RecordKind::Result, StoragePolicy::Plain).unwrap();
            entry.before_hash = hash(&raw);
            entry.before_bytes = raw.len();
            entry.after_hash = hash(&bytes);
            entry.after_bytes = bytes.len();
        }
        validate_pending(&root, &reverse).unwrap();
        let mut altered = reverse.clone();
        altered[0].before_hash = "sha256:external-edit".to_owned();
        assert!(validate_pending(&root, &altered).is_err());
        let mut resumed = reverse.clone();
        for entry in &mut resumed {
            let raw = storage_io::read_record(&root.join(&entry.path)).unwrap();
            let (_, bytes) = converted(&raw, RecordKind::Result, StoragePolicy::Adaptive).unwrap();
            entry.after_hash = hash(&bytes);
            entry.after_bytes = bytes.len();
        }
        apply(&root, StoragePolicy::Adaptive, resumed, None).unwrap();
        for entry in &mut reverse {
            let raw = storage_io::read_record(&root.join(&entry.path)).unwrap();
            let (_, bytes) = converted(&raw, RecordKind::Result, StoragePolicy::Plain).unwrap();
            entry.before_hash = hash(&raw);
            entry.before_bytes = raw.len();
            entry.after_hash = hash(&bytes);
            entry.after_bytes = bytes.len();
        }
        apply(&root, StoragePolicy::Plain, reverse, Some(0)).unwrap_err();
        let mut rollback = entries.clone();
        for entry in &mut rollback {
            let raw = storage_io::read_record(&root.join(&entry.path)).unwrap();
            let (_, bytes) = converted(&raw, RecordKind::Result, StoragePolicy::Plain).unwrap();
            entry.before_hash = hash(&raw);
            entry.before_bytes = raw.len();
            entry.after_hash = hash(&bytes);
            entry.after_bytes = bytes.len();
        }
        apply(&root, StoragePolicy::Plain, rollback, None).unwrap();
        assert_eq!(
            load_project_config(&root).unwrap().record_storage,
            StoragePolicy::Plain
        );
        for entry in entries {
            let raw = storage_io::read_record(&root.join(entry.path)).unwrap();
            assert!(
                record_storage::parse_strict(&raw)
                    .unwrap()
                    .get("storage_format")
                    .is_none()
            );
            assert_eq!(
                canonical_digest(&record_storage::decode(&raw, RecordKind::Result).unwrap())
                    .unwrap(),
                entry.logical_digest
            );
        }
        drop(guard);
        fs::remove_dir_all(root).unwrap();
    }
}
