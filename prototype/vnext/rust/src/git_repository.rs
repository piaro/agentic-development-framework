//! Git-backed, fail-closed source observation.
//!
//! Git defines the mechanical analysis target. Language-specific parsing
//! extracts physical observations, then reviewed Binding Records map those
//! observations to stable project vocabulary.

use crate::canonical_digest;
use crate::project_config::repository_path;
use crate::signal_catalog::{RepositoryFactDefinition, SignalCatalogRegistry};
use crate::source_detection::{
    SourceObservation, SourceObservationKind, detector_for_language, detector_for_path,
    source_pathspecs,
};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

pub const OBSERVATION_SCHEMA_VERSION: &str = "5";
const LEGACY_OBSERVATION_SCHEMA_VERSION: &str = "4";

pub struct GitRepositoryAdapter {
    root: PathBuf,
    manifest_path: PathBuf,
    require_clean: bool,
    signal_registry: SignalCatalogRegistry,
}

#[derive(Clone)]
struct BindingRecord {
    logical_ref: String,
    owner: String,
    authority_ref: String,
}

struct ArtifactBinding {
    symbols: BTreeMap<String, BindingRecord>,
    resources: BTreeMap<String, ResourceBindingRecord>,
    methods: BTreeMap<String, MethodBindingRecord>,
}

#[derive(Clone)]
struct ResourceBindingRecord {
    logical_refs: BTreeMap<String, String>,
    owner: String,
    authority_ref: String,
}

struct MethodBindingRecord {
    fact_kinds: Vec<String>,
    owner: String,
    authority_ref: String,
}

impl GitRepositoryAdapter {
    pub fn new(
        root: &Path,
        manifest_path: &str,
        require_clean: bool,
    ) -> Result<Self, GitRepositoryError> {
        let signal_registry =
            SignalCatalogRegistry::built_in().map_err(|error| git_error(error.to_string()))?;
        Self::with_signal_registry(root, manifest_path, require_clean, signal_registry)
    }

