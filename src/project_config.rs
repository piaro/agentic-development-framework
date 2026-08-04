//! Strict loading of the small, project-owned `.adf/config.yaml`.

use serde_json::Value;
use std::collections::BTreeSet;
use std::fmt;
use std::fs;
use std::path::{Component, Path, PathBuf};

pub const PROJECT_CONFIG_SCHEMA_VERSION: &str = "1";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectConfig {
    pub contract_root: String,
    pub decision_root: String,
    pub repository_observation: String,
}

pub fn load_project_config(root: &Path) -> Result<ProjectConfig, ProjectConfigError> {
    let path = repository_path(root, ".adf/config.yaml")?;
    let text = fs::read_to_string(&path)
        .map_err(|error| config_error(format!("{}: {error}", path.display())))?;
    let value: Value = serde_yaml::from_str(&text)
        .map_err(|error| config_error(format!("{}: {error}", path.display())))?;
    let object = value
        .as_object()
        .ok_or_else(|| config_error("project config must be a mapping"))?;
    assert_exact_fields(
        object.keys().map(String::as_str),
        &[
            "schema_version",
            "project_sources",
            "repository_observation",
        ],
        "project config",
    )?;
    if object["schema_version"].as_str() != Some(PROJECT_CONFIG_SCHEMA_VERSION) {
        return Err(config_error(format!(
            "unsupported project config schema: {}",
            object["schema_version"]
        )));
    }
    let sources = object["project_sources"]
        .as_object()
        .ok_or_else(|| config_error("project_sources must be a mapping"))?;
    assert_exact_fields(
        sources.keys().map(String::as_str),
        &["contracts", "decisions"],
        "project_sources",
    )?;
    let contract_root = required_string(&sources["contracts"], "project_sources.contracts")?;
    let decision_root = required_string(&sources["decisions"], "project_sources.decisions")?;
    let repository_observation =
        required_string(&object["repository_observation"], "repository_observation")?;

    // Validate all configured paths before an Adapter reads any project data.
    repository_path(root, contract_root)?;
    repository_path(root, decision_root)?;
    repository_path(root, repository_observation)?;
    Ok(ProjectConfig {
        contract_root: contract_root.to_owned(),
        decision_root: decision_root.to_owned(),
        repository_observation: repository_observation.to_owned(),
    })
}

/// Resolve a repository-relative path without allowing `..` or symlink escape.
pub fn repository_path(root: &Path, relative: &str) -> Result<PathBuf, ProjectConfigError> {
    let path = Path::new(relative);
    if path.is_absolute() {
        return Err(config_error(format!(
            "configured path must be repository-relative: {relative}"
        )));
    }
    let mut candidate = root.to_path_buf();
    for component in path.components() {
        match component {
            Component::Normal(part) => {
                candidate.push(part);
                if candidate.exists() {
                    candidate = candidate.canonicalize().map_err(|error| {
                        config_error(format!("cannot resolve {relative}: {error}"))
                    })?;
                    if !candidate.starts_with(root) {
                        return Err(config_error(format!(
                            "configured path escapes repository: {relative}"
                        )));
                    }
                }
            }
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(config_error(format!(
                    "configured path escapes repository: {relative}"
                )));
            }
        }
    }
    Ok(candidate)
}

fn assert_exact_fields<'a>(
    actual: impl Iterator<Item = &'a str>,
    expected: &[&str],
    label: &str,
) -> Result<(), ProjectConfigError> {
    let actual: BTreeSet<&str> = actual.collect();
    let expected: BTreeSet<&str> = expected.iter().copied().collect();
    let mut details = Vec::new();
    for field in actual.difference(&expected) {
        details.push(format!("unexpected field: {field}"));
    }
    for field in expected.difference(&actual) {
        details.push(format!("missing field: {field}"));
    }
    if details.is_empty() {
        Ok(())
    } else {
        Err(config_error(format!(
            "invalid {label}: {}",
            details.join(", ")
        )))
    }
}

fn required_string<'a>(value: &'a Value, label: &str) -> Result<&'a str, ProjectConfigError> {
    value
        .as_str()
        .ok_or_else(|| config_error(format!("{label} must be a string")))
}

fn config_error(message: impl Into<String>) -> ProjectConfigError {
    ProjectConfigError {
        message: message.into(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectConfigError {
    message: String,
}

impl fmt::Display for ProjectConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ProjectConfigError {}
