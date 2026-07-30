//! Git-backed, fail-closed source observation.
//!
//! Git defines the mechanical analysis target. Language-specific parsing
//! extracts physical observations, then reviewed Binding Records map those
//! observations to stable project vocabulary.

use crate::canonical_digest;
use crate::project_config::repository_path;
use crate::python_detection::{PythonObservationKind, observe_python};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

pub const OBSERVATION_SCHEMA_VERSION: &str = "3";

pub struct GitRepositoryAdapter {
    root: PathBuf,
    manifest_path: PathBuf,
    require_clean: bool,
}

#[derive(Clone)]
struct BindingRecord {
    logical_ref: String,
    owner: String,
    authority_ref: String,
}

struct ArtifactBinding {
    symbols: BTreeMap<String, BindingRecord>,
    resources: BTreeMap<String, BindingRecord>,
}

impl GitRepositoryAdapter {
    pub fn new(
        root: &Path,
        manifest_path: &str,
        require_clean: bool,
    ) -> Result<Self, GitRepositoryError> {
        let root = root
            .canonicalize()
            .map_err(|error| git_error(format!("cannot resolve project root: {error}")))?;
        let manifest_path =
            repository_path(&root, manifest_path).map_err(|error| git_error(error.to_string()))?;
        Ok(Self {
            root,
            manifest_path,
            require_clean,
        })
    }

    pub fn observe(&self) -> Result<Value, GitRepositoryError> {
        self.assert_repository_state()?;
        let revision = self.git(&["rev-parse", "HEAD"]).map_err(|_| {
            git_error(
                "Git repository has no initial commit.\nNext: review the generated Agentic files, then git add and commit them.",
            )
        })?;
        let manifest = self.read_manifest()?;
        let phase = manifest
            .get("phase")
            .and_then(Value::as_str)
            .filter(|phase| matches!(*phase, "pre-build" | "post-build"))
            .ok_or_else(|| git_error("repository observation phase is invalid"))?;
        let analysis_roots = analysis_roots(
            manifest
                .get("analysis")
                .ok_or_else(|| git_error("repository analysis is missing"))?,
        )?;

        let mut artifacts = Vec::new();
        let mut artifact_refs = BTreeSet::new();
        let mut artifact_paths = BTreeSet::new();
        let mut facts = Vec::new();
        let mut analyzed_refs = Vec::new();
        let mut gaps = Vec::new();

        for declaration in required_array(&manifest, "artifacts", "repository observation")? {
            let declaration = declaration
                .as_object()
                .ok_or_else(|| git_error("artifact declaration must be a mapping"))?;
            assert_exact_fields(
                declaration,
                &["ref", "path", "language", "bindings"],
                "artifact declaration",
            )?;
            let reference = required_nonempty_string(declaration, "ref", "artifact declaration")?;
            if !artifact_refs.insert(reference.to_owned()) {
                return Err(git_error(format!("duplicate artifact ref: {reference:?}")));
            }
            let configured_path =
                required_nonempty_string(declaration, "path", "artifact declaration")?;
            if !is_under_analysis_root(configured_path, &analysis_roots) {
                return Err(git_error(format!(
                    "artifact is outside configured analysis roots: {configured_path}"
                )));
            }
            if !artifact_paths.insert(configured_path.to_owned()) {
                return Err(git_error(format!(
                    "duplicate artifact path: {configured_path}"
                )));
            }
            let language =
                required_nonempty_string(declaration, "language", "artifact declaration")?;
            let bindings = artifact_bindings(
                declaration
                    .get("bindings")
                    .ok_or_else(|| git_error("artifact declaration bindings is missing"))?,
            )?;

            let artifact_path = repository_path(&self.root, configured_path)
                .map_err(|error| git_error(error.to_string()))?;
            let artifact_relative = self.relative_string(&artifact_path)?;
            self.assert_tracked(&artifact_relative)?;
            if !artifact_path.is_file() {
                return Err(git_error(format!(
                    "artifact is not a file: {artifact_relative}"
                )));
            }
            let source = fs::read_to_string(&artifact_path)
                .map_err(|error| git_error(format!("{}: {error}", artifact_path.display())))?;
            let content_digest = format!("sha256:{:x}", Sha256::digest(source.as_bytes()));
            let applies_to = binding_logical_refs(&bindings);
            let normalized_declaration = declaration.clone();
            let digest = canonical_digest(&json!({
                "content_digest": content_digest,
                "declaration": normalized_declaration,
            }))
            .map_err(|error| git_error(error.to_string()))?;

            let mut artifact_observations = Vec::new();
            if language != "python" {
                gaps.push(coverage_gap(
                    "unsupported-language",
                    Some(reference),
                    format!("language {language} is not supported"),
                ));
            } else {
                match observe_python(&source) {
                    Err(error) => gaps.push(coverage_gap("parse-error", Some(reference), error)),
                    Ok(observations) => {
                        analyzed_refs.push(reference.to_owned());
                        for observation in observations {
                            if observation.kind == PythonObservationKind::OtherMethodCall
                                && !bindings.resources.contains_key(&observation.resource)
                            {
                                continue;
                            }
                            artifact_observations.push(json!({
                                "kind": match observation.kind {
                                    PythonObservationKind::DbWrite => "db_write",
                                    PythonObservationKind::MessagePublish => "message_publish",
                                    PythonObservationKind::OtherMethodCall => "unsupported_method_call",
                                },
                                "symbol": &observation.symbol,
                                "resource": &observation.resource,
                                "method": &observation.method,
                                "line": observation.line,
                            }));
                            bind_observation(
                                &observation,
                                reference,
                                &bindings,
                                &mut facts,
                                &mut gaps,
                            );
                        }
                    }
                }
            }

            artifacts.push(json!({
                "ref": reference,
                "path": artifact_relative,
                "language": language,
                "applies_to": applies_to,
                "observations": artifact_observations,
                "content_digest": content_digest,
                "digest": digest,
            }));
        }

        for path in self.analysis_targets(&analysis_roots)? {
            if !artifact_paths.contains(&path) {
                gaps.push(coverage_gap(
                    "unbound-source-artifact",
                    None,
                    format!("Git source target has no artifact Binding Record: {path}"),
                ));
            }
        }

        artifacts.sort_by(|left, right| left["ref"].as_str().cmp(&right["ref"].as_str()));
        facts.sort_by_key(canonical_value);
        analyzed_refs.sort();
        sort_gaps(&mut gaps);
        let coverage = json!({
            "status": if gaps.is_empty() { "complete" } else { "incomplete" },
            "scope": "git-source-roots",
            "analyzed_refs": analyzed_refs,
            "gaps": gaps,
        });

        Ok(json!({
            "phase": phase,
            "revision": revision,
            "artifacts": artifacts,
            "facts": facts,
            "coverage": coverage,
        }))
    }