    pub fn with_signal_registry(
        root: &Path,
        manifest_path: &str,
        require_clean: bool,
        signal_registry: SignalCatalogRegistry,
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
            signal_registry,
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
                &self.signal_registry,
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
            let Some(detector) = detector_for_language(language) else {
                gaps.push(coverage_gap(
                    "unsupported-language",
                    Some(reference),
                    format!("language {language} is not supported"),
                ));
                artifacts.push(json!({
                    "ref": reference,
                    "path": artifact_relative,
                    "language": language,
                    "applies_to": applies_to,
                    "observations": artifact_observations,
                    "content_digest": content_digest,
                    "digest": digest,
                }));
                continue;
            };
            if !detector.is_supported() {
                gaps.push(coverage_gap(
                    "unsupported-language",
                    Some(reference),
                    format!("language {language} is not supported"),
                ));
            } else {
                if detector_for_path(configured_path)
                    .is_some_and(|path_detector| path_detector.language != language)
                {
                    gaps.push(coverage_gap(
                        "language-path-mismatch",
                        Some(reference),
                        format!("language {language} does not match source path {configured_path}"),
                    ));
                }
                match detector.observe(&source) {
                    Err(error) => gaps.push(coverage_gap("parse-error", Some(reference), error)),
                    Ok(observations) => {
                        analyzed_refs.push(reference.to_owned());
                        let symbol_aliases = symbol_aliases(&observations);
                        for observation in observations {
                            if observation.kind == SourceObservationKind::OtherMethodCall
                                && !bindings.resources.contains_key(&observation.resource)
                            {
                                continue;
                            }
                            artifact_observations.push(json!({
                                "kind": match observation.kind {
                                    SourceObservationKind::DbWrite => "db_write",
                                    SourceObservationKind::MessagePublish => "message_publish",
                                    SourceObservationKind::OtherMethodCall => "unsupported_method_call",
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
                                &symbol_aliases,
                                &self.signal_registry,
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
                let language = detector_for_path(&path)
                    .map(|detector| detector.language)
                    .unwrap_or("unknown");
                gaps.push(coverage_gap(
                    "unbound-source-artifact",
                    None,
                    format!("Git {language} source target has no artifact Binding Record: {path}"),
                ));
            }
        }

        artifacts.sort_by(|left, right| left["ref"].as_str().cmp(&right["ref"].as_str()));
        facts.sort_by_key(canonical_value);
        facts.dedup();
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

    pub fn binding_authority_refs(&self) -> Result<Vec<String>, GitRepositoryError> {
        let manifest = self.read_manifest()?;
        let mut authority_refs = BTreeSet::new();
        for declaration in required_array(&manifest, "artifacts", "repository observation")? {
            let declaration = declaration
                .as_object()
                .ok_or_else(|| git_error("artifact declaration must be a mapping"))?;
            let bindings = artifact_bindings(
                declaration
                    .get("bindings")
                    .ok_or_else(|| git_error("artifact declaration bindings is missing"))?,
                &self.signal_registry,
            )?;
            authority_refs.extend(
                bindings
                    .symbols
                    .values()
                    .map(|binding| binding.authority_ref.clone()),
            );
            authority_refs.extend(
                bindings
                    .resources
                    .values()
                    .map(|binding| binding.authority_ref.clone()),
            );
            authority_refs.extend(
                bindings
                    .methods
                    .values()
                    .map(|binding| binding.authority_ref.clone()),
            );
        }
        Ok(authority_refs.into_iter().collect())
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
        if !matches!(
            manifest.get("schema_version").and_then(Value::as_str),
            Some(OBSERVATION_SCHEMA_VERSION | LEGACY_OBSERVATION_SCHEMA_VERSION)
        ) {
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
        let pathspecs = source_pathspecs();
        let mut arguments = vec![
            "ls-files",
            "--cached",
            "--others",
            "--exclude-standard",
            "--",
        ];
        arguments.extend(pathspecs.iter().map(String::as_str));
        let output = self.git(&arguments)?;
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

fn artifact_bindings(
    value: &Value,
    signal_registry: &SignalCatalogRegistry,
) -> Result<ArtifactBinding, GitRepositoryError> {
    let bindings = value
        .as_object()
        .ok_or_else(|| git_error("artifact bindings must be a mapping"))?;
    assert_exact_fields(
        bindings,
        &["symbols", "resources", "methods"],
        "artifact bindings",
    )?;
    Ok(ArtifactBinding {
        symbols: binding_map(
            bindings
                .get("symbols")
                .ok_or_else(|| git_error("artifact symbol bindings are missing"))?,
            "artifact symbol binding",
        )?,
        resources: resource_binding_map(
            bindings
                .get("resources")
                .ok_or_else(|| git_error("artifact resource bindings are missing"))?,
            "artifact resource binding",
        )?,
        methods: method_binding_map(
            bindings
                .get("methods")
                .ok_or_else(|| git_error("artifact method bindings are missing"))?,
            signal_registry,
        )?,
    })
}

fn method_binding_map(
    value: &Value,
    signal_registry: &SignalCatalogRegistry,
) -> Result<BTreeMap<String, MethodBindingRecord>, GitRepositoryError> {
    let records = value
        .as_object()
        .ok_or_else(|| git_error("artifact method bindings must be a mapping"))?;
    records
        .iter()
        .map(|(physical_call, value)| {
            if physical_call.is_empty() || !physical_call.contains('.') {
                return Err(git_error(
                    "artifact method binding name must be resource.method",
                ));
            }
            let record = value
                .as_object()
                .ok_or_else(|| git_error("artifact method binding must be a mapping"))?;
            let fact_kinds = if record.contains_key("fact_kinds") {
                assert_exact_fields(
                    record,
                    &["fact_kinds", "owner", "authority_ref"],
                    "artifact method binding",
                )?;
                nonempty_unique_string_array(
                    record.get("fact_kinds").ok_or_else(|| {
                        git_error("artifact method binding fact_kinds is missing")
                    })?,
                    "artifact method binding fact_kinds",
                )?
            } else {
                assert_exact_fields(
                    record,
                    &["kind", "owner", "authority_ref"],
                    "artifact method binding",
                )?;
                vec![
                    required_nonempty_string(record, "kind", "artifact method binding")?.to_owned(),
                ]
            };
            for kind in &fact_kinds {
                let definition = signal_registry
                    .repository_fact_definition(kind)
                    .ok_or_else(|| {
                        git_error(format!(
                            "artifact method binding kind is not supported: {kind}"
                        ))
                    })?;
                source_fact_bindings(definition)?;
            }
            Ok((
                physical_call.clone(),
                MethodBindingRecord {
                    fact_kinds,
                    owner: required_nonempty_string(record, "owner", "artifact method binding")?
                        .to_owned(),
                    authority_ref: required_nonempty_string(
                        record,
                        "authority_ref",
                        "artifact method binding",
                    )?
                    .to_owned(),
                },
            ))
        })
        .collect()
}

fn resource_binding_map(
    value: &Value,
    label: &str,
) -> Result<BTreeMap<String, ResourceBindingRecord>, GitRepositoryError> {
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
            let logical_refs = if record.contains_key("logical_refs") {
                assert_exact_fields(record, &["logical_refs", "owner", "authority_ref"], label)?;
                logical_ref_map(
                    record
                        .get("logical_refs")
                        .ok_or_else(|| git_error(format!("{label} logical_refs is missing")))?,
                    label,
                )?
            } else {
                assert_exact_fields(record, &["logical_ref", "owner", "authority_ref"], label)?;
                let logical_ref = required_nonempty_string(record, "logical_ref", label)?;
                let (binding, _) = logical_ref.split_once('.').ok_or_else(|| {
                    git_error(format!("{label} logical_ref must contain a binding prefix"))
                })?;
                BTreeMap::from([(binding.to_owned(), logical_ref.to_owned())])
            };
            Ok((
                physical_name.clone(),
                ResourceBindingRecord {
                    logical_refs,
                    owner: required_nonempty_string(record, "owner", label)?.to_owned(),
                    authority_ref: required_nonempty_string(record, "authority_ref", label)?
                        .to_owned(),
                },
            ))
        })
        .collect()
}

fn logical_ref_map(
    value: &Value,
    label: &str,
) -> Result<BTreeMap<String, String>, GitRepositoryError> {
    let values = value
        .as_object()
        .ok_or_else(|| git_error(format!("{label} logical_refs must be a mapping")))?;
    if values.is_empty() {
        return Err(git_error(format!("{label} logical_refs must not be empty")));
    }
    values
        .iter()
        .map(|(binding, value)| {
            let logical_ref = value
                .as_str()
                .filter(|value| !value.is_empty())
                .ok_or_else(|| {
                    git_error(format!(
                        "{label} logical_refs values must be non-empty strings"
                    ))
                })?;
            if binding.is_empty() || !logical_ref.starts_with(&format!("{binding}.")) {
                return Err(git_error(format!(
                    "{label} logical_refs {binding:?} must map to a {binding}.* logical ref"
                )));
            }
            Ok((binding.clone(), logical_ref.to_owned()))
        })
        .collect()
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
        .map(|record| record.logical_ref.clone())
        .chain(
            bindings
                .resources
                .values()
                .flat_map(|record| record.logical_refs.values().cloned()),
        )
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn bind_observation(
    observation: &SourceObservation,
    artifact_ref: &str,
    bindings: &ArtifactBinding,
    symbol_aliases: &BTreeMap<String, BTreeSet<String>>,
    signal_registry: &SignalCatalogRegistry,
    facts: &mut Vec<Value>,
    gaps: &mut Vec<Value>,
) {
    let short_symbol = unqualified_symbol(&observation.symbol);
    let symbol = bindings.symbols.get(&observation.symbol).or_else(|| {
        symbol_aliases
            .get(short_symbol)
            .filter(|qualified| qualified.len() == 1)
            .and_then(|_| bindings.symbols.get(short_symbol))
    });
    let Some(symbol) = symbol else {
        let ambiguity = symbol_aliases
            .get(short_symbol)
            .filter(|qualified| qualified.len() > 1);
        if let Some(qualified) = ambiguity
            && bindings.symbols.contains_key(short_symbol)
        {
            gaps.push(coverage_gap(
                "ambiguous-symbol-binding",
                Some(artifact_ref),
                format!(
                    "symbol binding {short_symbol} matches multiple physical symbols: {}",
                    qualified.iter().cloned().collect::<Vec<_>>().join(", ")
                ),
            ));
            return;
        }
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
    let method_key = format!("{}.{}", observation.resource, observation.method);
    let method_binding = bindings.methods.get(&method_key);
    let fact_kinds = if observation.kind == SourceObservationKind::OtherMethodCall {
        let Some(method_binding) = method_binding else {
            gaps.push(coverage_gap(
                "unsupported-observation",
                Some(artifact_ref),
                format!(
                    "bound resource {} uses unsupported method {} at line {}",
                    observation.resource, observation.method, observation.line
                ),
            ));
            return;
        };
        method_binding.fact_kinds.clone()
    } else {
        vec![match observation.kind {
            SourceObservationKind::DbWrite => "db_write".to_owned(),
            SourceObservationKind::MessagePublish => "message_publish".to_owned(),
            SourceObservationKind::OtherMethodCall => unreachable!("handled above"),
        }]
    };
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
    let mut resolved = Vec::new();
    for fact_kind in &fact_kinds {
        let Some(fact_definition) = signal_registry.repository_fact_definition(fact_kind) else {
            gaps.push(coverage_gap(
                "unsupported-observation",
                Some(artifact_ref),
                format!("repository fact kind {fact_kind} is not defined by the Signal Catalog"),
            ));
            return;
        };
        let Ok((operation_binding, resource_binding)) = source_fact_bindings(fact_definition)
        else {
            gaps.push(coverage_gap(
                "unsupported-observation",
                Some(artifact_ref),
                format!(
                    "repository fact kind {fact_kind} cannot be generated from a source method call"
                ),
            ));
            return;
        };
        let Some(resource_logical_ref) = resource.logical_refs.get(&resource_binding.binding)
        else {
            gaps.push(coverage_gap(
                "invalid-binding",
                Some(artifact_ref),
                format!(
                    "resource {} has no {} logical ref required by {fact_kind}",
                    observation.resource, resource_binding.binding
                ),
            ));
            return;
        };
        resolved.push((
            fact_kind,
            operation_binding,
            resource_binding,
            resource_logical_ref,
        ));
    }
    let mut evidence_refs = BTreeSet::from([
        artifact_ref,
        symbol.authority_ref.as_str(),
        resource.authority_ref.as_str(),
    ]);
    let mut binding_authority_refs = BTreeSet::from([
        symbol.authority_ref.as_str(),
        resource.authority_ref.as_str(),
    ]);
    let mut binding_owners = BTreeSet::from([symbol.owner.as_str(), resource.owner.as_str()]);
    if let Some(method_binding) = method_binding {
        evidence_refs.insert(method_binding.authority_ref.as_str());
        binding_authority_refs.insert(method_binding.authority_ref.as_str());
        binding_owners.insert(method_binding.owner.as_str());
    }
    let evidence_refs = evidence_refs.into_iter().collect::<Vec<_>>();
    let binding_authority_refs = binding_authority_refs.into_iter().collect::<Vec<_>>();
    let binding_owners = binding_owners.into_iter().collect::<Vec<_>>();
    for (fact_kind, operation_binding, resource_binding, resource_logical_ref) in resolved {
        let fact = Map::from_iter([
            ("kind".to_owned(), Value::String(fact_kind.to_owned())),
            (
                operation_binding.fact_field.clone(),
                Value::String(symbol.logical_ref.clone()),
            ),
            (
                resource_binding.fact_field.clone(),
                Value::String(resource_logical_ref.clone()),
            ),
            ("evidence_refs".to_owned(), json!(evidence_refs)),
            (
                "binding_authority_refs".to_owned(),
                json!(binding_authority_refs),
            ),
            ("binding_owners".to_owned(), json!(binding_owners)),
        ]);
        facts.push(Value::Object(fact));
    }
}

fn source_fact_bindings(
    definition: &RepositoryFactDefinition,
) -> Result<
    (
        &crate::signal_catalog::FactBindingDefinition,
        &crate::signal_catalog::FactBindingDefinition,
    ),
    GitRepositoryError,
> {
    let operation = definition
        .bindings
        .iter()
        .find(|binding| binding.binding == "operation");
    let resources = definition
        .bindings
        .iter()
        .filter(|binding| binding.binding != "operation")
        .collect::<Vec<_>>();
    match (operation, resources.as_slice()) {
        (Some(operation), [resource]) => Ok((operation, *resource)),
        _ => Err(git_error(format!(
            "repository fact kind {} must define operation and exactly one resource binding",
            definition.id
        ))),
    }
}

fn symbol_aliases(observations: &[SourceObservation]) -> BTreeMap<String, BTreeSet<String>> {
    let mut aliases = BTreeMap::<String, BTreeSet<String>>::new();
    for observation in observations {
        aliases
            .entry(unqualified_symbol(&observation.symbol).to_owned())
            .or_default()
            .insert(observation.symbol.clone());
    }
    aliases
}

fn unqualified_symbol(symbol: &str) -> &str {
    symbol
        .rsplit_once("::")
        .map(|(_, name)| name)
        .or_else(|| symbol.rsplit_once(['.', '#']).map(|(_, name)| name))
        .unwrap_or(symbol)
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

fn nonempty_unique_string_array(
    value: &Value,
    label: &str,
) -> Result<Vec<String>, GitRepositoryError> {
    let values = string_array(value, label)?;
    if values.is_empty() || values.iter().any(String::is_empty) {
        return Err(git_error(format!(
            "{label} must contain at least one non-empty string"
        )));
    }
    if values.iter().collect::<BTreeSet<_>>().len() != values.len() {
        return Err(git_error(format!("{label} must not contain duplicates")));
    }
    Ok(values)
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
