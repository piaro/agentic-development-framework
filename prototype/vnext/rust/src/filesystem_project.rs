//! Filesystem-backed storage for project-owned, Git-managed Records.
//!
//! Repository observations are injected and remain outside this Store. Git
//! tracking and revision checks belong to a separate Repository Adapter.

use crate::application::{
    ProjectStore, ProjectStoreError, merge_contract_clause_update, validate_contract_update,
    validate_decision_update,
};
use crate::contract_health::{ContractHealthReport, build_contract_health_report};
use crate::kernel::ProjectSnapshot;
use crate::project::build_project_snapshot;
use crate::schema::SchemaRegistry;
use regex::Regex;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};

pub const DEFAULT_CONTRACT_ROOT: &str = "contracts";
pub const DEFAULT_DECISION_ROOT: &str = "decisions";
use std::sync::atomic::{AtomicU64, Ordering};

const DISALLOWED_SOURCE_ROOTS: [&str; 5] = [
    ".agentic/cache",
    ".agentic/bundles",
    ".agentic/local",
    ".agentic/logs",
    ".agentic/tmp",
];
pub const FILESYSTEM_PROJECT_PROTOCOL_VERSION: &str = "3";
static TEMPORARY_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocumentFormat {
    Auto,
    Yaml,
    Markdown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FileFormat {
    Json,
    Yaml,
    Markdown(&'static str),
}

struct InitializationTarget {
    path: PathBuf,
    value: Value,
    format: FileFormat,
}

pub struct FileProjectStore<'a> {
    project_root: PathBuf,
    contract_root: PathBuf,
    decision_root: PathBuf,
    change_root: PathBuf,
    repository: Value,
    default_document_format: DocumentFormat,
    schema_registry: &'a SchemaRegistry,
}

impl<'a> FileProjectStore<'a> {
    pub fn open(
        root: impl AsRef<Path>,
        repository: Value,
        schema_registry: &'a SchemaRegistry,
    ) -> Result<Self, FileProjectError> {
        Self::open_with_options(
            root,
            repository,
            DEFAULT_CONTRACT_ROOT,
            DEFAULT_DECISION_ROOT,
            DocumentFormat::Auto,
            schema_registry,
        )
    }

    pub fn open_with_options(
        root: impl AsRef<Path>,
        repository: Value,
        contract_root: &str,
        decision_root: &str,
        default_document_format: DocumentFormat,
        schema_registry: &'a SchemaRegistry,
    ) -> Result<Self, FileProjectError> {
        let root = root
            .as_ref()
            .canonicalize()
            .map_err(|error| file_error(format!("project root does not exist: {error}")))?;
        if !root.is_dir() {
            return Err(file_error(format!(
                "project root does not exist: {}",
                root.display()
            )));
        }
        let contract_root = source_root(&root, contract_root)?;
        let decision_root = source_root(&root, decision_root)?;
        let change_root = root.join(".agentic").join("changes");
        Ok(Self {
            project_root: root,
            contract_root,
            decision_root,
            change_root,
            repository,
            default_document_format,
            schema_registry,
        })
    }

    pub fn initialize(
        root: impl AsRef<Path>,
        project: &Value,
        document_format: DocumentFormat,
        schema_registry: &'a SchemaRegistry,
    ) -> Result<Self, FileProjectError> {
        if !matches!(
            document_format,
            DocumentFormat::Yaml | DocumentFormat::Markdown
        ) {
            return Err(file_error(
                "initial document format must be yaml or markdown",
            ));
        }
        fs::create_dir_all(root.as_ref())
            .map_err(|error| file_error(format!("cannot create project root: {error}")))?;
        let store = Self::open_with_options(
            root,
            project["repository"].clone(),
            DEFAULT_CONTRACT_ROOT,
            DEFAULT_DECISION_ROOT,
            document_format,
            schema_registry,
        )?;

        // Validate every Record before creating the first target file.
        for (collection, record_kind) in [
            ("changes", "change"),
            ("contracts", "contract"),
            ("decisions", "decision"),
            ("results", "result"),
            ("evidence", "evidence"),
        ] {
            for record in project_records(project, collection)? {
                schema_registry
                    .validate(record_kind, record)
                    .map_err(|error| file_error(error.to_string()))?;
            }
        }

        let mut targets = Vec::new();
        for change in project_records(project, "changes")? {
            let change_id = safe_id(required_string(change, "id", "Change")?)?;
            let (name, format) = match document_format {
                DocumentFormat::Markdown => ("change.md", FileFormat::Markdown("change")),
                DocumentFormat::Yaml => ("change.yaml", FileFormat::Yaml),
                DocumentFormat::Auto => unreachable!("validated above"),
            };
            targets.push(InitializationTarget {
                path: store.change_root.join(change_id).join(name),
                value: change.clone(),
                format,
            });
        }
        for (collection, root, record_kind) in [
            ("contracts", &store.contract_root, "contract"),
            ("decisions", &store.decision_root, "decision"),
        ] {
            for record in project_records(project, collection)? {
                let record_id = safe_id(required_string(record, "id", record_kind)?)?;
                let (extension, format) = match document_format {
                    DocumentFormat::Markdown => ("md", FileFormat::Markdown(record_kind)),
                    DocumentFormat::Yaml => ("yaml", FileFormat::Yaml),
                    DocumentFormat::Auto => unreachable!("validated above"),
                };
                targets.push(InitializationTarget {
                    path: root.join(format!("{record_id}.{extension}")),
                    value: record.clone(),
                    format,
                });
            }
        }
        for result in project_records(project, "results")? {
            let change_id = safe_id(required_string(result, "change_id", "Result")?)?;
            targets.push(InitializationTarget {
                path: store
                    .change_root
                    .join(change_id)
                    .join("results")
                    .join(result_filename(result)?),
                value: result.clone(),
                format: FileFormat::Json,
            });
        }
        for evidence in project_records(project, "evidence")? {
            let change_id = safe_id(required_string(evidence, "change_id", "Evidence")?)?;
            let evidence_id = safe_id(required_string(evidence, "id", "Evidence")?)?;
            targets.push(InitializationTarget {
                path: store
                    .change_root
                    .join(change_id)
                    .join("evidence")
                    .join(format!("{evidence_id}.json")),
                value: evidence.clone(),
                format: FileFormat::Json,
            });
        }

        let mut seen = BTreeSet::new();
        let mut duplicates = BTreeSet::new();
        for target in &targets {
            if !seen.insert(target.path.clone()) {
                duplicates.insert(target.path.clone());
            }
        }
        if !duplicates.is_empty() {
            return Err(file_error(format!(
                "project records resolve to duplicate files: {}",
                display_paths(&duplicates)
            )));
        }
        let conflicts = targets
            .iter()
            .filter(|target| target.path.exists())
            .map(|target| target.path.clone())
            .collect::<BTreeSet<_>>();
        if !conflicts.is_empty() {
            return Err(file_error(format!(
                "project initialization would overwrite existing files: {}",
                display_paths(&conflicts)
            )));
        }
        for target in targets {
            store.write_new(&target.path, &target.value, target.format)?;
        }
        Ok(store)
    }

    pub fn snapshot(&self, change_id: &str) -> Result<ProjectSnapshot, FileProjectError> {
        let change_id = safe_id(change_id)?;
        let change_path = self.change_path(change_id)?;
        let project = json!({
            "changes": [self.load_document(&change_path, "change")?],
            "contracts": self.load_document_records(&self.contract_root, "contract")?,
            "decisions": self.load_document_records(&self.decision_root, "decision")?,
            "results": self.load_json_records(
                &self.change_root.join(change_id).join("results")
            )?,
            "evidence": self.load_json_records(
                &self.change_root.join(change_id).join("evidence")
            )?,
            "repository": self.repository,
        });
        build_project_snapshot(&project, change_id, self.schema_registry)
            .map_err(|error| file_error(error.to_string()))
    }

    pub fn contract_health(&self) -> Result<ContractHealthReport, FileProjectError> {
        let project = self.repository_project()?;
        build_contract_health_report(&project, self.schema_registry)
            .map_err(|error| file_error(error.to_string()))
    }

    pub fn record_paths(&self, change_id: &str) -> Result<Vec<PathBuf>, FileProjectError> {
        let change_id = safe_id(change_id)?;
        let change_directory = self.change_root.join(change_id);
        let mut paths = vec![self.change_path(change_id)?];
        paths.extend(document_paths(&self.contract_root)?);
        paths.extend(document_paths(&self.decision_root)?);
        paths.extend(json_paths(&change_directory.join("results"))?);
        paths.extend(json_paths(&change_directory.join("evidence"))?);
        Ok(paths)
    }

    pub fn all_record_paths(&self) -> Result<Vec<PathBuf>, FileProjectError> {
        let mut paths = document_paths(&self.contract_root)?;
        paths.extend(document_paths(&self.decision_root)?);
        paths.extend(
            document_paths(&self.change_root)?
                .into_iter()
                .filter(|path| is_change_document(path)),
        );
        paths.extend(
            recursive_json_paths(&self.change_root)?
                .into_iter()
                .filter(|path| is_change_record_json(path)),
        );
        paths.sort();
        Ok(paths)
    }

    pub fn append_result(&mut self, result: &Value) -> Result<(), FileProjectError> {
        self.schema_registry
            .validate("result", result)
            .map_err(|error| file_error(error.to_string()))?;
        let change_id = safe_id(required_string(result, "change_id", "Result")?)?;
        self.change_path(change_id)?;
        let path = self
            .change_root
            .join(change_id)
            .join("results")
            .join(result_filename(result)?);
        self.write_new(&path, result, FileFormat::Json)
    }

    pub fn upsert_contract(
        &mut self,
        contract: &Value,
        expected_digest: Option<&str>,
    ) -> Result<(), FileProjectError> {
        self.schema_registry
            .validate("contract", contract)
            .map_err(|error| file_error(error.to_string()))?;
        let _lock = self.acquire_contract_update_lock()?;
        let record_id = safe_id(required_string(contract, "id", "contract")?)?;
        let existing_path = self.record_path(&self.contract_root, record_id, "contract")?;
        let existing = existing_path
            .as_ref()
            .map(|path| self.load_document(path, "contract"))
            .transpose()?;
        validate_contract_update(existing.as_ref(), contract, expected_digest)
            .map_err(|error| file_error(error.to_string()))?;

        let (path, format) = if let Some(path) = existing_path {
            let format = if path.extension().and_then(|value| value.to_str()) == Some("md") {
                FileFormat::Markdown("contract")
            } else {
                FileFormat::Yaml
            };
            (path, format)
        } else {
            match self.preferred_document_format()? {
                DocumentFormat::Markdown => (
                    self.contract_root.join(format!("{record_id}.md")),
                    FileFormat::Markdown("contract"),
                ),
                DocumentFormat::Yaml => (
                    self.contract_root.join(format!("{record_id}.yaml")),
                    FileFormat::Yaml,
                ),
                DocumentFormat::Auto => unreachable!("preferred format resolves auto"),
            }
        };
        self.write_atomic(&path, contract, format)
    }

    pub fn upsert_contract_clauses(
        &mut self,
        contract: &Value,
        expected_clause_digests: &BTreeMap<String, String>,
    ) -> Result<(), FileProjectError> {
        self.schema_registry
            .validate("contract", contract)
            .map_err(|error| file_error(error.to_string()))?;
        let _lock = self.acquire_contract_update_lock()?;
        let record_id = safe_id(required_string(contract, "id", "contract")?)?;
        let existing_path = self.record_path(&self.contract_root, record_id, "contract")?;
        let existing = existing_path
            .as_ref()
            .map(|path| self.load_document(path, "contract"))
            .transpose()?;
        let merged =
            merge_contract_clause_update(existing.as_ref(), contract, expected_clause_digests)
                .map_err(|error| file_error(error.to_string()))?;
        self.schema_registry
            .validate("contract", &merged)
            .map_err(|error| file_error(error.to_string()))?;

        let (path, format) = if let Some(path) = existing_path {
            let format = if path.extension().and_then(|value| value.to_str()) == Some("md") {
                FileFormat::Markdown("contract")
            } else {
                FileFormat::Yaml
            };
            (path, format)
        } else {
            unreachable!("clause-scoped updates require an existing Contract")
        };
        self.write_atomic(&path, &merged, format)
    }

    pub fn upsert_decision(
        &mut self,
        decision: &Value,
        expected_digest: Option<&str>,
    ) -> Result<(), FileProjectError> {
        self.schema_registry
            .validate("decision", decision)
            .map_err(|error| file_error(error.to_string()))?;
        let _lock = self.acquire_contract_update_lock()?;
        let record_id = safe_id(required_string(decision, "id", "decision")?)?;
        let existing_path = self.record_path(&self.decision_root, record_id, "decision")?;
        let existing = existing_path
            .as_ref()
            .map(|path| self.load_document(path, "decision"))
            .transpose()?;
        validate_decision_update(existing.as_ref(), decision, expected_digest)
            .map_err(|error| file_error(error.to_string()))?;
        self.upsert_document_record(self.decision_root.clone(), decision, "decision")
    }

    pub fn add_evidence(&mut self, evidence: &Value) -> Result<(), FileProjectError> {
        self.schema_registry
            .validate("evidence", evidence)
            .map_err(|error| file_error(error.to_string()))?;
        let change_id = safe_id(required_string(evidence, "change_id", "Evidence")?)?;
        let evidence_id = safe_id(required_string(evidence, "id", "Evidence")?)?;
        self.change_path(change_id)?;
        let path = self
            .change_root
            .join(change_id)
            .join("evidence")
            .join(format!("{evidence_id}.json"));
        self.write_new(&path, evidence, FileFormat::Json)
    }

    pub fn update_repository(&mut self, repository: Value) {
        self.repository = repository;
    }

    fn upsert_document_record(
        &mut self,
        source_root: PathBuf,
        value: &Value,
        record_kind: &'static str,
    ) -> Result<(), FileProjectError> {
        let record_id = safe_id(required_string(value, "id", record_kind)?)?;
        let existing = self.record_path(&source_root, record_id, record_kind)?;
        let (path, format) = if let Some(path) = existing {
            let format = if path.extension().and_then(|value| value.to_str()) == Some("md") {
                FileFormat::Markdown(record_kind)
            } else {
                FileFormat::Yaml
            };
            (path, format)
        } else {
            match self.preferred_document_format()? {
                DocumentFormat::Markdown => (
                    source_root.join(format!("{record_id}.md")),
                    FileFormat::Markdown(record_kind),
                ),
                DocumentFormat::Yaml => (
                    source_root.join(format!("{record_id}.yaml")),
                    FileFormat::Yaml,
                ),
                DocumentFormat::Auto => unreachable!("preferred format resolves auto"),
            }
        };
        self.write_atomic(&path, value, format)
    }

    fn acquire_contract_update_lock(&self) -> Result<File, FileProjectError> {
        let directory = self
            .project_root
            .join(".agentic")
            .join("cache")
            .join("locks");
        fs::create_dir_all(&directory).map_err(|error| {
            file_error(format!("cannot create {}: {error}", directory.display()))
        })?;
        let path = directory.join("contract-updates.lock");
        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(&path)
            .map_err(|error| file_error(format!("{}: {error}", path.display())))?;
        file.lock().map_err(|error| {
            file_error(format!(
                "cannot lock Shared Contract updates for {}: {error}",
                self.project_root.display()
            ))
        })?;
        Ok(file)
    }

    fn record_path(
        &self,
        source_root: &Path,
        record_id: &str,
        record_kind: &str,
    ) -> Result<Option<PathBuf>, FileProjectError> {
        let mut matches = Vec::new();
        for path in document_paths(source_root)? {
            if self.load_document(&path, record_kind)?["id"].as_str() == Some(record_id) {
                matches.push(path);
            }
        }
        if matches.len() > 1 {
            return Err(file_error(format!("duplicate record id: {record_id}")));
        }
        Ok(matches.pop())
    }

    fn load_document_records(
        &self,
        source_root: &Path,
        record_kind: &str,
    ) -> Result<Vec<Value>, FileProjectError> {
        let values = document_paths(source_root)?
            .iter()
            .map(|path| self.load_document(path, record_kind))
            .collect::<Result<Vec<_>, _>>()?;
        assert_unique_ids(&values)?;
        Ok(values)
    }

    fn load_json_records(&self, source_root: &Path) -> Result<Vec<Value>, FileProjectError> {
        let values = json_paths(source_root)?
            .iter()
            .map(|path| read_json(path))
            .collect::<Result<Vec<_>, _>>()?;
        assert_unique_ids(&values)?;
        Ok(values)
    }

    fn load_document(&self, path: &Path, record_kind: &str) -> Result<Value, FileProjectError> {
        let text = fs::read_to_string(path)
            .map_err(|error| file_error(format!("{}: {error}", path.display())))?;
        if path.extension().and_then(|value| value.to_str()) == Some("md") {
            parse_markdown_record(&text, record_kind)
        } else {
            parse_yaml(&text, path)
        }
    }

    fn repository_project(&self) -> Result<Value, FileProjectError> {
        let changes = document_paths(&self.change_root)?
            .into_iter()
            .filter(|path| is_change_document(path))
            .map(|path| self.load_document(&path, "change"))
            .collect::<Result<Vec<_>, _>>()?;
        let record_paths = recursive_json_paths(&self.change_root)?;
        let results = record_paths
            .iter()
            .filter(|path| parent_name(path) == Some("results"))
            .map(|path| read_json(path))
            .collect::<Result<Vec<_>, _>>()?;
        let evidence = record_paths
            .iter()
            .filter(|path| parent_name(path) == Some("evidence"))
            .map(|path| read_json(path))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(json!({
            "changes": changes,
            "contracts": self.load_document_records(&self.contract_root, "contract")?,
            "decisions": self.load_document_records(&self.decision_root, "decision")?,
            "results": results,
            "evidence": evidence,
            "repository": self.repository,
        }))
    }

    fn change_path(&self, change_id: &str) -> Result<PathBuf, FileProjectError> {
        let directory = self.change_root.join(change_id);
        let candidates = ["change.md", "change.yaml"]
            .iter()
            .map(|name| directory.join(name))
            .filter(|path| path.is_file())
            .collect::<Vec<_>>();
        match candidates.as_slice() {
            [] => Err(file_error(format!("unknown change: {change_id}"))),
            [path] => Ok(path.clone()),
            _ => Err(file_error(format!("multiple Change records: {change_id}"))),
        }
    }

    fn preferred_document_format(&self) -> Result<DocumentFormat, FileProjectError> {
        if self.default_document_format != DocumentFormat::Auto {
            return Ok(self.default_document_format);
        }
        let markdown_exists = document_paths(&self.contract_root)?
            .iter()
            .chain(document_paths(&self.decision_root)?.iter())
            .any(|path| path.extension().and_then(|value| value.to_str()) == Some("md"))
            || change_markdown_exists(&self.change_root)?;
        Ok(if markdown_exists {
            DocumentFormat::Markdown
        } else {
            DocumentFormat::Yaml
        })
    }

    fn write_new(
        &self,
        path: &Path,
        value: &Value,
        format: FileFormat,
    ) -> Result<(), FileProjectError> {
        let serialized = serialize_record(value, format, None)?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| {
                file_error(format!("cannot create {}: {error}", parent.display()))
            })?;
        }
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .map_err(|error| {
                if error.kind() == std::io::ErrorKind::AlreadyExists {
                    file_error(format!("record already exists: {}", path.display()))
                } else {
                    file_error(format!("cannot create {}: {error}", path.display()))
                }
            })?;
        file.write_all(serialized.as_bytes())
            .and_then(|()| file.sync_all())
            .map_err(|error| file_error(format!("cannot write {}: {error}", path.display())))
    }

    fn write_atomic(
        &self,
        path: &Path,
        value: &Value,
        format: FileFormat,
    ) -> Result<(), FileProjectError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| {
                file_error(format!("cannot create {}: {error}", parent.display()))
            })?;
        }
        let existing = if path.is_file() && matches!(format, FileFormat::Markdown(_)) {
            Some(
                fs::read_to_string(path)
                    .map_err(|error| file_error(format!("{}: {error}", path.display())))?,
            )
        } else {
            None
        };
        let serialized = serialize_record(value, format, existing.as_deref())?;
        let temporary = temporary_path(path)?;
        let write_result = (|| {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)
                .map_err(|error| {
                    file_error(format!(
                        "cannot create temporary file {}: {error}",
                        temporary.display()
                    ))
                })?;
            file.write_all(serialized.as_bytes())
                .and_then(|()| file.sync_all())
                .map_err(|error| {
                    file_error(format!(
                        "cannot write temporary file {}: {error}",
                        temporary.display()
                    ))
                })?;
            fs::rename(&temporary, path).map_err(|error| {
                file_error(format!(
                    "cannot replace {} with {}: {error}",
                    path.display(),
                    temporary.display()
                ))
            })
        })();
        if temporary.exists() {
            let _ = fs::remove_file(&temporary);
        }
        write_result
    }
}

