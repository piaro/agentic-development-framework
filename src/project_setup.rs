//! Safe, mechanical setup for a project without inventing semantic bindings.

use crate::delivery::resolve_verified_release;
use crate::distribution_trust::{DISTRIBUTION_TRUST_FILE, trust_store_for_lock};
use crate::framework_detection::{
    FrameworkCandidate, FrameworkCatalog, is_framework_manifest_path,
};
use crate::project_config::load_project_config;
use crate::remote_delivery::install_release_archive;
use crate::source_detection::{
    SourceObservation, SourceObservationKind, detector_for_path, source_pathspecs,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

pub const CANDIDATE_LOCK_FILE: &str = "candidate-framework.lock";
pub const FRAMEWORK_ARCHIVE_FILE: &str = "framework-release.tar";
pub const PUBLISH_RECEIPT_FILE: &str = "publish-receipt.json";
pub const PUBLICATION_RECORD_FILE: &str = "publication-record.json";
static PROMOTION_SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub struct ProjectInitOptions<'a> {
    pub project_root: &'a Path,
    pub candidate_root: &'a Path,
    pub analysis_roots: &'a [String],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectInitReceipt {
    pub project_root: PathBuf,
    pub release_id: String,
    pub already_installed: bool,
    pub created_files: Vec<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObservationPromotionReceipt {
    pub observation_path: PathBuf,
    pub artifacts: usize,
}

pub fn initialize_project(
    options: &ProjectInitOptions<'_>,
) -> Result<ProjectInitReceipt, ProjectSetupError> {
    let project_root = options
        .project_root
        .canonicalize()
        .map_err(|error| setup_error(format!("cannot resolve project root: {error}")))?;
    if !project_root.is_dir() {
        return Err(setup_error("project root must be a directory"));
    }
    assert_git_root(&project_root)?;
    for relative in [
        ".agentic",
        ".agentic/cache",
        ".agentic/changes",
        "contracts",
        "decisions",
    ] {
        reject_symlink_components(&project_root, Path::new(relative))?;
    }
    let candidate_root = options
        .candidate_root
        .canonicalize()
        .map_err(|error| setup_error(format!("cannot resolve candidate root: {error}")))?;
    if !candidate_root.is_dir() {
        return Err(setup_error("candidate root must be a directory"));
    }
    let analysis_roots = normalize_analysis_roots(options.analysis_roots)?;
    let lock_path = candidate_root.join(CANDIDATE_LOCK_FILE);
    let archive_path = candidate_root.join(FRAMEWORK_ARCHIVE_FILE);
    let trust_path = candidate_root.join(DISTRIBUTION_TRUST_FILE);
    require_regular_file(&lock_path)?;
    require_regular_file(&archive_path)?;
    require_regular_file(&trust_path)?;
    let framework_lock = read_yaml(&lock_path)?;
    let trust_store = trust_store_for_lock(&trust_path, &framework_lock)
        .map_err(|error| setup_error(error.to_string()))?;
    let release_id = framework_lock
        .get("framework_release")
        .and_then(Value::as_str)
        .ok_or_else(|| setup_error("candidate Framework lock has no framework_release"))?
        .to_owned();

    let targets = [
        project_root.join(".agentic/config.yaml"),
        project_root.join(".agentic/framework.lock"),
        project_root.join(".agentic/repository-observation.yaml"),
        project_root.join(".agentic/trusted-release-keys.yaml"),
        project_root.join(".agentic/cache/.gitignore"),
    ];
    let conflicts = targets
        .iter()
        .filter(|path| path.exists())
        .map(|path| relative_display(&project_root, path))
        .collect::<Vec<_>>();
    if !conflicts.is_empty() {
        return Err(setup_error(format!(
            "project initialization would overwrite existing files: {}",
            conflicts.join(", ")
        )));
    }

    fs::create_dir_all(project_root.join(".agentic/changes"))
        .and_then(|()| fs::create_dir_all(project_root.join("contracts")))
        .and_then(|()| fs::create_dir_all(project_root.join("decisions")))
        .map_err(|error| setup_error(format!("cannot create project directories: {error}")))?;

    let trust_yaml =
        serde_yaml::to_string(&trust_store).map_err(|error| setup_error(error.to_string()))?;
    write_new(&targets[3], trust_yaml.as_bytes())?;
    let install = match install_release_archive(&project_root, &framework_lock, &archive_path) {
        Ok(receipt) => receipt,
        Err(error) => {
            let _ = fs::remove_file(&targets[3]);
            return Err(setup_error(error.to_string()));
        }
    };

    let config = concat!(
        "schema_version: \"1\"\n",
        "project_sources:\n",
        "  contracts: contracts\n",
        "  decisions: decisions\n",
        "repository_observation: .agentic/repository-observation.yaml\n"
    );
    let observation = serde_yaml::to_string(&json!({
        "schema_version": "5",
        "phase": "pre-build",
        "analysis": {"roots": analysis_roots},
        "artifacts": [],
    }))
    .map_err(|error| setup_error(error.to_string()))?;
    let lock_bytes = fs::read(&lock_path)
        .map_err(|error| setup_error(format!("{}: {error}", lock_path.display())))?;

    let writes = [
        (&targets[0], config.as_bytes()),
        (&targets[1], lock_bytes.as_slice()),
        (&targets[2], observation.as_bytes()),
        (&targets[4], b"*\n!.gitignore\n".as_slice()),
    ];
    let mut created = vec![targets[3].clone()];
    for (path, bytes) in writes {
        if let Err(error) = write_new(path, bytes) {
            for created_path in &created {
                let _ = fs::remove_file(created_path);
            }
            return Err(error);
        }
        created.push(path.clone());
    }
    created.sort();
    Ok(ProjectInitReceipt {
        project_root,
        release_id,
        already_installed: install.already_installed,
        created_files: created,
    })
}

pub fn initialize_change(
    project_root: &Path,
    change_id: &str,
    title: &str,
    intent: &str,
) -> Result<PathBuf, ProjectSetupError> {
    if !safe_prefixed_id(change_id, "change.") {
        return Err(setup_error(
            "Change ID must use change.<id> with letters, digits, '.', '_', or '-'",
        ));
    }
    let root = project_root
        .canonicalize()
        .map_err(|error| setup_error(format!("cannot resolve project root: {error}")))?;
    assert_git_root(&root)?;
    reject_symlink_components(
        &root,
        &Path::new(".agentic/changes")
            .join(change_id)
            .join("change.yaml"),
    )?;
    if title.trim().is_empty() || intent.trim().is_empty() {
        return Err(setup_error("Change title and intent must not be empty"));
    }
    let config = root.join(".agentic/config.yaml");
    if !config.is_file() {
        return Err(setup_error(format!(
            "Project is not initialized: {} is missing.\nNext: agentic project init --project {}",
            config.display(),
            root.display()
        )));
    }
    let path = root
        .join(".agentic/changes")
        .join(change_id)
        .join("change.yaml");
    let value = json!({
        "schema_version": "1",
        "id": change_id,
        "title": title,
        "intent": intent,
    });
    let yaml = serde_yaml::to_string(&value).map_err(|error| setup_error(error.to_string()))?;
    write_new(&path, yaml.as_bytes())?;
    Ok(path)
}

pub fn observation_draft(
    project_root: &Path,
    analysis_roots: &[String],
) -> Result<Value, ProjectSetupError> {
    let root = project_root
        .canonicalize()
        .map_err(|error| setup_error(format!("cannot resolve project root: {error}")))?;
    assert_git_root(&root)?;
    let roots = normalize_analysis_roots(analysis_roots)?;
    let base_observation_digest = observation_base_digest(&root)?;
    let pathspecs = source_pathspecs();
    let mut arguments = vec![
        "ls-files",
        "--cached",
        "--others",
        "--exclude-standard",
        "--",
    ];
    arguments.extend(pathspecs.iter().map(String::as_str));
    let output = git(&root, &arguments)?;
    let framework_catalog = framework_catalog(&root)?;
    let mut artifacts = Vec::new();
    let mut binding_artifacts = Vec::new();
    let mut source_digests = BTreeMap::new();
    for relative in output.lines().filter(|path| under_roots(path, &roots)) {
        let path = root.join(relative);
        let metadata = fs::symlink_metadata(&path)
            .map_err(|error| setup_error(format!("{}: {error}", path.display())))?;
        if metadata.file_type().is_symlink() {
            return Err(setup_error(format!(
                "refusing symlinked analysis artifact: {relative}"
            )));
        }
        if !metadata.is_file() {
            continue;
        }
        let source_bytes =
            fs::read(&path).map_err(|error| setup_error(format!("{}: {error}", path.display())))?;
        source_digests.insert(
            relative.to_owned(),
            format!("sha256:{:x}", Sha256::digest(&source_bytes)),
        );
        let detector = detector_for_path(relative)
            .ok_or_else(|| setup_error(format!("source language is unknown: {relative}")))?;
        let source = if detector.is_supported() {
            Some(
                String::from_utf8(source_bytes)
                    .map_err(|error| setup_error(format!("{}: {error}", path.display())))?,
            )
        } else {
            None
        };
        let observations = if let Some(source) = source.as_deref() {
            detector
                .observe(source)
                .map_err(|error| setup_error(format!("{relative}: {error}")))?
        } else {
            Vec::new()
        };
        let framework_candidates = source
            .as_deref()
            .map(|source| {
                framework_catalog.candidates(relative, detector.language, source, &observations)
            })
            .unwrap_or_default();
        let symbols = observations
            .iter()
            .map(|item| item.symbol.clone())
            .collect::<BTreeSet<_>>();
        let resources = observations
            .iter()
            .map(|item| item.resource.clone())
            .collect::<BTreeSet<_>>();
        binding_artifacts.push(binding_artifact_template(
            relative,
            detector.language,
            &observations,
            &framework_candidates,
        ));
        artifacts.push(json!({
            "path": relative,
            "language": detector.language,
            "detector_status": if detector.is_supported() { "supported" } else { "unsupported" },
            "candidate_ref": candidate_ref(relative),
            "symbols": symbols,
            "resources": resources,
            "observations": observations.into_iter().map(|item| json!({
                "kind": match item.kind {
                    SourceObservationKind::DbWrite => "db_write",
                    SourceObservationKind::MessagePublish => "message_publish",
                    SourceObservationKind::OtherMethodCall => "other_method_call",
                },
                "symbol": item.symbol,
                "resource": item.resource,
                "method": item.method,
                "line": item.line,
            })).collect::<Vec<_>>(),
            "framework_candidates": framework_candidates.into_iter().map(framework_candidate_value).collect::<Vec<_>>(),
        }));
    }
    artifacts.sort_by(|left, right| left["path"].as_str().cmp(&right["path"].as_str()));
    binding_artifacts.sort_by(|left, right| left["path"].as_str().cmp(&right["path"].as_str()));
    Ok(json!({
        "schema_version": "6",
        "kind": "repository-observation-draft",
        "analysis_roots": roots,
        "base_observation_digest": base_observation_digest,
        "source_digests": source_digests,
        "artifacts": artifacts,
        "binding_artifacts": binding_artifacts,
        "next": "Review each physical symbol, resource, and framework candidate. In binding_artifacts, remove irrelevant placeholders and fill every retained null with reviewed logical_refs or fact_kinds, owner, and accepted Decision authority_ref. Validate the Draft, then explicitly promote it into the repository observation schema v5 file. Suggested fact kinds are non-authoritative, and candidates with an empty list require call-specific classification. This draft is not authoritative and never updates project files by itself.",
    }))
}

/// Atomically replace the configured authoritative Observation with reviewed Draft bindings.
pub fn promote_observation_draft(
    project_root: &Path,
    draft: &Value,
) -> Result<ObservationPromotionReceipt, ProjectSetupError> {
    let root = project_root
        .canonicalize()
        .map_err(|error| setup_error(format!("cannot resolve project root: {error}")))?;
    assert_git_root(&root)?;
    let observation_path = configured_observation_path(&root)?;
    let current_bytes = read_regular_file(&observation_path, "Repository Observation")?;
    let expected_digest = draft["base_observation_digest"]
        .as_str()
        .filter(|digest| *digest == sha256_digest(&current_bytes))
        .ok_or_else(|| {
            setup_error(
                "Repository Observation changed after Draft generation; generate and review a new Draft",
            )
        })?;
    let current: Value = serde_yaml::from_slice(&current_bytes)
        .map_err(|error| setup_error(format!("{}: {error}", observation_path.display())))?;
    let phase = current["phase"]
        .as_str()
        .filter(|phase| matches!(*phase, "pre-build" | "post-build"))
        .ok_or_else(|| setup_error("Repository Observation phase is invalid"))?;
    let roots = draft["analysis_roots"]
        .as_array()
        .ok_or_else(|| setup_error("Binding Draft analysis_roots must be an array"))?;
    let artifacts = draft["binding_artifacts"]
        .as_array()
        .ok_or_else(|| setup_error("Binding Draft binding_artifacts must be an array"))?;
    let promoted = json!({
        "schema_version": "5",
        "phase": phase,
        "analysis": {"roots": roots},
        "artifacts": artifacts,
    });
    let bytes = serde_yaml::to_string(&promoted)
        .map_err(|error| setup_error(error.to_string()))?
        .into_bytes();
    atomic_replace_if_digest(&observation_path, expected_digest, &bytes)?;
    Ok(ObservationPromotionReceipt {
        observation_path,
        artifacts: artifacts.len(),
    })
}

/// Write an explicitly requested Observation Draft without replacing any file.
pub fn write_observation_draft(
    project_root: &Path,
    relative: &str,
    contents: &[u8],
) -> Result<PathBuf, ProjectSetupError> {
    let root = project_root
        .canonicalize()
        .map_err(|error| setup_error(format!("cannot resolve project root: {error}")))?;
    assert_git_root(&root)?;
    let relative = Path::new(relative);
    if relative.as_os_str().is_empty() {
        return Err(setup_error(
            "Observation Draft output path must not be empty",
        ));
    }
    if relative
        .components()
        .next()
        .is_some_and(|component| component.as_os_str() == ".git")
    {
        return Err(setup_error(
            "Observation Draft output path must not be inside .git",
        ));
    }
    reject_symlink_components(&root, relative)?;
    let path = root.join(relative);
    write_new(&path, contents)?;
    Ok(path)
}

/// Read one Project-relative Observation Draft without following symlinks.
pub fn read_observation_draft(
    project_root: &Path,
    relative: &str,
) -> Result<Value, ProjectSetupError> {
    let root = project_root
        .canonicalize()
        .map_err(|error| setup_error(format!("cannot resolve project root: {error}")))?;
    assert_git_root(&root)?;
    let relative = Path::new(relative);
    reject_symlink_components(&root, relative)?;
    let path = root.join(relative);
    let metadata = fs::symlink_metadata(&path)
        .map_err(|error| setup_error(format!("{}: {error}", path.display())))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(setup_error(format!(
            "Observation Draft is not a regular file: {}",
            path.display()
        )));
    }
    let contents = fs::read_to_string(&path)
        .map_err(|error| setup_error(format!("{}: {error}", path.display())))?;
    serde_yaml::from_str(&contents)
        .map_err(|error| setup_error(format!("{}: {error}", path.display())))
}