    /// Require authoritative project files to be present in the Git index.
    pub fn assert_tracked_paths(&self, paths: &[PathBuf]) -> Result<(), GitRepositoryError> {
        for path in paths {
            let relative = self.relative_string(path)?;
            self.assert_tracked(&relative)?;
        }
        Ok(())
    }

    fn assert_repository_state(&self) -> Result<(), GitRepositoryError> {
        let top_level = PathBuf::from(self.git(&["rev-parse", "--show-toplevel"])?)
            .canonicalize()
            .map_err(|error| git_error(format!("cannot resolve Git top-level: {error}")))?;
        if top_level != self.root {
            return Err(git_error(format!(
                "configured root is not Git top-level: {}",
                self.root.display()
            )));
        }
        if self.require_clean
            && !self
                .git(&["status", "--porcelain", "--untracked-files=all"])?
                .is_empty()
        {
            return Err(git_error("Git working tree is not clean"));
        }
        Ok(())
    }

    fn read_manifest(&self) -> Result<Map<String, Value>, GitRepositoryError> {
        let manifest_relative = self.relative_string(&self.manifest_path)?;
        self.assert_tracked(&manifest_relative)?;
        let manifest_text = fs::read_to_string(&self.manifest_path)
            .map_err(|error| git_error(format!("{}: {error}", self.manifest_path.display())))?;
        let manifest: Value = serde_yaml::from_str(&manifest_text)
            .map_err(|error| git_error(format!("{}: {error}", self.manifest_path.display())))?;
        let manifest = manifest
            .as_object()
            .ok_or_else(|| git_error("repository observation must be a mapping"))?;
        if manifest.get("schema_version").and_then(Value::as_str)
            != Some(OBSERVATION_SCHEMA_VERSION)
        {
            return Err(git_error("unsupported repository observation schema"));
        }
        assert_exact_fields(
            manifest,
            &["schema_version", "phase", "analysis", "artifacts"],
            "repository observation",
        )?;
        Ok(manifest.clone())
    }

    fn analysis_targets(&self, roots: &[String]) -> Result<Vec<String>, GitRepositoryError> {
        let output = self.git(&[
            "ls-files",
            "--cached",
            "--others",
            "--exclude-standard",
            "--",
            "*.py",
        ])?;
        let mut targets: Vec<String> = output
            .lines()
            .filter(|path| is_under_analysis_root(path, roots))
            .map(str::to_owned)
            .collect();
        targets.sort();
        targets.dedup();
        Ok(targets)
    }