fn source_root(root: &Path, relative: &str) -> Result<PathBuf, FileProjectError> {
    let path = Path::new(relative);
    if path.is_absolute() {
        return Err(file_error(
            "project source root must be repository-relative",
        ));
    }
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(value) => parts.push(value.to_string_lossy().into_owned()),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(file_error(format!(
                    "project source root escapes repository: {relative}"
                )));
            }
        }
    }
    let normalized = parts.join("/");
    if DISALLOWED_SOURCE_ROOTS
        .iter()
        .any(|blocked| normalized == *blocked || normalized.starts_with(&format!("{blocked}/")))
    {
        return Err(file_error(format!(
            "project source root cannot use generated/local path: {relative}"
        )));
    }
    let mut candidate = root.to_path_buf();
    for part in parts {
        candidate.push(part);
        if candidate.exists() {
            candidate = candidate.canonicalize().map_err(|error| {
                file_error(format!("cannot resolve source root {relative}: {error}"))
            })?;
            if !candidate.starts_with(root) {
                return Err(file_error(format!(
                    "project source root escapes repository: {relative}"
                )));
            }
        }
    }
    Ok(candidate)
}

fn safe_id(value: &str) -> Result<&str, FileProjectError> {
    let mut characters = value.chars();
    let first = characters.next();
    let valid = first.is_some_and(|character| character.is_ascii_alphanumeric())
        && characters.all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-')
        });
    if !valid {
        return Err(file_error(format!("unsafe record id: {value:?}")));
    }
    Ok(value)
}