fn binding_artifact_template(
    path: &str,
    language: &str,
    observations: &[SourceObservation],
    candidates: &[FrameworkCandidate],
) -> Value {
    let candidate_calls = candidates
        .iter()
        .map(|candidate| {
            (
                candidate.symbol.as_str(),
                candidate.resource.as_str(),
                candidate.method.as_str(),
            )
        })
        .collect::<BTreeSet<_>>();
    let relevant = observations
        .iter()
        .filter(|observation| {
            observation.kind != SourceObservationKind::OtherMethodCall
                || candidate_calls.contains(&(
                    observation.symbol.as_str(),
                    observation.resource.as_str(),
                    observation.method.as_str(),
                ))
        })
        .collect::<Vec<_>>();
    let symbols = relevant
        .iter()
        .map(|observation| {
            (
                observation.symbol.clone(),
                json!({
                    "logical_ref": null,
                    "owner": null,
                    "authority_ref": null,
                }),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let resources = relevant
        .iter()
        .map(|observation| {
            (
                observation.resource.clone(),
                json!({
                    "logical_refs": null,
                    "owner": null,
                    "authority_ref": null,
                }),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let method_keys = candidates
        .iter()
        .filter(|candidate| candidate.method_binding_required)
        .map(|candidate| candidate.binding_key.clone())
        .collect::<BTreeSet<_>>();
    let methods = method_keys
        .into_iter()
        .map(|binding_key| {
            (
                binding_key,
                json!({
                    "fact_kinds": null,
                    "owner": null,
                    "authority_ref": null,
                }),
            )
        })
        .collect::<BTreeMap<_, _>>();
    json!({
        "ref": candidate_ref(path),
        "path": path,
        "language": language,
        "bindings": {
            "symbols": symbols,
            "resources": resources,
            "methods": methods,
        },
    })
}

fn framework_candidate_value(candidate: FrameworkCandidate) -> Value {
    json!({
        "framework": candidate.framework,
        "symbol": candidate.symbol,
        "resource": candidate.resource,
        "method": candidate.method,
        "line": candidate.line,
        "binding_key": candidate.binding_key,
        "suggested_fact_kinds": candidate.suggested_fact_kinds.into_iter().map(|kind| kind.as_str()).collect::<Vec<_>>(),
        "method_binding_required": candidate.method_binding_required,
        "review_status": "required",
        "evidence": candidate.evidence,
        "rationale": candidate.rationale,
    })
}

fn framework_catalog(root: &Path) -> Result<FrameworkCatalog, ProjectSetupError> {
    let output = git(
        root,
        &["ls-files", "--cached", "--others", "--exclude-standard"],
    )?;
    let lock_path = root.join(".agentic/framework.lock");
    let mut catalog = if lock_path.is_file() {
        let framework_lock = read_yaml(&lock_path)?;
        resolve_verified_release(root, &framework_lock, None)
            .map_err(|error| setup_error(error.to_string()))?
            .framework_catalog
    } else {
        FrameworkCatalog::default()
    };
    for relative in output
        .lines()
        .filter(|path| is_framework_manifest_path(path))
    {
        let relative_path = Path::new(relative);
        if reject_symlink_components(root, relative_path).is_err() {
            continue;
        }
        let path = root.join(relative_path);
        let Ok(metadata) = fs::symlink_metadata(&path) else {
            continue;
        };
        if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() > 1_048_576 {
            continue;
        }
        let Ok(content) = fs::read_to_string(&path) else {
            continue;
        };
        catalog.record_manifest(relative, &content);
    }
    Ok(catalog)
}

pub fn default_candidate_root() -> Result<PathBuf, ProjectSetupError> {
    let executable = std::env::current_exe()
        .map_err(|error| setup_error(format!("cannot locate current executable: {error}")))?;
    executable
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| setup_error("current executable has no parent directory"))
}

fn normalize_analysis_roots(values: &[String]) -> Result<Vec<String>, ProjectSetupError> {
    let values = if values.is_empty() {
        vec![".".to_owned()]
    } else {
        values.to_vec()
    };
    let mut roots = BTreeSet::new();
    for value in values {
        let path = Path::new(&value);
        if path.is_absolute() {
            return Err(setup_error("analysis roots must be repository-relative"));
        }
        let mut parts = Vec::new();
        for component in path.components() {
            match component {
                Component::Normal(part) => parts.push(part.to_string_lossy().into_owned()),
                Component::CurDir => {}
                Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                    return Err(setup_error("analysis roots must not escape the repository"));
                }
            }
        }
        roots.insert(if parts.is_empty() {
            ".".to_owned()
        } else {
            parts.join("/")
        });
    }
    Ok(roots.into_iter().collect())
}

fn under_roots(path: &str, roots: &[String]) -> bool {
    roots
        .iter()
        .any(|root| root == "." || path == root || path.starts_with(&format!("{root}/")))
}

fn candidate_ref(path: &str) -> String {
    let normalized = path
        .trim_end_matches(".py")
        .chars()
        .map(|value| {
            if value.is_ascii_alphanumeric() {
                value
            } else {
                '.'
            }
        })
        .collect::<String>();
    format!("code.{}", normalized.trim_matches('.'))
}

fn assert_git_root(root: &Path) -> Result<(), ProjectSetupError> {
    let top = git(root, &["rev-parse", "--show-toplevel"])?;
    let top = PathBuf::from(top)
        .canonicalize()
        .map_err(|error| setup_error(format!("cannot resolve Git top-level: {error}")))?;
    if top != root {
        return Err(setup_error(format!(
            "project root must be the Git top-level: {}",
            top.display()
        )));
    }
    Ok(())
}

fn git(root: &Path, arguments: &[&str]) -> Result<String, ProjectSetupError> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(arguments)
        .output()
        .map_err(|error| setup_error(format!("cannot execute Git: {error}")))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        return Err(setup_error(format!(
            "Git command failed ({}): {stderr}",
            arguments.join(" ")
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn read_yaml(path: &Path) -> Result<Value, ProjectSetupError> {
    let text = fs::read_to_string(path)
        .map_err(|error| setup_error(format!("{}: {error}", path.display())))?;
    serde_yaml::from_str(&text).map_err(|error| setup_error(format!("{}: {error}", path.display())))
}

fn require_regular_file(path: &Path) -> Result<(), ProjectSetupError> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| setup_error(format!("{}: {error}", path.display())))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(setup_error(format!(
            "candidate asset is not a regular file: {}",
            path.display()
        )));
    }
    Ok(())
}

fn reject_symlink_components(root: &Path, relative: &Path) -> Result<(), ProjectSetupError> {
    if relative.is_absolute() {
        return Err(setup_error("generated project path must be relative"));
    }
    let mut current = root.to_path_buf();
    for component in relative.components() {
        match component {
            Component::Normal(part) => current.push(part),
            Component::CurDir => continue,
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(setup_error(
                    "generated project path must stay in the repository",
                ));
            }
        }
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(setup_error(format!(
                    "refusing symlinked project path: {}",
                    relative_display(root, &current)
                )));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(setup_error(format!(
                    "cannot inspect {}: {error}",
                    current.display()
                )));
            }
        }
    }
    Ok(())
}

fn configured_observation_path(root: &Path) -> Result<PathBuf, ProjectSetupError> {
    let config = load_project_config(root).map_err(|error| setup_error(error.to_string()))?;
    let relative = Path::new(&config.repository_observation);
    reject_symlink_components(root, relative)?;
    Ok(root.join(relative))
}

fn observation_base_digest(root: &Path) -> Result<String, ProjectSetupError> {
    match fs::symlink_metadata(root.join(".agentic/config.yaml")) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            // Observation-only use remains available before Project initialization.
            // Such a Draft cannot be promoted because promotion requires a configured
            // authoritative Observation whose digest will not match this sentinel.
            return Ok(sha256_digest(&[]));
        }
        Err(error) => {
            return Err(setup_error(format!(
                "cannot inspect Project config: {error}"
            )));
        }
        Ok(_) => {}
    }
    let path = configured_observation_path(root)?;
    read_regular_file(&path, "Repository Observation").map(|bytes| sha256_digest(&bytes))
}