    fn assert_tracked(&self, relative: &str) -> Result<(), GitRepositoryError> {
        self.git(&["ls-files", "--error-unmatch", "--", relative])
            .map(|_| ())
            .map_err(|_| {
                git_error(format!(
                    "required project input is not tracked by Git: {relative}\nNext: git add {relative} and commit the reviewed file."
                ))
            })
    }

    fn relative_string(&self, path: &Path) -> Result<String, GitRepositoryError> {
        path.strip_prefix(&self.root)
            .map(|relative| {
                relative
                    .components()
                    .map(|component| component.as_os_str().to_string_lossy())
                    .collect::<Vec<_>>()
                    .join("/")
            })
            .map_err(|error| git_error(error.to_string()))
    }

    fn git(&self, arguments: &[&str]) -> Result<String, GitRepositoryError> {
        let output = Command::new("git")
            .arg("-C")
            .arg(&self.root)
            .args(arguments)
            .output()
            .map_err(|error| git_error(format!("cannot execute Git: {error}")))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
            let stdout = String::from_utf8_lossy(&output.stdout).trim().to_owned();
            let message = if stderr.is_empty() { stdout } else { stderr };
            return Err(git_error(format!(
                "Git command failed ({}): {message}",
                arguments.join(" ")
            )));
        }
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
    }
}

fn analysis_roots(value: &Value) -> Result<Vec<String>, GitRepositoryError> {
    let analysis = value
        .as_object()
        .ok_or_else(|| git_error("repository analysis must be a mapping"))?;
    assert_exact_fields(analysis, &["roots"], "repository analysis")?;
    let roots = string_array(
        analysis
            .get("roots")
            .ok_or_else(|| git_error("repository analysis roots is missing"))?,
        "repository analysis roots",
    )?;
    if roots.is_empty()
        || roots
            .iter()
            .any(|root| root.is_empty() || root.starts_with('/') || root.contains(".."))
    {
        return Err(git_error(
            "repository analysis roots must be non-empty safe relative paths",
        ));
    }
    Ok(roots)
}

fn is_under_analysis_root(path: &str, roots: &[String]) -> bool {
    roots
        .iter()
        .any(|root| root == "." || path == root || path.starts_with(&format!("{root}/")))
}

fn artifact_bindings(value: &Value) -> Result<ArtifactBinding, GitRepositoryError> {
    let bindings = value
        .as_object()
        .ok_or_else(|| git_error("artifact bindings must be a mapping"))?;
    assert_exact_fields(bindings, &["symbols", "resources"], "artifact bindings")?;
    Ok(ArtifactBinding {
        symbols: binding_map(
            bindings
                .get("symbols")
                .ok_or_else(|| git_error("artifact symbol bindings are missing"))?,
            "artifact symbol binding",
        )?,
        resources: binding_map(
            bindings
                .get("resources")
                .ok_or_else(|| git_error("artifact resource bindings are missing"))?,
            "artifact resource binding",
        )?,
    })
}

fn binding_map(
    value: &Value,
    label: &str,
) -> Result<BTreeMap<String, BindingRecord>, GitRepositoryError> {
    let records = value
        .as_object()
        .ok_or_else(|| git_error(format!("{label}s must be a mapping")))?;
    records
        .iter()
        .map(|(physical_name, value)| {
            if physical_name.is_empty() {
                return Err(git_error(format!("{label} name must not be empty")));
            }
            let record = value
                .as_object()
                .ok_or_else(|| git_error(format!("{label} must be a mapping")))?;
            assert_exact_fields(record, &["logical_ref", "owner", "authority_ref"], label)?;
            Ok((
                physical_name.clone(),
                BindingRecord {
                    logical_ref: required_nonempty_string(record, "logical_ref", label)?.to_owned(),
                    owner: required_nonempty_string(record, "owner", label)?.to_owned(),
                    authority_ref: required_nonempty_string(record, "authority_ref", label)?
                        .to_owned(),
                },
            ))
        })
        .collect()
}

