//! Load one real project and connect its configured sources to the Application.

use crate::application::{Application, ApplicationError};
use crate::binding_validation::BindingValidationReport;
use crate::delivery::{TRUST_STORE_PATH, resolve_verified_release};
use crate::filesystem_project::{DocumentFormat, FileProjectStore};
use crate::git_repository::GitRepositoryAdapter;
use crate::project_config::{ProjectConfig, load_project_config, repository_path};
use crate::remote_delivery::RELEASE_SOURCES_PATH;
use crate::schema::SchemaRegistry;
use crate::signal_catalog::SignalCatalogRegistry;
use serde_json::Value;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

pub struct LoadedProject {
    root: PathBuf,
    config: ProjectConfig,
    repository: Value,
    rule_source: Value,
    framework_lock: Value,
    schema_registry: SchemaRegistry,
    signal_registry: SignalCatalogRegistry,
    release_id: String,
    release_root: PathBuf,
}

impl LoadedProject {
    pub fn load(
        project_root: impl AsRef<Path>,
        release_root: Option<&Path>,
        require_clean: bool,
    ) -> Result<Self, ProjectRuntimeError> {
        let root = project_root
            .as_ref()
            .canonicalize()
            .map_err(|error| runtime_error(format!("cannot resolve project root: {error}")))?;
        let config =
            load_project_config(&root).map_err(|error| runtime_error(error.to_string()))?;
        let framework_lock = read_yaml(root.join(".agentic/framework.lock"))?;
        let release = resolve_verified_release(&root, &framework_lock, release_root)
            .map_err(|error| runtime_error(error.to_string()))?;
        let signal_registry =
            SignalCatalogRegistry::built_in().map_err(|error| runtime_error(error.to_string()))?;
        let repository = GitRepositoryAdapter::with_signal_registry(
            &root,
            &config.repository_observation,
            require_clean,
            signal_registry.clone(),
        )
        .and_then(|adapter| adapter.observe())
        .map_err(|error| runtime_error(error.to_string()))?;
        Ok(Self {
            root,
            config,
            repository,
            rule_source: release.rule_source,
            framework_lock,
            schema_registry: release.schema_registry,
            signal_registry,
            release_id: release.release_id,
            release_root: release.root,
        })
    }

    pub fn release_id(&self) -> &str {
        &self.release_id
    }

    pub fn release_root(&self) -> &Path {
        &self.release_root
    }

    pub fn binding_validation_report(
        &self,
    ) -> Result<BindingValidationReport, ProjectRuntimeError> {
        let store = FileProjectStore::open_with_options(
            &self.root,
            self.repository.clone(),
            &self.config.contract_root,
            &self.config.decision_root,
            DocumentFormat::Auto,
            &self.schema_registry,
        )
        .map_err(|error| runtime_error(error.to_string()))?;
        let decisions = store
            .decisions()
            .map_err(|error| runtime_error(error.to_string()))?;
        let binding_authority_refs = GitRepositoryAdapter::with_signal_registry(
            &self.root,
            &self.config.repository_observation,
            false,
            self.signal_registry.clone(),
        )
        .and_then(|adapter| adapter.binding_authority_refs())
        .map_err(|error| runtime_error(error.to_string()))?;
        Ok(BindingValidationReport::build(
            &self.repository,
            &decisions,
            &binding_authority_refs,
        ))
    }

    pub fn decisions(&self) -> Result<Vec<Value>, ProjectRuntimeError> {
        FileProjectStore::open_with_options(
            &self.root,
            self.repository.clone(),
            &self.config.contract_root,
            &self.config.decision_root,
            DocumentFormat::Auto,
            &self.schema_registry,
        )
        .and_then(|store| store.decisions())
        .map_err(|error| runtime_error(error.to_string()))
    }

    pub fn application(&self) -> Result<Application<'_, FileProjectStore<'_>>, ApplicationError> {
        let store = FileProjectStore::open_with_options(
            &self.root,
            self.repository.clone(),
            &self.config.contract_root,
            &self.config.decision_root,
            DocumentFormat::Auto,
            &self.schema_registry,
        )
        .map_err(|error| ApplicationError::new(error.to_string()))?;
        Application::with_store_and_signal_registry(
            store,
            &self.rule_source,
            &self.framework_lock,
            &self.schema_registry,
            self.signal_registry.clone(),
        )
    }