fn read_regular_file(path: &Path, label: &str) -> Result<Vec<u8>, ProjectSetupError> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| setup_error(format!("{}: {error}", path.display())))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(setup_error(format!(
            "{label} is not a regular file: {}",
            path.display()
        )));
    }
    fs::read(path).map_err(|error| setup_error(format!("{}: {error}", path.display())))
}

fn sha256_digest(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

fn atomic_replace_if_digest(
    path: &Path,
    expected_digest: &str,
    bytes: &[u8],
) -> Result<(), ProjectSetupError> {
    let parent = path
        .parent()
        .ok_or_else(|| setup_error("Repository Observation has no parent directory"))?;
    let temporary = parent.join(format!(
        ".repository-observation.tmp-{}-{}",
        std::process::id(),
        PROMOTION_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    let result = (|| {
        let permissions = fs::symlink_metadata(path)
            .map_err(|error| setup_error(format!("{}: {error}", path.display())))?
            .permissions();
        let mut output = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)
            .map_err(|error| setup_error(format!("{}: {error}", temporary.display())))?;
        output
            .write_all(bytes)
            .and_then(|()| output.sync_all())
            .map_err(|error| setup_error(format!("{}: {error}", temporary.display())))?;
        fs::set_permissions(&temporary, permissions)
            .map_err(|error| setup_error(format!("{}: {error}", temporary.display())))?;
        let current = read_regular_file(path, "Repository Observation")?;
        if sha256_digest(&current) != expected_digest {
            return Err(setup_error(
                "Repository Observation changed during promotion; no replacement was performed",
            ));
        }
        replace_file(&temporary, path)?;
        sync_directory(parent)
    })();
    if temporary.exists() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> Result<(), ProjectSetupError> {
    fs::File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| setup_error(format!("{}: {error}", path.display())))
}

#[cfg(windows)]
fn sync_directory(_path: &Path) -> Result<(), ProjectSetupError> {
    Ok(())
}

#[cfg(not(windows))]
fn replace_file(source: &Path, target: &Path) -> Result<(), ProjectSetupError> {
    fs::rename(source, target)
        .map_err(|error| setup_error(format!("{}: {error}", target.display())))
}

#[cfg(windows)]
fn replace_file(source: &Path, target: &Path) -> Result<(), ProjectSetupError> {
    use std::os::windows::ffi::OsStrExt;

    const MOVEFILE_REPLACE_EXISTING: u32 = 0x1;
    const MOVEFILE_WRITE_THROUGH: u32 = 0x8;
    #[link(name = "Kernel32")]
    unsafe extern "system" {
        fn MoveFileExW(existing: *const u16, replacement: *const u16, flags: u32) -> i32;
    }
    let source_wide = source
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let target_wide = target
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    // SAFETY: both pointers refer to live NUL-terminated buffers and the flags
    // request a documented replacement with write-through semantics.
    let moved = unsafe {
        MoveFileExW(
            source_wide.as_ptr(),
            target_wide.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if moved == 0 {
        Err(setup_error(format!(
            "{}: {}",
            target.display(),
            std::io::Error::last_os_error()
        )))
    } else {
        Ok(())
    }
}

fn write_new(path: &Path, bytes: &[u8]) -> Result<(), ProjectSetupError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| setup_error(format!("cannot create {}: {error}", parent.display())))?;
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::AlreadyExists {
                setup_error(format!(
                    "refusing to overwrite existing file: {}",
                    path.display()
                ))
            } else {
                setup_error(format!("cannot create {}: {error}", path.display()))
            }
        })?;
    file.write_all(bytes)
        .and_then(|()| file.sync_all())
        .map_err(|error| setup_error(format!("cannot write {}: {error}", path.display())))
}

fn relative_display(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn safe_prefixed_id(value: &str, prefix: &str) -> bool {
    value.starts_with(prefix)
        && value.len() > prefix.len()
        && value
            .bytes()
            .all(|value| value.is_ascii_alphanumeric() || matches!(value, b'.' | b'_' | b'-'))
}

fn setup_error(message: impl Into<String>) -> ProjectSetupError {
    ProjectSetupError {
        message: message.into(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectSetupError {
    message: String,
}

impl fmt::Display for ProjectSetupError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ProjectSetupError {}
