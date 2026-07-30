//! Safe, mechanical setup for a project without inventing semantic bindings.

use crate::distribution_trust::{DISTRIBUTION_TRUST_FILE, trust_store_for_lock};
use crate::python_detection::{PythonObservationKind, observe_python};
use crate::remote_delivery::install_release_archive;
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

pub const CANDIDATE_LOCK_FILE: &str = "candidate-framework.lock";
pub const FRAMEWORK_ARCHIVE_FILE: &str = "framework-release.tar";
pub const PUBLISH_RECEIPT_FILE: &str = "publish-receipt.json";
pub const PUBLICATION_RECORD_FILE: &str = "publication-record.json";

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
        "schema_version": "3",
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
    let output = git(
        &root,
        &[
            "ls-files",
            "--cached",
            "--others",
            "--exclude-standard",
            "--",
            "*.py",
        ],
    )?;
    let mut artifacts = Vec::new();
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
        let source = fs::read_to_string(&path)
            .map_err(|error| setup_error(format!("{}: {error}", path.display())))?;
        let observations =
            observe_python(&source).map_err(|error| setup_error(format!("{relative}: {error}")))?;
        let symbols = observations
            .iter()
            .map(|item| item.symbol.clone())
            .collect::<BTreeSet<_>>();
        let resources = observations
            .iter()
            .map(|item| item.resource.clone())
            .collect::<BTreeSet<_>>();
        artifacts.push(json!({
            "path": relative,
            "language": "python",
            "candidate_ref": candidate_ref(relative),
            "symbols": symbols,
            "resources": resources,
            "observations": observations.into_iter().map(|item| json!({
                "kind": match item.kind {
                    PythonObservationKind::DbWrite => "db_write",
                    PythonObservationKind::MessagePublish => "message_publish",
                    PythonObservationKind::OtherMethodCall => "other_method_call",
                },
                "symbol": item.symbol,
                "resource": item.resource,
                "method": item.method,
                "line": item.line,
            })).collect::<Vec<_>>(),
        }));
    }
    artifacts.sort_by(|left, right| left["path"].as_str().cmp(&right["path"].as_str()));
    Ok(json!({
        "schema_version": "1",
        "kind": "repository-observation-draft",
        "analysis_roots": roots,
        "artifacts": artifacts,
        "next": "Review each physical symbol and resource, then record logical_ref, owner, and an accepted Decision authority_ref in .agentic/repository-observation.yaml. This draft is not authoritative.",
    }))
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