fn result_filename(result: &Value) -> Result<String, FileProjectError> {
    let action_id = safe_id(required_string(result, "action_id", "Result")?)?;
    let context_digest = required_string(result, "context_digest", "Result")?;
    let digest = context_digest
        .strip_prefix("sha256:")
        .filter(|value| {
            value.len() == 64
                && value.chars().all(|character| {
                    character.is_ascii_hexdigit() && !character.is_ascii_uppercase()
                })
        })
        .ok_or_else(|| file_error("unsafe Result context digest"))?;
    Ok(format!("{action_id}.{digest}.json"))
}

fn project_records<'a>(
    project: &'a Value,
    collection: &str,
) -> Result<&'a [Value], FileProjectError> {
    match project.get(collection) {
        None => Ok(&[]),
        Some(Value::Array(values)) => Ok(values),
        Some(_) => Err(file_error(format!("{collection} must be an array"))),
    }
}

fn required_string<'a>(
    value: &'a Value,
    field: &str,
    label: &str,
) -> Result<&'a str, FileProjectError> {
    value[field]
        .as_str()
        .ok_or_else(|| file_error(format!("{label} {field} must be a string")))
}

fn document_paths(root: &Path) -> Result<Vec<PathBuf>, FileProjectError> {
    let mut output = Vec::new();
    collect_document_paths(root, &mut output)?;
    output.sort();
    Ok(output)
}