fn binding_logical_refs(bindings: &ArtifactBinding) -> Vec<String> {
    bindings
        .symbols
        .values()
        .chain(bindings.resources.values())
        .map(|record| record.logical_ref.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn bind_observation(
    observation: &crate::python_detection::PythonObservation,
    artifact_ref: &str,
    bindings: &ArtifactBinding,
    facts: &mut Vec<Value>,
    gaps: &mut Vec<Value>,
) {
    let Some(symbol) = bindings.symbols.get(&observation.symbol) else {
        gaps.push(coverage_gap(
            "unmapped-observation",
            Some(artifact_ref),
            format!(
                "function {} at line {} has no symbol binding",
                observation.symbol, observation.line
            ),
        ));
        return;
    };
    let Some(resource) = bindings.resources.get(&observation.resource) else {
        gaps.push(coverage_gap(
            "unmapped-observation",
            Some(artifact_ref),
            format!(
                "resource {} at line {} has no resource binding",
                observation.resource, observation.line
            ),
        ));
        return;
    };
    if observation.kind == PythonObservationKind::OtherMethodCall {
        gaps.push(coverage_gap(
            "unsupported-observation",
            Some(artifact_ref),
            format!(
                "bound resource {} uses unsupported method {} at line {}",
                observation.resource, observation.method, observation.line
            ),
        ));
        return;
    }
    if !symbol.logical_ref.starts_with("operation.") {
        gaps.push(coverage_gap(
            "invalid-binding",
            Some(artifact_ref),
            format!(
                "symbol {} must map to an operation.* logical ref",
                observation.symbol
            ),
        ));
        return;
    }
    let (resource_field, required_prefix) = match observation.kind {
        PythonObservationKind::DbWrite => ("data", "data."),
        PythonObservationKind::MessagePublish => ("integration", "integration."),
        PythonObservationKind::OtherMethodCall => unreachable!("handled above"),
    };
    if !resource.logical_ref.starts_with(required_prefix) {
        gaps.push(coverage_gap(
            "invalid-binding",
            Some(artifact_ref),
            format!(
                "resource {} must map to a {}* logical ref",
                observation.resource, required_prefix
            ),
        ));
        return;
    }
    let evidence_refs: Vec<&str> = BTreeSet::from([
        artifact_ref,
        symbol.authority_ref.as_str(),
        resource.authority_ref.as_str(),
    ])
    .into_iter()
    .collect();
    facts.push(json!({
        "kind": match observation.kind {
            PythonObservationKind::DbWrite => "db_write",
            PythonObservationKind::MessagePublish => "message_publish",
            PythonObservationKind::OtherMethodCall => unreachable!("handled above"),
        },
        "operation": symbol.logical_ref,
        (resource_field): resource.logical_ref,
        "evidence_refs": evidence_refs,
        "binding_authority_refs": [
            symbol.authority_ref,
            resource.authority_ref,
        ],
        "binding_owners": [
            symbol.owner,
            resource.owner,
        ],
    }));
}

fn coverage_gap(kind: &str, reference: Option<&str>, reason: String) -> Value {
    let mut gap = Map::from_iter([
        ("kind".to_owned(), Value::String(kind.to_owned())),
        ("reason".to_owned(), Value::String(reason)),
    ]);
    if let Some(reference) = reference {
        gap.insert("ref".to_owned(), Value::String(reference.to_owned()));
    }
    Value::Object(gap)
}

fn sort_gaps(gaps: &mut [Value]) {
    gaps.sort_by(|left, right| {
        (
            left["kind"].as_str(),
            left["ref"].as_str().unwrap_or(""),
            left["reason"].as_str(),
        )
            .cmp(&(
                right["kind"].as_str(),
                right["ref"].as_str().unwrap_or(""),
                right["reason"].as_str(),
            ))
    });
}

fn canonical_value(value: &Value) -> String {
    serde_json::to_string(value).expect("in-memory JSON value can be serialized")
}

fn required_array<'a>(
    object: &'a Map<String, Value>,
    field: &str,
    label: &str,
) -> Result<&'a [Value], GitRepositoryError> {
    object
        .get(field)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .ok_or_else(|| git_error(format!("{label} {field} must be a list")))
}

fn assert_exact_fields(
    object: &Map<String, Value>,
    expected: &[&str],
    label: &str,
) -> Result<(), GitRepositoryError> {
    let actual: BTreeSet<&str> = object.keys().map(String::as_str).collect();
    let expected: BTreeSet<&str> = expected.iter().copied().collect();
    if actual == expected {
        Ok(())
    } else {
        Err(git_error(format!(
            "{label} must contain only {}",
            expected.into_iter().collect::<Vec<_>>().join(", ")
        )))
    }
}

fn required_nonempty_string<'a>(
    object: &'a Map<String, Value>,
    field: &str,
    label: &str,
) -> Result<&'a str, GitRepositoryError> {
    object
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| git_error(format!("{label} {field} must be a non-empty string")))
}

fn string_array(value: &Value, label: &str) -> Result<Vec<String>, GitRepositoryError> {
    value
        .as_array()
        .ok_or_else(|| git_error(format!("{label} must be a list")))?
        .iter()
        .map(|item| {
            item.as_str()
                .map(str::to_owned)
                .ok_or_else(|| git_error(format!("{label} items must be strings")))
        })
        .collect()
}

fn git_error(message: impl Into<String>) -> GitRepositoryError {
    GitRepositoryError {
        message: message.into(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitRepositoryError {
    message: String,
}

impl fmt::Display for GitRepositoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for GitRepositoryError {}