    /// CI mode requires all authoritative Records and configuration to be tracked.
    pub fn assert_tracked_inputs(&self, change_id: &str) -> Result<(), ProjectRuntimeError> {
        let store = FileProjectStore::open_with_options(
            &self.root,
            self.repository.clone(),
            &self.config.contract_root,
            &self.config.decision_root,
            DocumentFormat::Auto,
            &self.schema_registry,
        )
        .map_err(|error| runtime_error(error.to_string()))?;
        let mut paths = store
            .record_paths(change_id)
            .map_err(|error| runtime_error(error.to_string()))?;
        paths.push(self.root.join(".agentic/config.yaml"));
        paths.push(self.root.join(".agentic/framework.lock"));
        let trust_store = self.root.join(TRUST_STORE_PATH);
        if trust_store.exists() {
            paths.push(trust_store);
        }
        let release_sources = self.root.join(RELEASE_SOURCES_PATH);
        if release_sources.exists() {
            paths.push(release_sources);
        }
        let adapter = GitRepositoryAdapter::with_signal_registry(
            &self.root,
            &self.config.repository_observation,
            false,
            self.signal_registry.clone(),
        )
        .map_err(|error| runtime_error(error.to_string()))?;
        adapter
            .assert_tracked_paths(&paths)
            .map_err(|error| runtime_error(error.to_string()))
    }

    /// Repository-wide reports must be based only on tracked authoritative Records.
    pub fn assert_tracked_project_inputs(&self) -> Result<(), ProjectRuntimeError> {
        let store = FileProjectStore::open_with_options(
            &self.root,
            self.repository.clone(),
            &self.config.contract_root,
            &self.config.decision_root,
            DocumentFormat::Auto,
            &self.schema_registry,
        )
        .map_err(|error| runtime_error(error.to_string()))?;
        let mut paths = store
            .all_record_paths()
            .map_err(|error| runtime_error(error.to_string()))?;
        paths.push(self.root.join(".agentic/config.yaml"));
        paths.push(self.root.join(".agentic/framework.lock"));
        let trust_store = self.root.join(TRUST_STORE_PATH);
        if trust_store.exists() {
            paths.push(trust_store);
        }
        let release_sources = self.root.join(RELEASE_SOURCES_PATH);
        if release_sources.exists() {
            paths.push(release_sources);
        }
        let adapter = GitRepositoryAdapter::with_signal_registry(
            &self.root,
            &self.config.repository_observation,
            false,
            self.signal_registry.clone(),
        )
        .map_err(|error| runtime_error(error.to_string()))?;
        adapter
            .assert_tracked_paths(&paths)
            .map_err(|error| runtime_error(error.to_string()))
    }

    /// Resolve a user-selected project policy without allowing repository escape.
    pub fn repository_path(&self, relative: &str) -> Result<PathBuf, ProjectRuntimeError> {
        repository_path(&self.root, relative).map_err(|error| runtime_error(error.to_string()))
    }

    /// CI-only policies supplied outside the project config must also be tracked.
    pub fn assert_tracked_paths(&self, paths: &[PathBuf]) -> Result<(), ProjectRuntimeError> {
        let adapter = GitRepositoryAdapter::with_signal_registry(
            &self.root,
            &self.config.repository_observation,
            false,
            self.signal_registry.clone(),
        )
        .map_err(|error| runtime_error(error.to_string()))?;
        adapter
            .assert_tracked_paths(paths)
            .map_err(|error| runtime_error(error.to_string()))
    }
}

fn read_yaml(path: impl AsRef<Path>) -> Result<Value, ProjectRuntimeError> {
    let path = path.as_ref();
    let text = fs::read_to_string(path)
        .map_err(|error| runtime_error(format!("{}: {error}", path.display())))?;
    serde_yaml::from_str(&text)
        .map_err(|error| runtime_error(format!("{}: {error}", path.display())))
}

fn runtime_error(message: impl Into<String>) -> ProjectRuntimeError {
    ProjectRuntimeError {
        message: message.into(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectRuntimeError {
    message: String,
}

impl fmt::Display for ProjectRuntimeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ProjectRuntimeError {}