fn collect_document_paths(root: &Path, output: &mut Vec<PathBuf>) -> Result<(), FileProjectError> {
    if !root.is_dir() {
        return Ok(());
    }
    let entries = fs::read_dir(root)
        .map_err(|error| file_error(format!("cannot list {}: {error}", root.display())))?;
    for entry in entries {
        let path = entry.map_err(|error| file_error(error.to_string()))?.path();
        if path.is_dir() {
            collect_document_paths(&path, output)?;
        } else if matches!(
            path.extension().and_then(|value| value.to_str()),
            Some("md" | "yaml")
        ) {
            output.push(path);
        }
    }
    Ok(())
}

fn json_paths(root: &Path) -> Result<Vec<PathBuf>, FileProjectError> {
    if !root.is_dir() {
        return Ok(Vec::new());
    }
    let mut output = fs::read_dir(root)
        .map_err(|error| file_error(format!("cannot list {}: {error}", root.display())))?
        .map(|entry| {
            entry
                .map(|value| value.path())
                .map_err(|error| file_error(error.to_string()))
        })
        .collect::<Result<Vec<_>, _>>()?;
    output.retain(|path| {
        path.is_file() && path.extension().and_then(|value| value.to_str()) == Some("json")
    });
    output.sort();
    Ok(output)
}

fn recursive_json_paths(root: &Path) -> Result<Vec<PathBuf>, FileProjectError> {
    let mut output = Vec::new();
    collect_json_paths(root, &mut output)?;
    output.sort();
    Ok(output)
}

fn collect_json_paths(root: &Path, output: &mut Vec<PathBuf>) -> Result<(), FileProjectError> {
    if !root.is_dir() {
        return Ok(());
    }
    let entries = fs::read_dir(root)
        .map_err(|error| file_error(format!("cannot list {}: {error}", root.display())))?;
    for entry in entries {
        let path = entry.map_err(|error| file_error(error.to_string()))?.path();
        if path.is_dir() {
            collect_json_paths(&path, output)?;
        } else if path.extension().and_then(|value| value.to_str()) == Some("json") {
            output.push(path);
        }
    }
    Ok(())
}

fn is_change_document(path: &Path) -> bool {
    matches!(
        path.file_name().and_then(|value| value.to_str()),
        Some("change.md" | "change.yaml")
    )
}

fn is_change_record_json(path: &Path) -> bool {
    matches!(parent_name(path), Some("results" | "evidence"))
}

fn parent_name(path: &Path) -> Option<&str> {
    path.parent()
        .and_then(Path::file_name)
        .and_then(|value| value.to_str())
}

fn assert_unique_ids(values: &[Value]) -> Result<(), FileProjectError> {
    let mut seen = BTreeSet::new();
    for value in values {
        let record_id = value["id"]
            .as_str()
            .ok_or_else(|| file_error("record id must be a string"))?;
        if !seen.insert(record_id) {
            return Err(file_error(format!("duplicate record id: {record_id}")));
        }
    }
    Ok(())
}

fn read_json(path: &Path) -> Result<Value, FileProjectError> {
    let mut bytes = Vec::new();
    File::open(path)
        .and_then(|mut file| file.read_to_end(&mut bytes))
        .map_err(|error| file_error(format!("{}: {error}", path.display())))?;
    let value: Value = serde_json::from_slice(&bytes)
        .map_err(|error| file_error(format!("{}: {error}", path.display())))?;
    require_object(value, path)
}

fn parse_yaml(text: &str, path: &Path) -> Result<Value, FileProjectError> {
    let value: Value = serde_yaml::from_str(text)
        .map_err(|error| file_error(format!("{}: {error}", path.display())))?;
    require_object(value, path)
}

fn parse_markdown_record(text: &str, record_kind: &str) -> Result<Value, FileProjectError> {
    let range = markdown_payload_range(text, record_kind)?;
    let value: Value = serde_yaml::from_str(&text[range]).map_err(|error| {
        file_error(format!(
            "{record_kind} Markdown payload is invalid: {error}"
        ))
    })?;
    if value["id"].as_str().is_none() {
        return Err(file_error(format!(
            "{record_kind} structured block requires string id"
        )));
    }
    Ok(value)
}

fn require_object(value: Value, path: &Path) -> Result<Value, FileProjectError> {
    if value.is_object() {
        Ok(value)
    } else {
        Err(file_error(format!(
            "record must be a mapping: {}",
            path.display()
        )))
    }
}

fn serialize_record(
    value: &Value,
    format: FileFormat,
    existing_text: Option<&str>,
) -> Result<String, FileProjectError> {
    match format {
        FileFormat::Json => serde_json::to_string_pretty(value)
            .map(|serialized| format!("{serialized}\n"))
            .map_err(|error| file_error(error.to_string())),
        FileFormat::Yaml => serialize_yaml(value),
        FileFormat::Markdown(record_kind) => {
            let payload = serialize_yaml(value)?;
            if let Some(existing) = existing_text {
                let range = markdown_payload_range(existing, record_kind)?;
                Ok(format!(
                    "{}{}{}",
                    &existing[..range.start],
                    payload,
                    &existing[range.end..]
                ))
            } else {
                let title = value["title"]
                    .as_str()
                    .or_else(|| value["id"].as_str())
                    .ok_or_else(|| file_error("Markdown record title is missing"))?;
                Ok(format!(
                    "# {title}\n\n\
                     This document is owned by the project. Human-readable rationale, \
                     examples, and diagrams may be added outside the structured block.\n\n\
                     ```agentic-{record_kind}\n{payload}```\n"
                ))
            }
        }
    }
}

fn serialize_yaml(value: &Value) -> Result<String, FileProjectError> {
    serde_yaml::to_string(value).map_err(|error| file_error(error.to_string()))
}

fn markdown_payload_range(
    text: &str,
    record_kind: &str,
) -> Result<std::ops::Range<usize>, FileProjectError> {
    let expression = Regex::new(&format!(
        r"(?ms)^```agentic-{}[ \t]*\n(?P<payload>.*?)^```[ \t]*$",
        regex::escape(record_kind)
    ))
    .expect("Framework-owned Markdown expression is valid");
    let matches = expression.captures_iter(text).collect::<Vec<_>>();
    if matches.len() != 1 {
        return Err(file_error(format!(
            "{record_kind} Markdown must contain exactly one agentic-{record_kind} block"
        )));
    }
    let payload = matches[0]
        .name("payload")
        .expect("expression defines payload");
    Ok(payload.start()..payload.end())
}

fn temporary_path(path: &Path) -> Result<PathBuf, FileProjectError> {
    let parent = path
        .parent()
        .ok_or_else(|| file_error("record path has no parent"))?;
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| file_error("record filename is not UTF-8"))?;
    for _ in 0..100 {
        let sequence = TEMPORARY_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let temporary = parent.join(format!(".{name}.{}.{}.tmp", std::process::id(), sequence));
        if !temporary.exists() {
            return Ok(temporary);
        }
    }
    Err(file_error("cannot allocate temporary record path"))
}

fn change_markdown_exists(root: &Path) -> Result<bool, FileProjectError> {
    if !root.is_dir() {
        return Ok(false);
    }
    for entry in fs::read_dir(root)
        .map_err(|error| file_error(format!("cannot list {}: {error}", root.display())))?
    {
        let directory = entry.map_err(|error| file_error(error.to_string()))?.path();
        if directory.join("change.md").is_file() {
            return Ok(true);
        }
    }
    Ok(false)
}

fn display_paths(paths: &BTreeSet<PathBuf>) -> String {
    paths
        .iter()
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

fn file_error(message: impl Into<String>) -> FileProjectError {
    FileProjectError {
        message: message.into(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileProjectError {
    message: String,
}

impl fmt::Display for FileProjectError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for FileProjectError {}

impl ProjectStore for FileProjectStore<'_> {
    fn snapshot(&self, change_id: &str) -> Result<ProjectSnapshot, ProjectStoreError> {
        FileProjectStore::snapshot(self, change_id)
            .map_err(|error| ProjectStoreError::new(error.to_string()))
    }

    fn contract_health(&self) -> Result<ContractHealthReport, ProjectStoreError> {
        FileProjectStore::contract_health(self)
            .map_err(|error| ProjectStoreError::new(error.to_string()))
    }

    fn append_result(&mut self, result: &Value) -> Result<(), ProjectStoreError> {
        FileProjectStore::append_result(self, result)
            .map_err(|error| ProjectStoreError::new(error.to_string()))
    }

    fn add_evidence(&mut self, evidence: &Value) -> Result<(), ProjectStoreError> {
        FileProjectStore::add_evidence(self, evidence)
            .map_err(|error| ProjectStoreError::new(error.to_string()))
    }

    fn upsert_decision(
        &mut self,
        decision: &Value,
        expected_digest: Option<&str>,
    ) -> Result<(), ProjectStoreError> {
        FileProjectStore::upsert_decision(self, decision, expected_digest)
            .map_err(|error| ProjectStoreError::new(error.to_string()))
    }

    fn upsert_contract(
        &mut self,
        contract: &Value,
        expected_digest: Option<&str>,
    ) -> Result<(), ProjectStoreError> {
        FileProjectStore::upsert_contract(self, contract, expected_digest)
            .map_err(|error| ProjectStoreError::new(error.to_string()))
    }

    fn upsert_contract_clauses(
        &mut self,
        contract: &Value,
        expected_clause_digests: &BTreeMap<String, String>,
    ) -> Result<(), ProjectStoreError> {
        FileProjectStore::upsert_contract_clauses(self, contract, expected_clause_digests)
            .map_err(|error| ProjectStoreError::new(error.to_string()))
    }

    fn update_repository(&mut self, repository: Value) -> Result<(), ProjectStoreError> {
        FileProjectStore::update_repository(self, repository);
        Ok(())
    }
}
