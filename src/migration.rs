//! Read-only inspection of a current CLI project before migration.
//!
//! This module inventories mechanically observable state and prepares a
//! reviewable work plan. It never overwrites active Project files or decides
//! whether legacy semantics are valid; candidate generation writes only to a
//! new reserved directory.

use crate::binding_validation::BindingValidationReport;
use crate::delivery::resolve_verified_release_for_activation;
use crate::filesystem_project::{DocumentFormat, FileProjectStore};
use crate::git_repository::GitRepositoryAdapter;
use crate::project_config::load_project_config;
use crate::signal_catalog::SignalCatalogRegistry;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::{ErrorKind, Write};
use std::path::{Component, Path, PathBuf};
use std::process::Command;

pub const MIGRATION_INSPECTION_SCHEMA_VERSION: &str = "1";
pub const MIGRATION_DRAFT_SCHEMA_VERSION: &str = "2";
pub const MIGRATION_DRAFT_VALIDATION_SCHEMA_VERSION: &str = "1";
pub const MIGRATION_CANDIDATE_SCHEMA_VERSION: &str = "2";
pub const MIGRATION_ACTION_COMPLETION_SCHEMA_VERSION: &str = "1";
pub const MIGRATION_CANDIDATE_VALIDATION_SCHEMA_VERSION: &str = "2";
pub const MIGRATION_APPLICATION_SCHEMA_VERSION: &str = "1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MigrationInspectionReport {
    pub schema_version: String,
    pub source_state: String,
    pub readiness: String,
    pub repository: MigrationRepository,
    pub installation: LegacyInstallation,
    pub inventory: BTreeMap<String, MigrationInventory>,
    pub components: Vec<MigrationComponent>,
    pub findings: Vec<MigrationFinding>,
    pub next: String,
}

impl MigrationInspectionReport {
    pub fn render_text(&self) -> String {
        let mut output = format!(
            "Migration inspection: {}\nsource state: {}\nrevision: {}\nworking tree clean: {}\n",
            self.readiness,
            self.source_state,
            self.repository.revision,
            self.repository.working_tree_clean
        );
        if self.installation.present {
            output.push_str(&format!(
                "legacy installation: kit={} mode={} level={}\n",
                display_optional(&self.installation.kit_version),
                display_optional(&self.installation.mode),
                display_optional(&self.installation.level)
            ));
        }
        output.push_str("inventory:\n");
        for (name, inventory) in &self.inventory {
            output.push_str(&format!(
                "- {name}: files={} tracked={} roots={}\n",
                inventory.files,
                inventory.tracked_files,
                inventory.roots.join(",")
            ));
        }
        output.push_str("migration components:\n");
        for component in &self.components {
            output.push_str(&format!(
                "- {}: {} (items={}) — {}\n",
                component.id, component.classification, component.items, component.rationale
            ));
        }
        if !self.findings.is_empty() {
            output.push_str("findings:\n");
            for finding in &self.findings {
                output.push_str(&format!(
                    "- [{}] {} {}: {}\n",
                    finding.severity, finding.category, finding.path, finding.message
                ));
            }
        }
        output.push_str(&format!("Next: {}\n", self.next));
        output
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MigrationDraftReport {
    pub schema_version: String,
    pub kind: String,
    pub source_revision: String,
    pub source_state: String,
    pub status: String,
    pub actions: Vec<MigrationDraftAction>,
    pub next: String,
}

impl MigrationDraftReport {
    pub fn render_text(&self) -> String {
        let mut output = format!(
            "Migration draft: {}\nsource state: {}\nsource revision: {}\n",
            self.status, self.source_state, self.source_revision
        );
        output.push_str("proposed actions:\n");
        for action in &self.actions {
            output.push_str(&format!(
                "- {}: {} / {} (items={}, human review={})\n",
                action.id,
                action.classification,
                action.operation,
                action.items,
                action.requires_human_review
            ));
            output.push_str(&format!(
                "  source: {}\n",
                display_paths(&action.source_paths)
            ));
            output.push_str(&format!(
                "  target: {}\n",
                display_paths(&action.target_paths)
            ));
            output.push_str(&format!("  instruction: {}\n", action.instruction));
            for check in &action.completion_checks {
                output.push_str(&format!("  check: {check}\n"));
            }
        }
        output.push_str(&format!("Next: {}\n", self.next));
        output
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MigrationDraftAction {
    pub id: String,
    pub classification: String,
    pub operation: String,
    pub source_paths: Vec<String>,
    pub target_paths: Vec<String>,
    pub items: usize,
    pub requires_human_review: bool,
    pub instruction: String,
    pub completion_checks: Vec<String>,
    pub review: Option<MigrationActionReview>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MigrationActionReview {
    pub decision: String,
    pub reviewer: String,
    pub rationale: String,
    pub evidence_refs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MigrationDraftValidationReport {
    pub schema_version: String,
    pub status: String,
    pub source_revision: Option<String>,
    pub issues: Vec<MigrationDraftValidationIssue>,
    pub next: String,
}

impl MigrationDraftValidationReport {
    pub fn render_text(&self) -> String {
        let mut output = format!(
            "Migration draft validation: {}\nsource revision: {}\n",
            self.status,
            self.source_revision.as_deref().unwrap_or("-")
        );
        for issue in &self.issues {
            output.push_str(&format!(
                "- {} {}: {}\n",
                issue.category,
                issue.action_id.as_deref().unwrap_or("-"),
                issue.message
            ));
        }
        output.push_str(&format!("Next: {}\n", self.next));
        output
    }

    pub fn is_valid(&self) -> bool {
        self.status == "valid"
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct MigrationDraftValidationIssue {
    pub category: String,
    pub action_id: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MigrationCandidateManifest {
    pub schema_version: String,
    pub kind: String,
    pub status: String,
    pub source_revision: String,
    pub draft_digest: String,
    pub draft_source_path: Option<String>,
    pub output_root: String,
    pub generated_files: Vec<MigrationCandidateFile>,
    pub pending_actions: Vec<MigrationPendingAction>,
    pub next: String,
}

impl MigrationCandidateManifest {
    pub fn render_text(&self) -> String {
        let mut output = format!(
            "Migration candidate: {}\noutput: {}\nsource revision: {}\ndraft digest: {}\n",
            self.status, self.output_root, self.source_revision, self.draft_digest
        );
        output.push_str("generated files:\n");
        for file in &self.generated_files {
            output.push_str(&format!("- {} {}\n", file.path, file.digest));
        }
        output.push_str("pending actions:\n");
        for action in &self.pending_actions {
            output.push_str(&format!(
                "- {}: decision={} targets={} — {}\n",
                action.id,
                action.decision,
                display_paths(&action.target_paths),
                action.instruction
            ));
        }
        output.push_str(&format!("Next: {}\n", self.next));
        output
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MigrationCandidateFile {
    pub path: String,
    pub digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MigrationPendingAction {
    pub id: String,
    pub decision: String,
    pub target_paths: Vec<String>,
    pub instruction: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MigrationActionCompletion {
    pub schema_version: String,
    pub kind: String,
    pub action_id: String,
    pub source_revision: String,
    pub draft_digest: String,
    pub review: MigrationCompletionReview,
    pub completed_checks: Vec<String>,
    pub artifacts: Vec<MigrationCandidateFile>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MigrationCompletionReview {
    pub reviewer: String,
    pub rationale: String,
    pub evidence_refs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MigrationCandidateValidationReport {
    pub schema_version: String,
    pub status: String,
    pub candidate_root: String,
    pub source_revision: Option<String>,
    pub issues: Vec<MigrationCandidateValidationIssue>,
    pub pending_actions: Vec<String>,
    pub pending_validations: Vec<String>,
    pub next: String,
}

impl MigrationCandidateValidationReport {
    pub fn render_text(&self) -> String {
        let mut output = format!(
            "Migration candidate validation: {}\ncandidate: {}\nsource revision: {}\n",
            self.status,
            self.candidate_root,
            self.source_revision.as_deref().unwrap_or("-")
        );
        for issue in &self.issues {
            output.push_str(&format!(
                "- {} {}: {}\n",
                issue.category, issue.path, issue.message
            ));
        }
        if !self.pending_actions.is_empty() {
            output.push_str(&format!(
                "pending actions: {}\n",
                self.pending_actions.join(",")
            ));
        }
        if !self.pending_validations.is_empty() {
            output.push_str(&format!(
                "pending validations: {}\n",
                self.pending_validations.join(",")
            ));
        }
        output.push_str(&format!("Next: {}\n", self.next));
        output
    }

    pub fn is_valid(&self) -> bool {
        self.status == "valid"
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct MigrationCandidateValidationIssue {
    pub category: String,
    pub path: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MigrationApplicationRecord {
    pub schema_version: String,
    pub kind: String,
    pub status: String,
    pub application_id: String,
    pub candidate_root: String,
    pub source_revision: String,
    pub candidate_manifest_digest: String,
    pub application_root: String,
    pub applied_files: Vec<MigrationAppliedFile>,
    pub archived_paths: Vec<MigrationArchivedPath>,
    pub next: String,
}

impl MigrationApplicationRecord {
    pub fn render_text(&self) -> String {
        let mut output = format!(
            "Migration application: {}\napplication: {}\ncandidate: {}\nsource revision: {}\n",
            self.status, self.application_id, self.candidate_root, self.source_revision
        );
        output.push_str("applied files:\n");
        for file in &self.applied_files {
            output.push_str(&format!("- {} {}\n", file.path, file.digest));
        }
        output.push_str("archived paths:\n");
        for archived in &self.archived_paths {
            output.push_str(&format!("- {} -> {}\n", archived.source, archived.archive));
        }
        output.push_str(&format!("Next: {}\n", self.next));
        output
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MigrationAppliedFile {
    pub path: String,
    pub digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MigrationArchivedPath {
    pub source: String,
    pub archive: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MigrationRepository {
    pub revision: String,
    pub working_tree_clean: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LegacyInstallation {
    pub present: bool,
    pub schema_version: Option<String>,
    pub kit_version: Option<String>,
    pub mode: Option<String>,
    pub level: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MigrationInventory {
    pub roots: Vec<String>,
    pub files: usize,
    pub tracked_files: usize,
    pub formats: BTreeMap<String, usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MigrationComponent {
    pub id: String,
    pub classification: String,
    pub source_paths: Vec<String>,
    pub target_paths: Vec<String>,
    pub items: usize,
    pub rationale: String,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct MigrationFinding {
    pub severity: String,
    pub category: String,
    pub path: String,
    pub message: String,
}

pub fn inspect_migration(
    project_root: impl AsRef<Path>,
) -> Result<MigrationInspectionReport, MigrationError> {
    inspect_migration_ignoring(project_root.as_ref(), &[])
}

fn inspect_migration_ignoring(
    project_root: &Path,
    ignored_worktree_paths: &[&Path],
) -> Result<MigrationInspectionReport, MigrationError> {
    let root = project_root
        .canonicalize()
        .map_err(|error| migration_error(format!("cannot resolve project root: {error}")))?;
    if !root.is_dir() {
        return Err(migration_error("project root must be a directory"));
    }
    assert_git_top_level(&root)?;
    let revision = git(&root, &["rev-parse", "HEAD"])?;
    let mut status_arguments = vec![
        "status".to_owned(),
        "--porcelain=v1".to_owned(),
        "--untracked-files=all".to_owned(),
    ];
    let ignored_pathspecs = ignored_worktree_paths
        .iter()
        .filter_map(|path| path.canonicalize().ok())
        .filter_map(|path| path.strip_prefix(&root).ok().map(Path::to_path_buf))
        .map(|path| format!(":(exclude){}", path.to_string_lossy().replace('\\', "/")))
        .collect::<Vec<_>>();
    if !ignored_pathspecs.is_empty() {
        status_arguments.push("--".to_owned());
        status_arguments.push(".".to_owned());
        status_arguments.extend(ignored_pathspecs);
    }
    let status_argument_refs = status_arguments
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>();
    let status = git(&root, &status_argument_refs)?;
    let working_tree_clean = status.is_empty();
    let tracked = git(&root, &["ls-files"])?
        .lines()
        .map(str::to_owned)
        .collect::<BTreeSet<_>>();

    let mut findings = Vec::new();
    if !working_tree_clean {
        findings.push(finding(
            "blocking",
            "dirty-worktree",
            ".",
            "Migration preparation requires a clean Git worktree at one fixed revision.",
        ));
    }

    let installation_path = root.join(".agentic/installation.yaml");
    let installation_value = read_control_yaml(
        &root,
        &installation_path,
        ".agentic/installation.yaml",
        &mut findings,
    );
    let installation = legacy_installation(&installation_path, installation_value.as_ref());

    // The retired CLI kept its project under `.agentic`; this framework keeps
    // its own under `.adf`. A project part way through migration has both, and
    // telling them apart is what makes that state detectable rather than a
    // config that fails to parse as either format.
    let legacy_config_path = root.join(".agentic/config.yaml");
    let legacy_config = read_control_yaml(
        &root,
        &legacy_config_path,
        ".agentic/config.yaml",
        &mut findings,
    );
    let config_path = root.join(".adf/config.yaml");
    let config = read_control_yaml(&root, &config_path, ".adf/config.yaml", &mut findings);
    let current_config = legacy_config
        .as_ref()
        .and_then(Value::as_object)
        .is_some_and(|object| object.contains_key("contract_roots"));
    let framework_config = config
        .as_ref()
        .and_then(Value::as_object)
        .is_some_and(|object| object.contains_key("project_sources"));
    if current_config
        && legacy_config
            .as_ref()
            .and_then(|value| value["schema_version"].as_u64())
            != Some(1)
    {
        findings.push(finding(
            "blocking",
            "unsupported-current-config",
            ".agentic/config.yaml",
            "Current CLI config schema_version must be 1 before migration inspection.",
        ));
    }
    if framework_config
        && config
            .as_ref()
            .and_then(|value| value["schema_version"].as_str())
            != Some("1")
    {
        findings.push(finding(
            "blocking",
            "unsupported-framework-config",
            ".adf/config.yaml",
            "Framework config schema_version must be the string 1.",
        ));
    }
    if legacy_config_path.is_file() && !current_config {
        findings.push(finding(
            "blocking",
            "unknown-config-format",
            ".agentic/config.yaml",
            "The project config is not in the retired CLI format.",
        ));
    }
    if config_path.is_file() && !framework_config {
        findings.push(finding(
            "blocking",
            "unknown-config-format",
            ".adf/config.yaml",
            "The project config is not in this framework's format.",
        ));
    }

    let framework_markers = [
        ".adf/framework.lock",
        ".adf/repository-observation.yaml",
        ".adf/trusted-release-keys.yaml",
    ];
    let present_framework_markers = framework_markers
        .iter()
        .filter(|relative| regular_marker(&root, relative, &mut findings))
        .copied()
        .collect::<Vec<_>>();
    let source_state = if current_config && !present_framework_markers.is_empty() {
        findings.push(finding(
            "blocking",
            "mixed-runtime-state",
            ".",
            "The retired CLI config and this framework's activation files coexist; reconcile the active runtime before preparing a migration.",
        ));
        "mixed"
    } else if current_config {
        "current"
    } else if framework_config {
        "framework"
    } else if installation.present {
        findings.push(finding(
            "blocking",
            "missing-current-config",
            ".adf/config.yaml",
            "A current CLI installation exists but its config is missing or unreadable.",
        ));
        "current-incomplete"
    } else if !present_framework_markers.is_empty() {
        findings.push(finding(
            "blocking",
            "incomplete-framework-project",
            ".adf",
            "Activation files exist without a recognizable framework config.",
        ));
        "framework-incomplete"
    } else {
        findings.push(finding(
            "blocking",
            "uninitialized-project",
            ".",
            "Neither a retired CLI project nor a framework project configuration was found.",
        ));
        "uninitialized"
    }
    .to_owned();

    if current_config {
        if !installation.present {
            findings.push(finding(
                "blocking",
                "missing-installation-metadata",
                ".agentic/installation.yaml",
                "Current CLI installation metadata is required to identify the migration source.",
            ));
        } else if installation.schema_version.as_deref() != Some("2")
            || installation.kit_version.is_none()
            || installation.mode.is_none()
            || installation.level.is_none()
        {
            findings.push(finding(
                "blocking",
                "incomplete-installation-metadata",
                ".agentic/installation.yaml",
                "Current CLI installation metadata must include schema 2, kit version, mode, and level.",
            ));
        }
        let active_path = root.join(".agentic/active-changes.yaml");
        if !active_path.exists() {
            findings.push(finding(
                "blocking",
                "missing-active-change-index",
                ".agentic/active-changes.yaml",
                "Current CLI active change index is required for workflow migration review.",
            ));
        } else {
            read_control_yaml(
                &root,
                &active_path,
                ".agentic/active-changes.yaml",
                &mut findings,
            );
        }
    }

    if source_state == "framework" {
        for marker in framework_markers {
            if !present_framework_markers.contains(&marker) {
                findings.push(finding(
                    "blocking",
                    "missing-activation-file",
                    marker,
                    "The project is not fully activatable.",
                ));
            }
        }
    }

    // The retired CLI named its roots in its own config; this framework names
    // them in its own. Which file to read follows from which project this is.
    let configured_contract_roots = if current_config {
        configured_string_list(&legacy_config, "contract_roots")
    } else {
        configured_framework_string(&config, "contracts").map(|path| vec![path])
    };
    let configured_decision_roots = if current_config {
        configured_string(&legacy_config, "decision_root").map(|path| vec![path])
    } else {
        configured_framework_string(&config, "decisions").map(|path| vec![path])
    };
    let contract_roots = normalized_roots(
        configured_contract_roots,
        "contracts",
        "contract_roots",
        current_config || framework_config,
        &mut findings,
    );
    let decision_roots = normalized_roots(
        configured_decision_roots,
        "decisions",
        "decision_root",
        current_config || framework_config,
        &mut findings,
    );
    let evidence_roots = normalized_roots(
        configured_string(&legacy_config, "evidence_root").map(|path| vec![path]),
        "evidence",
        "evidence_root",
        current_config,
        &mut findings,
    );
    let change_roots = vec![".agentic/changes".to_owned()];

    let mut inventory = BTreeMap::new();
    inventory.insert(
        "changes".to_owned(),
        inventory_roots(&root, &change_roots, &tracked, false, &mut findings),
    );
    inventory.insert(
        "contracts".to_owned(),
        inventory_roots(
            &root,
            &contract_roots,
            &tracked,
            current_config,
            &mut findings,
        ),
    );
    inventory.insert(
        "decisions".to_owned(),
        inventory_roots(
            &root,
            &decision_roots,
            &tracked,
            current_config,
            &mut findings,
        ),
    );
    inventory.insert(
        "evidence".to_owned(),
        inventory_roots(&root, &evidence_roots, &tracked, false, &mut findings),
    );

    let components = migration_components(&inventory, current_config, framework_config);
    findings.sort();
    let has_blocker = findings
        .iter()
        .any(|finding| finding.severity == "blocking");
    let readiness = if has_blocker {
        "blocked"
    } else if source_state == "framework" {
        "already-migrated"
    } else {
        "review-required"
    }
    .to_owned();
    let next = match readiness.as_str() {
        "blocked" => {
            "Resolve every blocking finding, then run migration inspect again. No files were changed."
        }
        "already-migrated" => {
            "Run the validation commands; no legacy migration draft is required."
        }
        _ => {
            "Review semantic components before generating a migration draft. No files were changed."
        }
    }
    .to_owned();

    Ok(MigrationInspectionReport {
        schema_version: MIGRATION_INSPECTION_SCHEMA_VERSION.to_owned(),
        source_state,
        readiness,
        repository: MigrationRepository {
            revision: revision.trim().to_owned(),
            working_tree_clean,
        },
        installation,
        inventory,
        components,
        findings,
        next,
    })
}

pub fn draft_migration(
    project_root: impl AsRef<Path>,
) -> Result<MigrationDraftReport, MigrationError> {
    let inspection = inspect_migration(project_root)?;
    draft_from_inspection(inspection)
}

fn draft_from_inspection(
    inspection: MigrationInspectionReport,
) -> Result<MigrationDraftReport, MigrationError> {
    match inspection.readiness.as_str() {
        "blocked" => {
            return Err(migration_error(
                "migration draft is blocked; resolve the findings from `adf migration inspect` first",
            ));
        }
        "already-migrated" => {
            return Err(migration_error(
                "migration draft is not required because the project already uses the framework",
            ));
        }
        "review-required" => {}
        other => {
            return Err(migration_error(format!(
                "unsupported migration readiness: {other}"
            )));
        }
    }

    let actions = inspection
        .components
        .iter()
        .map(draft_action)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(MigrationDraftReport {
        schema_version: MIGRATION_DRAFT_SCHEMA_VERSION.to_owned(),
        kind: "migration-draft".to_owned(),
        source_revision: inspection.repository.revision,
        source_state: inspection.source_state,
        status: "review-required".to_owned(),
        actions,
        next: "Review and complete every action against this source revision. Generate candidate files only after the semantic mappings and signed Framework Release have been selected. No files were changed."
            .to_owned(),
    })
}

pub fn validate_migration_draft(
    project_root: impl AsRef<Path>,
    draft_path: impl AsRef<Path>,
) -> Result<MigrationDraftValidationReport, MigrationError> {
    let root = project_root
        .as_ref()
        .canonicalize()
        .map_err(|error| migration_error(format!("cannot resolve project root: {error}")))?;
    let draft_path = resolve_draft_path(&root, draft_path.as_ref())?;
    let text = fs::read_to_string(&draft_path)
        .map_err(|error| migration_error(format!("cannot read migration draft: {error}")))?;
    let value: Value = match serde_yaml::from_str(&text) {
        Ok(value) => value,
        Err(error) => {
            return Ok(validation_report(
                "invalid",
                None,
                vec![validation_issue(
                    "invalid-draft",
                    None,
                    &format!("Migration Draft is not valid JSON or YAML: {error}"),
                )],
            ));
        }
    };
    let actual: MigrationDraftReport = match serde_json::from_value(value) {
        Ok(draft) => draft,
        Err(error) => {
            return Ok(validation_report(
                "invalid",
                None,
                vec![validation_issue(
                    "invalid-draft",
                    None,
                    &format!("Migration Draft does not match schema version 2: {error}"),
                )],
            ));
        }
    };
    if actual.source_revision.len() != 40
        || !actual
            .source_revision
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Ok(validation_report(
            "invalid",
            None,
            vec![validation_issue(
                "invalid-source-revision",
                None,
                "source_revision must be a 40-character lowercase Git object ID.",
            )],
        ));
    }

    let inspection = inspect_migration_ignoring(&root, &[&draft_path])?;
    if inspection.readiness != "review-required" {
        let message = match inspection.readiness.as_str() {
            "already-migrated" => {
                "The Project already uses the framework; migration is no longer required."
            }
            _ => {
                "The current Project state is blocked; resolve `migration inspect` findings before validating the Draft."
            }
        };
        return Ok(validation_report(
            "blocked",
            Some(actual.source_revision),
            vec![validation_issue("project-state", None, message)],
        ));
    }
    let expected = draft_from_inspection(inspection)?;
    let mut issues = Vec::new();
    if actual.source_revision != expected.source_revision {
        issues.push(validation_issue(
            "stale-revision",
            None,
            "Migration Draft source_revision does not match the current Git HEAD.",
        ));
    }

    let mut immutable_actual = actual.clone();
    immutable_actual.source_revision = expected.source_revision.clone();
    for action in &mut immutable_actual.actions {
        action.review = None;
    }
    if immutable_actual != expected {
        issues.push(validation_issue(
            "modified-draft",
            None,
            "Generated Migration Draft fields were changed; regenerate the Draft and edit only action.review values.",
        ));
    }

    for action in &actual.actions {
        validate_action_review(action, &mut issues);
    }
    issues.sort();
    let status = if issues.is_empty() {
        "valid"
    } else {
        "invalid"
    };
    Ok(validation_report(
        status,
        Some(actual.source_revision),
        issues,
    ))
}

pub fn generate_migration_candidate(
    project_root: impl AsRef<Path>,
    draft_path: impl AsRef<Path>,
    output_path: impl AsRef<Path>,
) -> Result<MigrationCandidateManifest, MigrationError> {
    let root = project_root
        .as_ref()
        .canonicalize()
        .map_err(|error| migration_error(format!("cannot resolve project root: {error}")))?;
    let draft_path = resolve_draft_path(&root, draft_path.as_ref())?;
    let (output_root, output_relative) = resolve_candidate_output(&root, output_path.as_ref())?;
    let validation = validate_migration_draft(&root, &draft_path)?;
    if !validation.is_valid() {
        return Err(migration_error(format!(
            "migration candidate requires a valid reviewed Draft; validation status was {}",
            validation.status
        )));
    }
    let draft_text = fs::read_to_string(&draft_path)
        .map_err(|error| migration_error(format!("cannot read migration draft: {error}")))?;
    let draft_value: Value = serde_yaml::from_str(&draft_text)
        .map_err(|error| migration_error(format!("cannot parse migration draft: {error}")))?;
    let draft: MigrationDraftReport = serde_json::from_value(draft_value)
        .map_err(|error| migration_error(format!("cannot load migration draft: {error}")))?;
    let canonical_value = serde_json::to_value(&draft)
        .map_err(|error| migration_error(format!("cannot serialize migration draft: {error}")))?;
    let draft_digest = crate::canonical_digest(&canonical_value)
        .map_err(|error| migration_error(format!("cannot digest migration draft: {error}")))?;
    let draft_source_path = draft_path
        .strip_prefix(&root)
        .ok()
        .map(|path| path.to_string_lossy().replace('\\', "/"));

    let config_bytes = candidate_config_bytes();
    let mut draft_bytes = serde_json::to_vec_pretty(&draft)
        .map_err(|error| migration_error(format!("cannot serialize migration draft: {error}")))?;
    draft_bytes.push(b'\n');
    let manifest = build_candidate_manifest(
        &draft,
        draft_digest,
        draft_source_path,
        output_relative,
        &config_bytes,
        &draft_bytes,
    );
    write_candidate_bundle(&output_root, &config_bytes, &draft_bytes, &manifest)?;
    Ok(manifest)
}

fn build_candidate_manifest(
    draft: &MigrationDraftReport,
    draft_digest: String,
    draft_source_path: Option<String>,
    output_relative: String,
    config_bytes: &[u8],
    draft_bytes: &[u8],
) -> MigrationCandidateManifest {
    let generated_files = vec![
        MigrationCandidateFile {
            path: ".adf/config.yaml".to_owned(),
            digest: byte_digest(config_bytes),
        },
        MigrationCandidateFile {
            path: "migration-draft.json".to_owned(),
            digest: byte_digest(draft_bytes),
        },
    ];
    let pending_actions = draft
        .actions
        .iter()
        .filter_map(|action| {
            let review = action.review.as_ref()?;
            match review.decision.as_str() {
                "proceed" => Some(MigrationPendingAction {
                    id: action.id.clone(),
                    decision: review.decision.clone(),
                    target_paths: action.target_paths.clone(),
                    instruction: action.instruction.clone(),
                }),
                "preserve-history" => Some(MigrationPendingAction {
                    id: action.id.clone(),
                    decision: review.decision.clone(),
                    target_paths: Vec::new(),
                    instruction: "Preserve the reviewed legacy material as non-active history without creating authoritative Records."
                        .to_owned(),
                }),
                _ => None,
            }
        })
        .collect::<Vec<_>>();
    MigrationCandidateManifest {
        schema_version: MIGRATION_CANDIDATE_SCHEMA_VERSION.to_owned(),
        kind: "migration-candidate".to_owned(),
        status: "incomplete".to_owned(),
        source_revision: draft.source_revision.clone(),
        draft_digest,
        draft_source_path,
        output_root: output_relative,
        generated_files,
        pending_actions,
        next: "Complete every pending action inside this isolated candidate, then validate the candidate before any explicit application. Existing Project files were not changed."
            .to_owned(),
    }
}

fn candidate_config_bytes() -> Vec<u8> {
    concat!(
        "schema_version: \"1\"\n",
        "project_sources:\n",
        "  contracts: contracts\n",
        "  decisions: decisions\n",
        "repository_observation: .adf/repository-observation.yaml\n"
    )
    .as_bytes()
    .to_vec()
}

pub fn validate_migration_candidate(
    project_root: impl AsRef<Path>,
    candidate_path: impl AsRef<Path>,
) -> Result<MigrationCandidateValidationReport, MigrationError> {
    let root = project_root
        .as_ref()
        .canonicalize()
        .map_err(|error| migration_error(format!("cannot resolve project root: {error}")))?;
    let (candidate_root, candidate_relative) =
        resolve_existing_candidate(&root, candidate_path.as_ref())?;
    let manifest_path = candidate_root.join("migration-manifest.yaml");
    let manifest_text = match read_regular_utf8(&manifest_path) {
        Ok(text) => text,
        Err(error) => {
            return Ok(candidate_validation_report(
                "invalid",
                candidate_relative,
                None,
                vec![candidate_validation_issue(
                    "invalid-manifest",
                    "migration-manifest.yaml",
                    &error.to_string(),
                )],
                Vec::new(),
            ));
        }
    };
    let manifest_value: Value = match serde_yaml::from_str(&manifest_text) {
        Ok(value) => value,
        Err(error) => {
            return Ok(candidate_validation_report(
                "invalid",
                candidate_relative,
                None,
                vec![candidate_validation_issue(
                    "invalid-manifest",
                    "migration-manifest.yaml",
                    &format!("Migration Candidate Manifest is not valid YAML: {error}"),
                )],
                Vec::new(),
            ));
        }
    };
    let manifest: MigrationCandidateManifest = match serde_json::from_value(manifest_value) {
        Ok(manifest) => manifest,
        Err(error) => {
            return Ok(candidate_validation_report(
                "invalid",
                candidate_relative,
                None,
                vec![candidate_validation_issue(
                    "invalid-manifest",
                    "migration-manifest.yaml",
                    &format!("Migration Candidate Manifest does not match version 2: {error}"),
                )],
                Vec::new(),
            ));
        }
    };

    let mut issues = Vec::new();
    if !is_lower_hex(&manifest.source_revision, 40) {
        return Ok(candidate_validation_report(
            "invalid",
            candidate_relative,
            None,
            vec![candidate_validation_issue(
                "invalid-source-revision",
                "migration-manifest.yaml",
                "source_revision must be a 40-character lowercase Git object ID.",
            )],
            Vec::new(),
        ));
    }
    if manifest.schema_version != MIGRATION_CANDIDATE_SCHEMA_VERSION
        || manifest.kind != "migration-candidate"
        || manifest.status != "incomplete"
    {
        issues.push(candidate_validation_issue(
            "invalid-manifest-identity",
            "migration-manifest.yaml",
            "Manifest version, kind, or generated status is not supported.",
        ));
    }
    if !manifest.draft_digest.starts_with("sha256:")
        || !is_lower_hex(&manifest.draft_digest["sha256:".len()..], 64)
    {
        issues.push(candidate_validation_issue(
            "invalid-draft-digest",
            "migration-manifest.yaml",
            "draft_digest must be a lowercase SHA-256 digest.",
        ));
    }
    let mut ignored_paths = vec![candidate_root.clone()];
    if let Some(relative) = &manifest.draft_source_path {
        match safe_repository_path(&root, relative) {
            Ok(source_path) if source_path.is_file() => match canonical_draft_digest(&source_path) {
                Ok(digest) if digest == manifest.draft_digest => ignored_paths.push(source_path),
                Ok(_) => issues.push(candidate_validation_issue(
                    "draft-source-mismatch",
                    relative,
                    "draft_source_path does not contain the reviewed Draft fixed by draft_digest.",
                )),
                Err(error) => issues.push(candidate_validation_issue(
                    "invalid-draft-source",
                    relative,
                    &error.to_string(),
                )),
            },
            Ok(_) => {}
            Err(error) => issues.push(candidate_validation_issue(
                "unsafe-draft-source",
                relative,
                &error.to_string(),
            )),
        }
    }
    let ignored_refs = ignored_paths
        .iter()
        .map(PathBuf::as_path)
        .collect::<Vec<_>>();
    let inspection = inspect_migration_ignoring(&root, &ignored_refs)?;
    if inspection.readiness != "review-required" {
        issues.push(candidate_validation_issue(
            "project-state",
            ".",
            "The current Project state is blocked; resolve migration inspect findings before validating the Candidate.",
        ));
        issues.sort();
        return Ok(candidate_validation_report(
            "blocked",
            candidate_relative,
            Some(manifest.source_revision),
            issues,
            manifest
                .pending_actions
                .iter()
                .map(|action| action.id.clone())
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect(),
        ));
    }
    if manifest.source_revision != inspection.repository.revision {
        issues.push(candidate_validation_issue(
            "stale-revision",
            "migration-manifest.yaml",
            "Candidate source_revision does not match the current Git HEAD.",
        ));
    }
    if manifest.output_root != candidate_relative {
        issues.push(candidate_validation_issue(
            "candidate-location-mismatch",
            "migration-manifest.yaml",
            "Manifest output_root does not match the Candidate directory.",
        ));
    }

    let draft_path = candidate_root.join("migration-draft.json");
    let draft_bytes = match read_regular_bytes(&draft_path) {
        Ok(bytes) => bytes,
        Err(error) => {
            issues.push(candidate_validation_issue(
                "invalid-generated-file",
                "migration-draft.json",
                &error.to_string(),
            ));
            Vec::new()
        }
    };
    let draft = if draft_bytes.is_empty() {
        None
    } else {
        parse_candidate_draft(&draft_bytes, &mut issues)
    };
    if let Some(draft) = &draft {
        let canonical_value = serde_json::to_value(draft).map_err(|error| {
            migration_error(format!("cannot serialize migration draft: {error}"))
        })?;
        let digest = crate::canonical_digest(&canonical_value)
            .map_err(|error| migration_error(format!("cannot digest migration draft: {error}")))?;
        if digest != manifest.draft_digest {
            issues.push(candidate_validation_issue(
                "draft-digest-mismatch",
                "migration-draft.json",
                "Embedded reviewed Draft does not match Manifest draft_digest.",
            ));
        }
        let expected_draft = draft_from_inspection(inspection)?;
        let mut immutable_draft = draft.clone();
        for action in &mut immutable_draft.actions {
            action.review = None;
        }
        if immutable_draft != expected_draft {
            issues.push(candidate_validation_issue(
                "modified-draft",
                "migration-draft.json",
                "Embedded Draft generated fields do not match the current source revision.",
            ));
        }
        for action in &draft.actions {
            let mut draft_issues = Vec::new();
            validate_action_review(action, &mut draft_issues);
            for issue in draft_issues {
                issues.push(candidate_validation_issue(
                    &issue.category,
                    "migration-draft.json",
                    &issue.message,
                ));
            }
        }

        let expected_manifest = build_candidate_manifest(
            draft,
            manifest.draft_digest.clone(),
            manifest.draft_source_path.clone(),
            candidate_relative.clone(),
            &candidate_config_bytes(),
            &draft_bytes,
        );
        if manifest != expected_manifest {
            issues.push(candidate_validation_issue(
                "modified-manifest",
                "migration-manifest.yaml",
                "Generated Manifest fields do not match the reviewed Draft and Candidate files.",
            ));
        }
    }

    verify_candidate_generated_files(&candidate_root, &manifest, &mut issues);
    let mut claimed_artifacts = BTreeMap::new();
    let pending_actions = match draft.as_ref() {
        Some(draft) => validate_action_completions(
            &candidate_root,
            &manifest,
            draft,
            &mut claimed_artifacts,
            &mut issues,
        ),
        None => manifest
            .pending_actions
            .iter()
            .map(|action| action.id.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>(),
    };
    verify_unclaimed_candidate_files(&candidate_root, &claimed_artifacts, &mut issues);
    if issues.is_empty() && pending_actions.is_empty() {
        validate_candidate_activation(
            &root,
            &candidate_root,
            &candidate_relative,
            &manifest,
            &claimed_artifacts,
            &mut issues,
        );
    }
    issues.sort();
    let status = if !issues.is_empty() {
        "invalid"
    } else if !pending_actions.is_empty() {
        "incomplete"
    } else {
        "valid"
    };
    Ok(candidate_validation_report(
        status,
        candidate_relative,
        Some(manifest.source_revision),
        issues,
        pending_actions,
    ))
}

pub fn apply_migration_candidate(
    project_root: impl AsRef<Path>,
    candidate_path: impl AsRef<Path>,
) -> Result<MigrationApplicationRecord, MigrationError> {
    let root = project_root
        .as_ref()
        .canonicalize()
        .map_err(|error| migration_error(format!("cannot resolve project root: {error}")))?;
    let (candidate_root, candidate_relative) =
        resolve_existing_candidate(&root, candidate_path.as_ref())?;
    let _lock = MigrationApplyLock::acquire(&candidate_root)?;
    let validation = validate_migration_candidate(&root, Path::new(&candidate_relative))?;
    if !validation.is_valid() {
        return Err(migration_error(format!(
            "migration candidate must be valid before application; validation status was {}",
            validation.status
        )));
    }

    let manifest = load_candidate_manifest(&candidate_root)?;
    let draft = load_candidate_draft(&candidate_root)?;
    let manifest_value = serde_json::to_value(&manifest).map_err(|error| {
        migration_error(format!("cannot serialize Candidate Manifest: {error}"))
    })?;
    let candidate_manifest_digest = crate::canonical_digest(&manifest_value)
        .map_err(|error| migration_error(format!("cannot digest Candidate Manifest: {error}")))?;
    let application_id = format!(
        "migration-{}-{}",
        &manifest.source_revision[..12],
        &candidate_manifest_digest["sha256:".len()..][..12]
    );
    let application_relative = format!(".adf/migrations/{application_id}");
    let application_root = safe_repository_path(&root, &application_relative)?;
    let archive_base_relative = format!(".adf/migration-history/{application_id}");
    let archive_base = safe_repository_path(&root, &archive_base_relative)?;
    if application_root.exists() || archive_base.exists() {
        return Err(migration_error(format!(
            "migration application {application_id} already exists or requires recovery"
        )));
    }

    let archive_sources = application_archive_sources(&root, &candidate_relative, &draft)?;
    let payloads = application_payloads(&candidate_root, &manifest)?;
    preflight_application_targets(&root, &archive_sources, &payloads)?;
    let archived_paths = archive_sources
        .iter()
        .map(|source| MigrationArchivedPath {
            source: source.clone(),
            archive: format!("{archive_base_relative}/source/{source}"),
        })
        .collect::<Vec<_>>();
    let applied_files = payloads
        .iter()
        .map(|payload| MigrationAppliedFile {
            path: payload.relative.clone(),
            digest: byte_digest(&payload.bytes),
        })
        .collect::<Vec<_>>();
    let record = MigrationApplicationRecord {
        schema_version: MIGRATION_APPLICATION_SCHEMA_VERSION.to_owned(),
        kind: "migration-application".to_owned(),
        status: "applied".to_owned(),
        application_id: application_id.clone(),
        candidate_root: candidate_relative,
        source_revision: manifest.source_revision,
        candidate_manifest_digest,
        application_root: application_relative.clone(),
        applied_files,
        archived_paths,
        next: "Review the archived sources and applied files, git add the reviewed migration, run normal validation, then commit it. Git index and commits were not changed."
            .to_owned(),
    };

    let mut created_files = Vec::new();
    let mut moved_sources = Vec::new();
    let result = (|| {
        write_application_provenance(&candidate_root, &application_root)?;
        for source in &archive_sources {
            let source_path = safe_repository_path(&root, source)?;
            if !source_path.exists() {
                continue;
            }
            let archive_relative = format!("{archive_base_relative}/source/{source}");
            let archive_path = safe_repository_path(&root, &archive_relative)?;
            if let Some(parent) = archive_path.parent() {
                fs::create_dir_all(parent).map_err(|error| {
                    migration_error(format!("cannot create migration archive parent: {error}"))
                })?;
            }
            fs::rename(&source_path, &archive_path).map_err(|error| {
                migration_error(format!("cannot archive migration source {source}: {error}"))
            })?;
            moved_sources.push((source_path, archive_path));
        }
        for payload in &payloads {
            let target = safe_repository_path(&root, &payload.relative)?;
            if target.is_file()
                && fs::read(&target).map_err(|error| {
                    migration_error(format!("cannot read existing migration target: {error}"))
                })? == payload.bytes
            {
                continue;
            }
            if let Err(error) = write_candidate_file(&target, &payload.bytes) {
                if target.exists() {
                    created_files.push(target);
                }
                return Err(error);
            }
            created_files.push(target);
        }
        let record_bytes = serde_yaml::to_string(&record)
            .map_err(|error| migration_error(format!("cannot serialize application: {error}")))?;
        write_candidate_file(
            &application_root.join("application.yaml"),
            record_bytes.as_bytes(),
        )?;
        Ok(())
    })();
    if let Err(error) = result {
        let rollback = rollback_failed_application(
            &root,
            &application_root,
            &archive_base,
            &created_files,
            &moved_sources,
        );
        return match rollback {
            Ok(()) => Err(error),
            Err(rollback_error) => Err(migration_error(format!(
                "{error}; automatic rollback also failed: {rollback_error}"
            ))),
        };
    }
    Ok(record)
}

struct MigrationApplicationPayload {
    relative: String,
    bytes: Vec<u8>,
}

struct MigrationApplyLock {
    path: PathBuf,
}

impl MigrationApplyLock {
    fn acquire(candidate_root: &Path) -> Result<Self, MigrationError> {
        let path = candidate_root.join("migration-apply.lock");
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|error| {
                migration_error(format!(
                    "another migration application is running or requires recovery: {error}"
                ))
            })?;
        writeln!(file, "pid={}", std::process::id())
            .and_then(|()| file.sync_all())
            .map_err(|error| {
                let _ = fs::remove_file(&path);
                migration_error(format!("cannot persist migration lock: {error}"))
            })?;
        Ok(Self { path })
    }
}

impl Drop for MigrationApplyLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn load_candidate_manifest(
    candidate_root: &Path,
) -> Result<MigrationCandidateManifest, MigrationError> {
    let value = read_yaml_value(&candidate_root.join("migration-manifest.yaml"))?;
    serde_json::from_value(value)
        .map_err(|error| migration_error(format!("cannot load Candidate Manifest: {error}")))
}

fn load_candidate_draft(candidate_root: &Path) -> Result<MigrationDraftReport, MigrationError> {
    let bytes = read_regular_bytes(&candidate_root.join("migration-draft.json"))?;
    serde_json::from_slice(&bytes)
        .map_err(|error| migration_error(format!("cannot load embedded Migration Draft: {error}")))
}

fn application_archive_sources(
    root: &Path,
    candidate_relative: &str,
    draft: &MigrationDraftReport,
) -> Result<Vec<String>, MigrationError> {
    let archived_action_ids = ["contracts", "decisions", "change-workflow", "evidence"]
        .into_iter()
        .collect::<BTreeSet<_>>();
    // What gets archived is what the retired CLI leaves behind, not the files
    // this framework is about to write.
    let mut candidates = BTreeSet::from([
        ".agentic/config.yaml".to_owned(),
        ".agentic/installation.yaml".to_owned(),
        ".agentic/active-changes.yaml".to_owned(),
    ]);
    for action in &draft.actions {
        if archived_action_ids.contains(action.id.as_str()) {
            candidates.extend(action.source_paths.iter().cloned());
        }
    }
    let mut candidates = candidates.into_iter().collect::<Vec<_>>();
    candidates.sort_by(|left, right| {
        (left.split('/').count(), left.as_str()).cmp(&(right.split('/').count(), right.as_str()))
    });
    let mut sources: Vec<String> = Vec::new();
    for relative in candidates {
        if relative == "."
            || relative == ".agentic"
            || relative == ".adf"
            || relative == ".git"
            || relative.starts_with(".git/")
            || path_is_within(candidate_relative, &relative)
        {
            return Err(migration_error(format!(
                "refusing to archive unsafe migration source: {relative}"
            )));
        }
        let path = safe_repository_path(root, &relative)?;
        if !path.exists() {
            continue;
        }
        if sources
            .iter()
            .any(|parent| path_is_within(&relative, parent))
        {
            continue;
        }
        sources.push(relative);
    }
    Ok(sources)
}

fn application_payloads(
    candidate_root: &Path,
    manifest: &MigrationCandidateManifest,
) -> Result<Vec<MigrationApplicationPayload>, MigrationError> {
    let mut paths = BTreeSet::from([".adf/config.yaml".to_owned()]);
    for action in &manifest.pending_actions {
        let completion = load_action_completion(candidate_root, &action.id)?;
        paths.extend(
            completion
                .artifacts
                .into_iter()
                .map(|artifact| artifact.path),
        );
    }
    let release_root = candidate_root.join(".adf/cache/releases");
    if release_root.is_dir() {
        collect_regular_relative_paths(candidate_root, &release_root, &mut paths)?;
    }
    paths
        .into_iter()
        .map(|relative| {
            let path = safe_repository_path(candidate_root, &relative)?;
            let bytes = read_regular_bytes(&path)?;
            Ok(MigrationApplicationPayload { relative, bytes })
        })
        .collect()
}

fn load_action_completion(
    candidate_root: &Path,
    action_id: &str,
) -> Result<MigrationActionCompletion, MigrationError> {
    let path = candidate_root
        .join("migration-completions")
        .join(format!("{action_id}.yaml"));
    let value = read_yaml_value(&path)?;
    serde_json::from_value(value)
        .map_err(|error| migration_error(format!("cannot load Completion Record: {error}")))
}

fn collect_regular_relative_paths(
    root: &Path,
    directory: &Path,
    output: &mut BTreeSet<String>,
) -> Result<(), MigrationError> {
    let mut entries = fs::read_dir(directory)
        .map_err(|error| migration_error(format!("cannot read Candidate directory: {error}")))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| migration_error(format!("cannot read Candidate entry: {error}")))?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|error| migration_error(format!("cannot inspect Candidate entry: {error}")))?;
        if file_type.is_symlink() {
            return Err(migration_error(
                "Candidate payload must not contain symlinks",
            ));
        }
        if file_type.is_dir() {
            collect_regular_relative_paths(root, &path, output)?;
        } else if file_type.is_file() {
            output.insert(relative_display(root, &path));
        } else {
            return Err(migration_error(
                "Candidate payload must contain only regular files and directories",
            ));
        }
    }
    Ok(())
}

fn preflight_application_targets(
    root: &Path,
    archive_sources: &[String],
    payloads: &[MigrationApplicationPayload],
) -> Result<(), MigrationError> {
    for payload in payloads {
        let target = safe_repository_path(root, &payload.relative)?;
        if archive_sources
            .iter()
            .any(|source| path_is_within(&payload.relative, source))
        {
            continue;
        }
        match fs::symlink_metadata(&target) {
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Ok(metadata)
                if metadata.is_file()
                    && fs::read(&target).ok().as_deref() == Some(&payload.bytes) => {}
            Ok(_) => {
                return Err(migration_error(format!(
                    "refusing to overwrite existing migration target: {}",
                    payload.relative
                )));
            }
            Err(error) => {
                return Err(migration_error(format!(
                    "cannot inspect migration target {}: {error}",
                    payload.relative
                )));
            }
        }
    }
    Ok(())
}

fn write_application_provenance(
    candidate_root: &Path,
    application_root: &Path,
) -> Result<(), MigrationError> {
    let parent = application_root
        .parent()
        .ok_or_else(|| migration_error("migration application has no parent"))?;
    fs::create_dir_all(parent).map_err(|error| {
        migration_error(format!("cannot create application record parent: {error}"))
    })?;
    fs::create_dir(application_root)
        .map_err(|error| migration_error(format!("cannot create application record: {error}")))?;
    for name in ["migration-manifest.yaml", "migration-draft.json"] {
        let bytes = read_regular_bytes(&candidate_root.join(name))?;
        write_candidate_file(&application_root.join(name), &bytes)?;
    }
    let source = candidate_root.join("migration-completions");
    let target = application_root.join("migration-completions");
    fs::create_dir(&target).map_err(|error| {
        migration_error(format!(
            "cannot create application completion records: {error}"
        ))
    })?;
    let mut entries = fs::read_dir(&source)
        .map_err(|error| migration_error(format!("cannot read Completion Records: {error}")))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| migration_error(format!("cannot read Completion Record: {error}")))?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let bytes = read_regular_bytes(&entry.path())?;
        write_candidate_file(&target.join(entry.file_name()), &bytes)?;
    }
    Ok(())
}

fn rollback_failed_application(
    root: &Path,
    application_root: &Path,
    archive_base: &Path,
    created_files: &[PathBuf],
    moved_sources: &[(PathBuf, PathBuf)],
) -> Result<(), MigrationError> {
    let mut failures = Vec::new();
    for path in created_files.iter().rev() {
        if path.exists()
            && let Err(error) = fs::remove_file(path)
        {
            failures.push(format!(
                "cannot remove partially applied file {}: {error}",
                path.display()
            ));
            continue;
        }
        remove_empty_parents(path.parent(), root);
    }
    let mut restore_failed = false;
    for (source, archive) in moved_sources.iter().rev() {
        if source.is_dir() {
            if let Err(error) = fs::remove_dir(source) {
                failures.push(format!(
                    "cannot clear partial migration directory {}: {error}",
                    source.display()
                ));
                restore_failed = true;
                continue;
            }
        } else if source.exists() {
            failures.push(format!(
                "cannot restore archived source because target exists: {}",
                source.display()
            ));
            restore_failed = true;
            continue;
        }
        if let Some(parent) = source.parent()
            && let Err(error) = fs::create_dir_all(parent)
        {
            failures.push(format!(
                "cannot recreate archived source parent {}: {error}",
                parent.display()
            ));
            restore_failed = true;
            continue;
        }
        if let Err(error) = fs::rename(archive, source) {
            failures.push(format!(
                "cannot restore archived source {}: {error}",
                source.display()
            ));
            restore_failed = true;
        }
    }
    if application_root.exists()
        && let Err(error) = fs::remove_dir_all(application_root)
    {
        failures.push(format!(
            "cannot remove partial application record {}: {error}",
            application_root.display()
        ));
    }
    if !restore_failed
        && archive_base.exists()
        && let Err(error) = fs::remove_dir_all(archive_base)
    {
        failures.push(format!(
            "cannot remove empty migration archive {}: {error}",
            archive_base.display()
        ));
    }
    if !failures.is_empty() {
        return Err(migration_error(failures.join("; ")));
    }
    Ok(())
}

fn remove_empty_parents(mut directory: Option<&Path>, root: &Path) {
    while let Some(path) = directory {
        if path == root || !path.starts_with(root) || fs::remove_dir(path).is_err() {
            break;
        }
        directory = path.parent();
    }
}

fn path_is_within(path: &str, parent: &str) -> bool {
    path == parent || path.starts_with(&format!("{parent}/"))
}

fn validate_action_completions(
    candidate_root: &Path,
    manifest: &MigrationCandidateManifest,
    draft: &MigrationDraftReport,
    claimed_artifacts: &mut BTreeMap<String, String>,
    issues: &mut Vec<MigrationCandidateValidationIssue>,
) -> Vec<String> {
    let completion_root = candidate_root.join("migration-completions");
    let expected_files = manifest
        .pending_actions
        .iter()
        .filter(|action| is_action_id(&action.id))
        .map(|action| format!("{}.yaml", action.id))
        .collect::<BTreeSet<_>>();
    inspect_completion_directory(&completion_root, &expected_files, issues);

    let draft_actions = draft
        .actions
        .iter()
        .map(|action| (action.id.as_str(), action))
        .collect::<BTreeMap<_, _>>();
    let mut pending = BTreeSet::new();
    for action in &manifest.pending_actions {
        let issue_count = issues.len();
        if !is_action_id(&action.id) {
            issues.push(candidate_validation_issue(
                "invalid-completion-action",
                "migration-manifest.yaml",
                "Pending action id cannot be used as a Completion Record filename.",
            ));
            pending.insert(action.id.clone());
            continue;
        }
        let relative = format!("migration-completions/{}.yaml", action.id);
        let path = completion_root.join(format!("{}.yaml", action.id));
        if !path.exists() {
            pending.insert(action.id.clone());
            continue;
        }
        let completion = match parse_action_completion(&path, &relative, issues) {
            Some(completion) => completion,
            None => {
                pending.insert(action.id.clone());
                continue;
            }
        };
        let Some(draft_action) = draft_actions.get(action.id.as_str()) else {
            issues.push(candidate_validation_issue(
                "unknown-completion-action",
                &relative,
                "Completion Record action_id is not present in the embedded Draft.",
            ));
            pending.insert(action.id.clone());
            continue;
        };
        validate_completion_identity(
            &completion,
            action,
            draft_action,
            manifest,
            &relative,
            issues,
        );
        validate_completion_artifacts(
            candidate_root,
            &completion,
            action,
            &relative,
            claimed_artifacts,
            issues,
        );
        if issues.len() != issue_count {
            pending.insert(action.id.clone());
        }
    }
    pending.into_iter().collect()
}

fn verify_unclaimed_candidate_files(
    candidate_root: &Path,
    claimed_artifacts: &BTreeMap<String, String>,
    issues: &mut Vec<MigrationCandidateValidationIssue>,
) {
    let mut files = Vec::new();
    collect_candidate_files(candidate_root, candidate_root, &mut files, issues);
    for file in files {
        if is_reserved_candidate_artifact(&file)
            || file.starts_with("migration-completions/")
            || file.starts_with(".adf/cache/releases/")
        {
            continue;
        }
        if !claimed_artifacts.contains_key(&file) {
            issues.push(candidate_validation_issue(
                "unclaimed-candidate-file",
                &file,
                "Candidate files must be bound to exactly one Completion Record.",
            ));
        }
    }
}

fn collect_candidate_files(
    candidate_root: &Path,
    directory: &Path,
    files: &mut Vec<String>,
    issues: &mut Vec<MigrationCandidateValidationIssue>,
) {
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) => {
            let relative = directory
                .strip_prefix(candidate_root)
                .unwrap_or(directory)
                .to_string_lossy()
                .replace('\\', "/");
            issues.push(candidate_validation_issue(
                "invalid-candidate-entry",
                if relative.is_empty() { "." } else { &relative },
                &format!("Cannot read Candidate directory: {error}"),
            ));
            return;
        }
    };
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                issues.push(candidate_validation_issue(
                    "invalid-candidate-entry",
                    ".",
                    &format!("Cannot inspect Candidate entry: {error}"),
                ));
                continue;
            }
        };
        let path = entry.path();
        let relative = path
            .strip_prefix(candidate_root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        let file_type = match entry.file_type() {
            Ok(file_type) => file_type,
            Err(error) => {
                issues.push(candidate_validation_issue(
                    "invalid-candidate-entry",
                    &relative,
                    &format!("Cannot inspect Candidate entry: {error}"),
                ));
                continue;
            }
        };
        if file_type.is_symlink() {
            issues.push(candidate_validation_issue(
                "unsafe-candidate-entry",
                &relative,
                "Candidate entries must not be symlinks.",
            ));
        } else if file_type.is_dir() {
            collect_candidate_files(candidate_root, &path, files, issues);
        } else if file_type.is_file() {
            files.push(relative);
        } else {
            issues.push(candidate_validation_issue(
                "unsafe-candidate-entry",
                &relative,
                "Candidate entries must be regular files or directories.",
            ));
        }
    }
}

fn inspect_completion_directory(
    completion_root: &Path,
    expected_files: &BTreeSet<String>,
    issues: &mut Vec<MigrationCandidateValidationIssue>,
) {
    let metadata = match fs::symlink_metadata(completion_root) {
        Err(error) if error.kind() == ErrorKind::NotFound => return,
        Err(error) => {
            issues.push(candidate_validation_issue(
                "invalid-completion-directory",
                "migration-completions",
                &format!("Cannot inspect Completion Record directory: {error}"),
            ));
            return;
        }
        Ok(metadata) => metadata,
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        issues.push(candidate_validation_issue(
            "invalid-completion-directory",
            "migration-completions",
            "Completion Records must be stored in a regular directory, not a symlink.",
        ));
        return;
    }
    let entries = match fs::read_dir(completion_root) {
        Ok(entries) => entries,
        Err(error) => {
            issues.push(candidate_validation_issue(
                "invalid-completion-directory",
                "migration-completions",
                &format!("Cannot read Completion Record directory: {error}"),
            ));
            return;
        }
    };
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                issues.push(candidate_validation_issue(
                    "invalid-completion-record",
                    "migration-completions",
                    &format!("Cannot inspect Completion Record entry: {error}"),
                ));
                continue;
            }
        };
        let name = entry.file_name().to_string_lossy().into_owned();
        let relative = format!("migration-completions/{name}");
        let file_type = match entry.file_type() {
            Ok(file_type) => file_type,
            Err(error) => {
                issues.push(candidate_validation_issue(
                    "invalid-completion-record",
                    &relative,
                    &format!("Cannot inspect Completion Record: {error}"),
                ));
                continue;
            }
        };
        if file_type.is_symlink() || !file_type.is_file() || !expected_files.contains(&name) {
            issues.push(candidate_validation_issue(
                "unexpected-completion-record",
                &relative,
                "Only regular <pending-action-id>.yaml Completion Records are allowed.",
            ));
        }
    }
}

fn parse_action_completion(
    path: &Path,
    relative: &str,
    issues: &mut Vec<MigrationCandidateValidationIssue>,
) -> Option<MigrationActionCompletion> {
    let text = match read_regular_utf8(path) {
        Ok(text) => text,
        Err(error) => {
            issues.push(candidate_validation_issue(
                "invalid-completion-record",
                relative,
                &error.to_string(),
            ));
            return None;
        }
    };
    let value: Value = match serde_yaml::from_str(&text) {
        Ok(value) => value,
        Err(error) => {
            issues.push(candidate_validation_issue(
                "invalid-completion-record",
                relative,
                &format!("Completion Record is not valid YAML: {error}"),
            ));
            return None;
        }
    };
    match serde_json::from_value(value) {
        Ok(completion) => Some(completion),
        Err(error) => {
            issues.push(candidate_validation_issue(
                "invalid-completion-record",
                relative,
                &format!("Completion Record does not match schema version 1: {error}"),
            ));
            None
        }
    }
}

fn validate_completion_identity(
    completion: &MigrationActionCompletion,
    pending: &MigrationPendingAction,
    draft_action: &MigrationDraftAction,
    manifest: &MigrationCandidateManifest,
    relative: &str,
    issues: &mut Vec<MigrationCandidateValidationIssue>,
) {
    if completion.schema_version != MIGRATION_ACTION_COMPLETION_SCHEMA_VERSION
        || completion.kind != "migration-action-completion"
        || completion.action_id != pending.id
        || completion.source_revision != manifest.source_revision
        || completion.draft_digest != manifest.draft_digest
    {
        issues.push(candidate_validation_issue(
            "completion-identity-mismatch",
            relative,
            "Completion Record identity does not match its pending action, source revision, and reviewed Draft.",
        ));
    }
    if completion.review.reviewer.trim().is_empty() || completion.review.rationale.trim().is_empty()
    {
        issues.push(candidate_validation_issue(
            "invalid-completion-review",
            relative,
            "Completion reviewer and rationale must be non-empty.",
        ));
    }
    let evidence = completion
        .review
        .evidence_refs
        .iter()
        .map(|reference| reference.trim())
        .collect::<BTreeSet<_>>();
    if evidence.is_empty()
        || evidence.contains("")
        || evidence.len() != completion.review.evidence_refs.len()
    {
        issues.push(candidate_validation_issue(
            "invalid-completion-evidence",
            relative,
            "Completion evidence_refs must contain unique non-empty references.",
        ));
    }
    let completed_checks = completion
        .completed_checks
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let expected_checks = draft_action
        .completion_checks
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    if completed_checks.len() != completion.completed_checks.len()
        || completed_checks != expected_checks
    {
        issues.push(candidate_validation_issue(
            "completion-check-mismatch",
            relative,
            "completed_checks must exactly match the checks fixed by the embedded Draft.",
        ));
    }
}

fn validate_completion_artifacts(
    candidate_root: &Path,
    completion: &MigrationActionCompletion,
    pending: &MigrationPendingAction,
    relative: &str,
    claimed_artifacts: &mut BTreeMap<String, String>,
    issues: &mut Vec<MigrationCandidateValidationIssue>,
) {
    if completion.artifacts.is_empty() {
        issues.push(candidate_validation_issue(
            "missing-completion-artifact",
            relative,
            "A Completion Record must bind at least one reviewed artifact.",
        ));
        return;
    }
    let mut action_artifacts = BTreeSet::new();
    for artifact in &completion.artifacts {
        if !action_artifacts.insert(artifact.path.clone()) {
            issues.push(candidate_validation_issue(
                "duplicate-completion-artifact",
                relative,
                "Completion artifact paths must be unique.",
            ));
            continue;
        }
        if let Some(owner) = claimed_artifacts.get(&artifact.path) {
            issues.push(candidate_validation_issue(
                "shared-completion-artifact",
                &artifact.path,
                &format!(
                    "One artifact cannot complete both {owner:?} and {:?}.",
                    pending.id
                ),
            ));
        } else {
            claimed_artifacts.insert(artifact.path.clone(), pending.id.clone());
        }
        if is_reserved_candidate_artifact(&artifact.path) {
            issues.push(candidate_validation_issue(
                "reserved-completion-artifact",
                &artifact.path,
                "Generated Candidate metadata cannot be used as completion evidence.",
            ));
            continue;
        }
        if !artifact.digest.starts_with("sha256:")
            || !is_lower_hex(&artifact.digest["sha256:".len()..], 64)
        {
            issues.push(candidate_validation_issue(
                "invalid-completion-artifact-digest",
                &artifact.path,
                "Completion artifact digest must be a lowercase SHA-256 digest.",
            ));
            continue;
        }
        let path = match safe_repository_path(candidate_root, &artifact.path) {
            Ok(path) => path,
            Err(error) => {
                issues.push(candidate_validation_issue(
                    "unsafe-completion-artifact",
                    &artifact.path,
                    &error.to_string(),
                ));
                continue;
            }
        };
        match read_regular_bytes(&path) {
            Ok(bytes) if byte_digest(&bytes) == artifact.digest => {}
            Ok(_) => issues.push(candidate_validation_issue(
                "completion-artifact-digest-mismatch",
                &artifact.path,
                "Completion artifact bytes do not match the reviewed digest.",
            )),
            Err(error) => issues.push(candidate_validation_issue(
                "invalid-completion-artifact",
                &artifact.path,
                &error.to_string(),
            )),
        }
    }
    if pending.decision == "preserve-history" {
        let history_root = format!("migration-history/{}/", pending.id);
        if !action_artifacts
            .iter()
            .all(|path| path.starts_with(&history_root))
        {
            issues.push(candidate_validation_issue(
                "active-preserved-history",
                relative,
                "preserve-history artifacts must stay under migration-history/<action-id>/.",
            ));
        }
    } else {
        for artifact in &action_artifacts {
            if !pending
                .target_paths
                .iter()
                .any(|target| artifact_covers_target(artifact, target))
            {
                issues.push(candidate_validation_issue(
                    "out-of-scope-completion-artifact",
                    artifact,
                    "A proceed artifact must stay within one of its action target paths.",
                ));
            }
        }
        for target in &pending.target_paths {
            if !action_artifacts
                .iter()
                .any(|artifact| artifact_covers_target(artifact, target))
            {
                issues.push(candidate_validation_issue(
                    "uncovered-completion-target",
                    relative,
                    &format!("No reviewed artifact covers required target {target:?}."),
                ));
            }
        }
    }
}

fn is_action_id(value: &str) -> bool {
    !value.is_empty()
        && value.bytes().enumerate().all(|(index, byte)| {
            if index == 0 {
                byte.is_ascii_lowercase()
            } else {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-'
            }
        })
}

fn is_reserved_candidate_artifact(path: &str) -> bool {
    matches!(
        path,
        ".adf/config.yaml"
            | "migration-draft.json"
            | "migration-manifest.yaml"
            | "migration-apply.lock"
    ) || path == "migration-completions"
        || path.starts_with("migration-completions/")
}

fn artifact_covers_target(artifact: &str, target: &str) -> bool {
    let artifact_parts = artifact.split('/').collect::<Vec<_>>();
    let target_parts = target.split('/').collect::<Vec<_>>();
    if artifact_parts.len() < target_parts.len() {
        return false;
    }
    if !target_parts
        .iter()
        .zip(&artifact_parts)
        .all(|(expected, actual)| {
            (expected.starts_with('<') && expected.ends_with('>') && !actual.is_empty())
                || expected == actual
        })
    {
        return false;
    }
    let target_is_file = target_parts.last().is_some_and(|part| {
        [".json", ".yaml", ".yml", ".md", ".lock"]
            .iter()
            .any(|extension| part.ends_with(extension))
    });
    !target_is_file || artifact_parts.len() == target_parts.len()
}

fn validate_candidate_activation(
    project_root: &Path,
    candidate_root: &Path,
    candidate_relative: &str,
    manifest: &MigrationCandidateManifest,
    claimed_artifacts: &BTreeMap<String, String>,
    issues: &mut Vec<MigrationCandidateValidationIssue>,
) {
    let config = match load_project_config(candidate_root) {
        Ok(config) => config,
        Err(error) => {
            issues.push(candidate_validation_issue(
                "invalid-candidate-config",
                ".adf/config.yaml",
                &error.to_string(),
            ));
            return;
        }
    };
    let lock_path = candidate_root.join(".adf/framework.lock");
    let framework_lock = match read_yaml_value(&lock_path) {
        Ok(value) => value,
        Err(error) => {
            issues.push(candidate_validation_issue(
                "invalid-framework-lock",
                ".adf/framework.lock",
                &error.to_string(),
            ));
            return;
        }
    };
    let release =
        match resolve_verified_release_for_activation(candidate_root, &framework_lock, None) {
            Ok(release) => release,
            Err(error) => {
                issues.push(candidate_validation_issue(
                    "invalid-framework-release",
                    ".adf/cache/releases",
                    &error.to_string(),
                ));
                return;
            }
        };
    validate_candidate_rules(candidate_root, &release.rule_source, issues);

    let observation_relative = format!(
        "{candidate_relative}/{}",
        config.repository_observation.trim_start_matches("./")
    );
    let signal_registry = match SignalCatalogRegistry::built_in() {
        Ok(registry) => registry,
        Err(error) => {
            issues.push(candidate_validation_issue(
                "invalid-repository-observation",
                &config.repository_observation,
                &error.to_string(),
            ));
            return;
        }
    };
    let adapter = match GitRepositoryAdapter::with_reviewed_manifest(
        project_root,
        &observation_relative,
        signal_registry,
    ) {
        Ok(adapter) => adapter,
        Err(error) => {
            issues.push(candidate_validation_issue(
                "invalid-repository-observation",
                &config.repository_observation,
                &error.to_string(),
            ));
            return;
        }
    };
    let repository = match adapter.observe() {
        Ok(repository) => repository,
        Err(error) => {
            issues.push(candidate_validation_issue(
                "invalid-repository-observation",
                &config.repository_observation,
                &error.to_string(),
            ));
            return;
        }
    };
    if repository["revision"].as_str() != Some(&manifest.source_revision) {
        issues.push(candidate_validation_issue(
            "repository-revision-mismatch",
            &config.repository_observation,
            "Repository Observation does not resolve to the Candidate source revision.",
        ));
    }
    if repository["coverage"]["status"].as_str() != Some("complete") {
        issues.push(candidate_validation_issue(
            "incomplete-repository-coverage",
            &config.repository_observation,
            "Repository Observation has source coverage gaps.",
        ));
    }

    let store = match FileProjectStore::open_with_options(
        candidate_root,
        repository.clone(),
        &config.contract_root,
        &config.decision_root,
        DocumentFormat::Auto,
        &release.schema_registry,
    ) {
        Ok(store) => store,
        Err(error) => {
            issues.push(candidate_validation_issue(
                "invalid-candidate-records",
                ".",
                &error.to_string(),
            ));
            return;
        }
    };
    if let Err(error) = store.contract_health() {
        issues.push(candidate_validation_issue(
            "invalid-candidate-records",
            ".",
            &error.to_string(),
        ));
    }
    let decisions = match store.decisions() {
        Ok(decisions) => decisions,
        Err(error) => {
            issues.push(candidate_validation_issue(
                "invalid-candidate-records",
                &config.decision_root,
                &error.to_string(),
            ));
            Vec::new()
        }
    };
    match adapter.binding_authority_refs() {
        Ok(authority_refs) => {
            let report = BindingValidationReport::build(&repository, &decisions, &authority_refs);
            if !report.is_valid() {
                let reasons = report
                    .issues
                    .iter()
                    .map(|issue| format!("{}: {}", issue.kind, issue.reason))
                    .collect::<Vec<_>>()
                    .join("; ");
                issues.push(candidate_validation_issue(
                    "invalid-candidate-bindings",
                    &config.repository_observation,
                    &reasons,
                ));
            }
        }
        Err(error) => issues.push(candidate_validation_issue(
            "invalid-candidate-bindings",
            &config.repository_observation,
            &error.to_string(),
        )),
    }
    validate_candidate_record_inventory(
        candidate_root,
        &store,
        manifest,
        claimed_artifacts,
        issues,
    );
}

fn validate_candidate_rules(
    candidate_root: &Path,
    signed_rule_source: &Value,
    issues: &mut Vec<MigrationCandidateValidationIssue>,
) {
    let path = candidate_root.join("rules.yaml");
    if !path.exists() {
        return;
    }
    match read_yaml_value(&path) {
        Ok(candidate_rules) if candidate_rules == *signed_rule_source => {}
        Ok(_) => issues.push(candidate_validation_issue(
            "unsigned-candidate-rules",
            "rules.yaml",
            "Reviewed Candidate rules must exactly match the selected signed Framework Release rules.",
        )),
        Err(error) => issues.push(candidate_validation_issue(
            "invalid-candidate-rules",
            "rules.yaml",
            &error.to_string(),
        )),
    }
}

fn validate_candidate_record_inventory(
    candidate_root: &Path,
    store: &FileProjectStore<'_>,
    manifest: &MigrationCandidateManifest,
    claimed_artifacts: &BTreeMap<String, String>,
    issues: &mut Vec<MigrationCandidateValidationIssue>,
) {
    let record_actions = ["contracts", "decisions", "change-workflow", "evidence"]
        .into_iter()
        .collect::<BTreeSet<_>>();
    let active_actions = manifest
        .pending_actions
        .iter()
        .filter(|action| {
            action.decision == "proceed" && record_actions.contains(action.id.as_str())
        })
        .map(|action| action.id.as_str())
        .collect::<BTreeSet<_>>();
    let expected = claimed_artifacts
        .iter()
        .filter(|(_, action_id)| active_actions.contains(action_id.as_str()))
        .map(|(path, _)| path.clone())
        .collect::<BTreeSet<_>>();
    let actual = match store.all_record_paths() {
        Ok(paths) => paths
            .iter()
            .map(|path| relative_display(candidate_root, path))
            .collect::<BTreeSet<_>>(),
        Err(error) => {
            issues.push(candidate_validation_issue(
                "invalid-candidate-records",
                ".",
                &error.to_string(),
            ));
            return;
        }
    };
    if actual != expected {
        let missing = expected.difference(&actual).cloned().collect::<Vec<_>>();
        let unreviewed = actual.difference(&expected).cloned().collect::<Vec<_>>();
        issues.push(candidate_validation_issue(
            "candidate-record-inventory-mismatch",
            ".",
            &format!(
                "Reviewed Record files and Schema-loaded Record files differ; missing={missing:?}, unreviewed={unreviewed:?}."
            ),
        ));
    }
}

fn read_yaml_value(path: &Path) -> Result<Value, MigrationError> {
    let text = read_regular_utf8(path)?;
    serde_yaml::from_str(&text)
        .map_err(|error| migration_error(format!("cannot parse YAML: {error}")))
}

fn parse_candidate_draft(
    bytes: &[u8],
    issues: &mut Vec<MigrationCandidateValidationIssue>,
) -> Option<MigrationDraftReport> {
    let value: Value = match serde_json::from_slice(bytes) {
        Ok(value) => value,
        Err(error) => {
            issues.push(candidate_validation_issue(
                "invalid-generated-file",
                "migration-draft.json",
                &format!("Embedded Draft is not valid JSON: {error}"),
            ));
            return None;
        }
    };
    match serde_json::from_value(value) {
        Ok(draft) => Some(draft),
        Err(error) => {
            issues.push(candidate_validation_issue(
                "invalid-generated-file",
                "migration-draft.json",
                &format!("Embedded Draft does not match schema version 2: {error}"),
            ));
            None
        }
    }
}

fn verify_candidate_generated_files(
    candidate_root: &Path,
    manifest: &MigrationCandidateManifest,
    issues: &mut Vec<MigrationCandidateValidationIssue>,
) {
    let expected_paths = [".adf/config.yaml", "migration-draft.json"];
    let manifest_paths = manifest
        .generated_files
        .iter()
        .map(|file| file.path.as_str())
        .collect::<BTreeSet<_>>();
    if manifest_paths != expected_paths.into_iter().collect() {
        issues.push(candidate_validation_issue(
            "invalid-generated-file-list",
            "migration-manifest.yaml",
            "generated_files must contain exactly the config and embedded Draft.",
        ));
    }
    for file in &manifest.generated_files {
        let path = match safe_repository_path(candidate_root, &file.path) {
            Ok(path) => path,
            Err(error) => {
                issues.push(candidate_validation_issue(
                    "unsafe-generated-file",
                    &file.path,
                    &error.to_string(),
                ));
                continue;
            }
        };
        match read_regular_bytes(&path) {
            Ok(bytes) if byte_digest(&bytes) == file.digest => {}
            Ok(_) => issues.push(candidate_validation_issue(
                "generated-file-digest-mismatch",
                &file.path,
                "Generated file bytes do not match the Manifest digest.",
            )),
            Err(error) => issues.push(candidate_validation_issue(
                "invalid-generated-file",
                &file.path,
                &error.to_string(),
            )),
        }
    }
    let config_path = candidate_root.join(".adf/config.yaml");
    if let Ok(bytes) = read_regular_bytes(&config_path)
        && bytes != candidate_config_bytes()
    {
        issues.push(candidate_validation_issue(
            "modified-config-candidate",
            ".adf/config.yaml",
            "Generated config candidate was modified.",
        ));
    }
}

fn canonical_draft_digest(path: &Path) -> Result<String, MigrationError> {
    let text = read_regular_utf8(path)?;
    let value: Value = serde_yaml::from_str(&text)
        .map_err(|error| migration_error(format!("cannot parse migration draft: {error}")))?;
    let draft: MigrationDraftReport = serde_json::from_value(value)
        .map_err(|error| migration_error(format!("cannot load migration draft: {error}")))?;
    let canonical_value = serde_json::to_value(draft)
        .map_err(|error| migration_error(format!("cannot serialize migration draft: {error}")))?;
    crate::canonical_digest(&canonical_value)
        .map_err(|error| migration_error(format!("cannot digest migration draft: {error}")))
}

fn read_regular_utf8(path: &Path) -> Result<String, MigrationError> {
    let bytes = read_regular_bytes(path)?;
    String::from_utf8(bytes)
        .map_err(|error| migration_error(format!("file is not valid UTF-8: {error}")))
}

fn read_regular_bytes(path: &Path) -> Result<Vec<u8>, MigrationError> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| migration_error(format!("cannot inspect file: {error}")))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(migration_error(
            "path must be a regular file, not a symlink",
        ));
    }
    fs::read(path).map_err(|error| migration_error(format!("cannot read file: {error}")))
}

fn candidate_validation_report(
    status: &str,
    candidate_root: String,
    source_revision: Option<String>,
    issues: Vec<MigrationCandidateValidationIssue>,
    pending_actions: Vec<String>,
) -> MigrationCandidateValidationReport {
    let pending_validations = if status == "incomplete" {
        vec!["candidate-schema-and-release".to_owned()]
    } else {
        Vec::new()
    };
    let next = match status {
        "valid" => "The Candidate is complete and may proceed to explicit application review.",
        "incomplete" if pending_actions.is_empty() => {
            "Completion Records are structurally complete. Candidate Schema and signed Framework Release validation must pass before explicit application review. No active Project files were changed."
        }
        "incomplete" => {
            "Create migration-completions/<action-id>.yaml for every pending action using the Migration Action Completion schema, then validate again. No active Project files were changed."
        }
        "blocked" => {
            "Resolve the Project-state blocker and regenerate the Candidate from the current revision. No active Project files were changed."
        }
        _ => {
            "Resolve every integrity issue or regenerate the Candidate. No active Project files were changed."
        }
    };
    MigrationCandidateValidationReport {
        schema_version: MIGRATION_CANDIDATE_VALIDATION_SCHEMA_VERSION.to_owned(),
        status: status.to_owned(),
        candidate_root,
        source_revision,
        issues,
        pending_actions,
        pending_validations,
        next: next.to_owned(),
    }
}

fn candidate_validation_issue(
    category: &str,
    path: &str,
    message: &str,
) -> MigrationCandidateValidationIssue {
    MigrationCandidateValidationIssue {
        category: category.to_owned(),
        path: path.to_owned(),
        message: message.to_owned(),
    }
}

fn resolve_candidate_output(
    root: &Path,
    relative: &Path,
) -> Result<(PathBuf, String), MigrationError> {
    let (output, relative) = candidate_location(root, relative)?;
    match fs::symlink_metadata(&output) {
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Ok(_) => {
            return Err(migration_error(format!(
                "refusing to overwrite existing migration candidate: {relative}"
            )));
        }
        Err(error) => {
            return Err(migration_error(format!(
                "cannot inspect migration candidate output: {error}"
            )));
        }
    }
    Ok((output, relative))
}

fn resolve_existing_candidate(
    root: &Path,
    relative: &Path,
) -> Result<(PathBuf, String), MigrationError> {
    let (candidate, relative) = candidate_location(root, relative)?;
    let metadata = fs::symlink_metadata(&candidate)
        .map_err(|error| migration_error(format!("cannot inspect migration candidate: {error}")))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(migration_error(
            "migration candidate must be a directory, not a symlink",
        ));
    }
    Ok((candidate, relative))
}

fn candidate_location(root: &Path, relative: &Path) -> Result<(PathBuf, String), MigrationError> {
    if relative.is_absolute() {
        return Err(migration_error(
            "migration candidate output must be repository-relative",
        ));
    }
    let parts = relative
        .components()
        .map(|component| match component {
            Component::Normal(part) => part.to_str().map(str::to_owned),
            _ => None,
        })
        .collect::<Option<Vec<_>>>()
        .ok_or_else(|| migration_error("migration candidate output contains an unsafe path"))?;
    if parts.len() < 3 || parts[0] != ".adf" || parts[1] != "migration-candidates" {
        return Err(migration_error(
            "migration candidate output must be under .adf/migration-candidates/<name>",
        ));
    }
    if parts.iter().any(|part| {
        part.is_empty()
            || !part
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    }) {
        return Err(migration_error(
            "migration candidate output components must use letters, digits, dot, underscore, or hyphen",
        ));
    }
    let relative = parts.join("/");
    let output = safe_repository_path(root, &relative)?;
    Ok((output, relative))
}

fn write_candidate_bundle(
    output_root: &Path,
    config_bytes: &[u8],
    draft_bytes: &[u8],
    manifest: &MigrationCandidateManifest,
) -> Result<(), MigrationError> {
    let parent = output_root
        .parent()
        .ok_or_else(|| migration_error("migration candidate output has no parent"))?;
    fs::create_dir_all(parent).map_err(|error| {
        migration_error(format!("cannot create migration candidate parent: {error}"))
    })?;
    fs::create_dir(output_root).map_err(|error| {
        migration_error(format!("cannot create migration candidate output: {error}"))
    })?;
    let result = (|| {
        write_candidate_file(&output_root.join(".adf/config.yaml"), config_bytes)?;
        write_candidate_file(&output_root.join("migration-draft.json"), draft_bytes)?;
        let manifest_bytes = serde_yaml::to_string(manifest)
            .map_err(|error| migration_error(format!("cannot serialize manifest: {error}")))?;
        write_candidate_file(
            &output_root.join("migration-manifest.yaml"),
            manifest_bytes.as_bytes(),
        )
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(output_root);
    }
    result
}

fn write_candidate_file(path: &Path, bytes: &[u8]) -> Result<(), MigrationError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            migration_error(format!("cannot create candidate directory: {error}"))
        })?;
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| migration_error(format!("cannot create candidate file: {error}")))?;
    file.write_all(bytes)
        .and_then(|()| file.sync_all())
        .map_err(|error| migration_error(format!("cannot write candidate file: {error}")))
}

fn byte_digest(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

fn is_lower_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn validate_action_review(
    action: &MigrationDraftAction,
    issues: &mut Vec<MigrationDraftValidationIssue>,
) {
    if !action.requires_human_review {
        if action.review.is_some() {
            issues.push(validation_issue(
                "unexpected-review",
                Some(&action.id),
                "This mechanical action does not accept a human review record.",
            ));
        }
        return;
    }
    let Some(review) = &action.review else {
        issues.push(validation_issue(
            "missing-review",
            Some(&action.id),
            "A human review record is required.",
        ));
        return;
    };
    let allowed = match action.id.as_str() {
        "decisions" | "change-workflow" => &["proceed", "retire", "preserve-history"][..],
        "legacy-policies" | "contracts" | "evidence" => &["proceed", "retire"][..],
        "repository-observation" | "framework-release" => &["proceed"][..],
        _ => &[][..],
    };
    if !allowed.contains(&review.decision.as_str()) {
        issues.push(validation_issue(
            "invalid-decision",
            Some(&action.id),
            "The review decision is not allowed for this migration action.",
        ));
    }
    if review.reviewer.trim().is_empty() {
        issues.push(validation_issue(
            "invalid-reviewer",
            Some(&action.id),
            "reviewer must not be empty.",
        ));
    }
    if review.rationale.trim().is_empty() {
        issues.push(validation_issue(
            "invalid-rationale",
            Some(&action.id),
            "rationale must not be empty.",
        ));
    }
    let evidence = review
        .evidence_refs
        .iter()
        .map(|reference| reference.trim())
        .collect::<BTreeSet<_>>();
    if evidence.is_empty() || evidence.contains("") || evidence.len() != review.evidence_refs.len()
    {
        issues.push(validation_issue(
            "invalid-evidence-refs",
            Some(&action.id),
            "evidence_refs must contain unique non-empty references.",
        ));
    }
}

fn resolve_draft_path(root: &Path, path: &Path) -> Result<PathBuf, MigrationError> {
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        let relative = path
            .to_str()
            .ok_or_else(|| migration_error("migration draft path is not valid UTF-8"))?;
        safe_repository_path(root, relative)?
    };
    let metadata = fs::symlink_metadata(&path)
        .map_err(|error| migration_error(format!("cannot inspect migration draft: {error}")))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(migration_error(
            "migration draft must be a regular file, not a symlink",
        ));
    }
    path.canonicalize()
        .map_err(|error| migration_error(format!("cannot resolve migration draft: {error}")))
}

fn validation_report(
    status: &str,
    source_revision: Option<String>,
    issues: Vec<MigrationDraftValidationIssue>,
) -> MigrationDraftValidationReport {
    let next = match status {
        "valid" => {
            "Use this reviewed Draft as the sole input to migration candidate generation. No files were changed."
        }
        "blocked" => {
            "Resolve the Project-state blocker, run migration inspect again, and regenerate the Draft. No files were changed."
        }
        _ => {
            "Resolve every issue without changing generated fields, then run migration validate-draft again. No files were changed."
        }
    };
    MigrationDraftValidationReport {
        schema_version: MIGRATION_DRAFT_VALIDATION_SCHEMA_VERSION.to_owned(),
        status: status.to_owned(),
        source_revision,
        issues,
        next: next.to_owned(),
    }
}

fn validation_issue(
    category: &str,
    action_id: Option<&str>,
    message: &str,
) -> MigrationDraftValidationIssue {
    MigrationDraftValidationIssue {
        category: category.to_owned(),
        action_id: action_id.map(str::to_owned),
        message: message.to_owned(),
    }
}

fn draft_action(component: &MigrationComponent) -> Result<MigrationDraftAction, MigrationError> {
    let (operation, requires_human_review, instruction, checks): (&str, bool, &str, &[&str]) =
        match component.id.as_str() {
            "project-source-inventory" => (
                "inventory-only",
                false,
                "Use the inventoried repository-relative roots as migration inputs; do not copy the legacy config into the config.",
                &["Every configured source root is represented in the migration review."],
            ),
            "legacy-policies" => (
                "replace-after-review",
                true,
                "Review each legacy policy and express only accepted semantics as Rules.",
                &[
                    "Every legacy policy is mapped to a reviewed Rule or explicitly retired.",
                    "The resulting rules validate against the selected Framework Release schema.",
                ],
            ),
            "contracts" => (
                "transform-after-review",
                true,
                "Map each current Contract to reviewed Contract clauses without inferring equivalent semantics from field names.",
                &[
                    "Every source Contract is mapped or explicitly retired.",
                    "Every generated Contract validates against the Contract schema.",
                ],
            ),
            "decisions" => (
                "transform-after-review",
                true,
                "Map accepted Decision outcomes and references to reviewed Decisions.",
                &[
                    "Every source Decision is mapped or explicitly retained as history only.",
                    "Resolution references point to existing records.",
                ],
            ),
            "change-workflow" => (
                "transform-after-review",
                true,
                "Review active and historical change state, then create only the Records still required for ongoing work.",
                &[
                    "Every active legacy change has an explicit migration or closure decision.",
                    "Historical workflow data is preserved outside active state when required.",
                ],
            ),
            "evidence" => (
                "transform-after-review",
                true,
                "Attach accepted legacy Evidence to explicit requirement instances and Contract clauses.",
                &[
                    "Every retained Evidence item has an explicit requirement and clause reference.",
                    "Unmapped Evidence is reported before activation.",
                ],
            ),
            "repository-observation" => (
                "generate-from-revision",
                true,
                "Run the Detector against the fixed source revision and review every Binding candidate.",
                &[
                    "Detector coverage is complete for every tracked source file.",
                    "Every Binding candidate is reviewed before promotion.",
                ],
            ),
            "framework-release" => (
                "install-from-release",
                true,
                "Select a reviewed signed Framework Release and derive the lock and trust configuration from it.",
                &[
                    "The Framework Release signature and asset digests verify.",
                    "The selected Release supports the migrated Project schemas and Rules.",
                ],
            ),
            other => {
                return Err(migration_error(format!(
                    "unsupported migration component in draft: {other}"
                )));
            }
        };
    Ok(MigrationDraftAction {
        id: component.id.clone(),
        classification: component.classification.clone(),
        operation: operation.to_owned(),
        source_paths: component.source_paths.clone(),
        target_paths: component.target_paths.clone(),
        items: component.items,
        requires_human_review,
        instruction: instruction.to_owned(),
        completion_checks: checks.iter().map(|check| (*check).to_owned()).collect(),
        review: None,
    })
}

fn migration_components(
    inventory: &BTreeMap<String, MigrationInventory>,
    current_config: bool,
    framework_config: bool,
) -> Vec<MigrationComponent> {
    if framework_config {
        return vec![component(
            "framework-activation",
            "already-present",
            &[".adf/config.yaml", ".adf/framework.lock"],
            &[],
            1,
            "The active project already uses the project format.",
        )];
    }
    if !current_config {
        return Vec::new();
    }
    vec![
        component(
            "project-source-inventory",
            "mechanical",
            &[".agentic/config.yaml"],
            &[],
            1,
            "Repository-relative Contract, Decision, and Evidence roots can be inventoried without semantic inference.",
        ),
        component(
            "legacy-policies",
            "review-required",
            &[".agentic/config.yaml"],
            &["rules.yaml"],
            1,
            "Current risk and evidence policies do not have a field-for-field Rule mapping.",
        ),
        component(
            "contracts",
            "review-required",
            &inventory["contracts"]
                .roots
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            &["contracts"],
            inventory["contracts"].files,
            "Current Contract kinds and clauses have different semantic schemas.",
        ),
        component(
            "decisions",
            "review-required",
            &inventory["decisions"]
                .roots
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            &["decisions"],
            inventory["decisions"].files,
            "Decision status and resolution references require reviewed mapping.",
        ),
        component(
            "change-workflow",
            "review-required",
            &[".agentic/active-changes.yaml", ".agentic/changes"],
            &[".adf/changes"],
            inventory["changes"].files,
            "Current assessments, challenges, and resolved locks are workflow history rather than Records.",
        ),
        component(
            "evidence",
            "review-required",
            &inventory["evidence"]
                .roots
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            &[".adf/changes/<change-id>/evidence"],
            inventory["evidence"].files,
            "Current evidence requirements must be tied to requirement instances and Contract clauses.",
        ),
        component(
            "repository-observation",
            "generated",
            &[],
            &[".adf/repository-observation.yaml"],
            0,
            "Detector coverage and Binding candidates must be generated from the target revision, then reviewed.",
        ),
        component(
            "framework-release",
            "release-supplied",
            &[],
            &[".adf/framework.lock", ".adf/trusted-release-keys.yaml"],
            0,
            "Framework lock and trust roots come from a reviewed signed Release candidate, not legacy project data.",
        ),
    ]
}

fn component(
    id: &str,
    classification: &str,
    source_paths: &[&str],
    target_paths: &[&str],
    items: usize,
    rationale: &str,
) -> MigrationComponent {
    MigrationComponent {
        id: id.to_owned(),
        classification: classification.to_owned(),
        source_paths: source_paths.iter().map(|path| (*path).to_owned()).collect(),
        target_paths: target_paths.iter().map(|path| (*path).to_owned()).collect(),
        items,
        rationale: rationale.to_owned(),
    }
}

fn inventory_roots(
    root: &Path,
    roots: &[String],
    tracked: &BTreeSet<String>,
    required: bool,
    findings: &mut Vec<MigrationFinding>,
) -> MigrationInventory {
    let mut files = BTreeSet::new();
    let mut formats = BTreeMap::new();
    for relative in roots {
        let Ok(path) = safe_repository_path(root, relative) else {
            findings.push(finding(
                "blocking",
                "unsafe-source-root",
                relative,
                "Configured migration source root must stay inside the repository.",
            ));
            continue;
        };
        if !path.exists() {
            if required {
                findings.push(finding(
                    "blocking",
                    "missing-source-root",
                    relative,
                    "Configured migration source root does not exist.",
                ));
            }
            continue;
        }
        collect_documents(root, &path, &mut files, &mut formats, findings);
    }
    let tracked_files = files.iter().filter(|path| tracked.contains(*path)).count();
    MigrationInventory {
        roots: roots.to_vec(),
        files: files.len(),
        tracked_files,
        formats,
    }
}

fn collect_documents(
    root: &Path,
    directory: &Path,
    files: &mut BTreeSet<String>,
    formats: &mut BTreeMap<String, usize>,
    findings: &mut Vec<MigrationFinding>,
) {
    let Ok(metadata) = fs::symlink_metadata(directory) else {
        return;
    };
    if metadata.file_type().is_symlink() {
        findings.push(finding(
            "blocking",
            "symlinked-source",
            relative_display(root, directory),
            "Migration inspection does not follow symlinked source paths.",
        ));
        return;
    }
    if metadata.is_file() {
        record_document(root, directory, files, formats);
        return;
    }
    let Ok(entries) = fs::read_dir(directory) else {
        findings.push(finding(
            "blocking",
            "unreadable-source",
            relative_display(root, directory),
            "Migration source directory cannot be read.",
        ));
        return;
    };
    let mut entries = match entries.collect::<Result<Vec<_>, _>>() {
        Ok(entries) => entries,
        Err(_) => {
            findings.push(finding(
                "blocking",
                "unreadable-source",
                relative_display(root, directory),
                "A migration source directory entry cannot be read.",
            ));
            return;
        }
    };
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        collect_documents(root, &entry.path(), files, formats, findings);
    }
}

fn record_document(
    root: &Path,
    path: &Path,
    files: &mut BTreeSet<String>,
    formats: &mut BTreeMap<String, usize>,
) {
    let Some(extension) = path.extension().and_then(|extension| extension.to_str()) else {
        return;
    };
    let format = match extension.to_ascii_lowercase().as_str() {
        "yaml" | "yml" => "yaml",
        "json" => "json",
        "md" => "markdown",
        _ => return,
    };
    files.insert(relative_display(root, path));
    *formats.entry(format.to_owned()).or_default() += 1;
}

fn read_control_yaml(
    root: &Path,
    path: &Path,
    relative: &str,
    findings: &mut Vec<MigrationFinding>,
) -> Option<Value> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => return None,
        Err(_) => {
            findings.push(finding(
                "blocking",
                "unreadable-control-file",
                relative,
                "Migration control file metadata cannot be read.",
            ));
            return None;
        }
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        findings.push(finding(
            "blocking",
            "unsafe-control-file",
            relative,
            "Migration control file must be a regular file, not a symlink.",
        ));
        return None;
    }
    let Ok(path) = safe_repository_path(root, relative) else {
        findings.push(finding(
            "blocking",
            "unsafe-control-file",
            relative,
            "Migration control file resolves outside the repository.",
        ));
        return None;
    };
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(_) => {
            findings.push(finding(
                "blocking",
                "unreadable-control-file",
                relative,
                "Migration control file cannot be read as UTF-8.",
            ));
            return None;
        }
    };
    match serde_yaml::from_str(&text) {
        Ok(value) => Some(value),
        Err(_) => {
            findings.push(finding(
                "blocking",
                "invalid-control-file",
                relative,
                "Migration control file is not valid YAML.",
            ));
            None
        }
    }
}

fn legacy_installation(path: &Path, value: Option<&Value>) -> LegacyInstallation {
    LegacyInstallation {
        present: path.is_file(),
        schema_version: value.and_then(|value| scalar_string(&value["schema_version"])),
        kit_version: value.and_then(|value| scalar_string(&value["kit_version"])),
        mode: value.and_then(|value| scalar_string(&value["mode"])),
        level: value.and_then(|value| scalar_string(&value["level"])),
    }
}

fn scalar_string(value: &Value) -> Option<String> {
    match value {
        Value::String(value) => Some(value.clone()),
        Value::Number(value) => Some(value.to_string()),
        _ => None,
    }
}

fn configured_string(config: &Option<Value>, field: &str) -> Option<String> {
    config.as_ref()?.get(field)?.as_str().map(str::to_owned)
}

fn configured_string_list(config: &Option<Value>, field: &str) -> Option<Vec<String>> {
    let values = config.as_ref()?.get(field)?.as_array()?;
    values
        .iter()
        .map(|value| value.as_str().map(str::to_owned))
        .collect()
}

fn configured_framework_string(config: &Option<Value>, field: &str) -> Option<String> {
    config
        .as_ref()?
        .get("project_sources")?
        .get(field)?
        .as_str()
        .map(str::to_owned)
}

fn normalized_roots(
    values: Option<Vec<String>>,
    fallback: &str,
    field: &str,
    required: bool,
    findings: &mut Vec<MigrationFinding>,
) -> Vec<String> {
    let Some(values) = values else {
        if required {
            findings.push(finding(
                "blocking",
                "invalid-config-field",
                ".adf/config.yaml",
                &format!("{field} must contain repository-relative path strings."),
            ));
        }
        return vec![fallback.to_owned()];
    };
    let value_count = values.len();
    let valid = !values.is_empty() && values.iter().all(|value| !value.trim().is_empty());
    let unique = values.into_iter().collect::<BTreeSet<_>>();
    if !valid || unique.is_empty() || unique.len() != value_count {
        findings.push(finding(
            "blocking",
            "invalid-config-field",
            ".adf/config.yaml",
            &format!("{field} must contain non-empty repository-relative path strings."),
        ));
        vec![fallback.to_owned()]
    } else {
        unique.into_iter().collect()
    }
}

fn regular_marker(root: &Path, relative: &str, findings: &mut Vec<MigrationFinding>) -> bool {
    let path = root.join(relative);
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => return false,
        Err(_) => {
            findings.push(finding(
                "blocking",
                "unreadable-activation-file",
                relative,
                "activation file metadata cannot be read.",
            ));
            return false;
        }
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        findings.push(finding(
            "blocking",
            "unsafe-activation-file",
            relative,
            "activation file must be a regular file, not a symlink.",
        ));
        return false;
    }
    safe_repository_path(root, relative).is_ok()
}

fn safe_repository_path(root: &Path, relative: &str) -> Result<PathBuf, MigrationError> {
    let path = Path::new(relative);
    if path.is_absolute() {
        return Err(migration_error(
            "migration path must be repository-relative",
        ));
    }
    let mut candidate = root.to_path_buf();
    for component in path.components() {
        match component {
            Component::Normal(part) => {
                candidate.push(part);
                let metadata = match fs::symlink_metadata(&candidate) {
                    Ok(metadata) => metadata,
                    Err(error) if error.kind() == ErrorKind::NotFound => continue,
                    Err(error) => return Err(migration_error(error.to_string())),
                };
                if metadata.file_type().is_symlink() {
                    return Err(migration_error("migration path contains a symlink"));
                }
                let resolved = candidate
                    .canonicalize()
                    .map_err(|error| migration_error(error.to_string()))?;
                if !resolved.starts_with(root) {
                    return Err(migration_error("migration path escapes repository"));
                }
            }
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(migration_error("migration path escapes repository"));
            }
        }
    }
    Ok(candidate)
}

fn assert_git_top_level(root: &Path) -> Result<(), MigrationError> {
    let top = git(root, &["rev-parse", "--show-toplevel"])?;
    let top = PathBuf::from(top.trim())
        .canonicalize()
        .map_err(|error| migration_error(error.to_string()))?;
    if top != root {
        return Err(migration_error(format!(
            "project root is not Git top-level: {}",
            root.display()
        )));
    }
    Ok(())
}

fn git(root: &Path, arguments: &[&str]) -> Result<String, MigrationError> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(arguments)
        .output()
        .map_err(|error| migration_error(format!("cannot execute git: {error}")))?;
    if !output.status.success() {
        return Err(migration_error(format!(
            "git {} failed: {}",
            arguments.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    String::from_utf8(output.stdout)
        .map_err(|error| migration_error(format!("git output is not UTF-8: {error}")))
        .map(|output| output.trim_end().to_owned())
}

fn finding(
    severity: &str,
    category: &str,
    path: impl Into<String>,
    message: &str,
) -> MigrationFinding {
    MigrationFinding {
        severity: severity.to_owned(),
        category: category.to_owned(),
        path: path.into(),
        message: message.to_owned(),
    }
}

fn relative_display(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn display_optional(value: &Option<String>) -> &str {
    value.as_deref().unwrap_or("-")
}

fn display_paths(paths: &[String]) -> String {
    if paths.is_empty() {
        "-".to_owned()
    } else {
        paths.join(",")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationError {
    message: String,
}

impl fmt::Display for MigrationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for MigrationError {}

fn migration_error(message: impl Into<String>) -> MigrationError {
    MigrationError {
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn temporary_root(label: &str) -> PathBuf {
        static SEQUENCE: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "adf-migration-{label}-{}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&root).unwrap();
        root.canonicalize().unwrap()
    }

    #[test]
    fn application_preflight_never_overwrites_an_unarchived_target() {
        let root = temporary_root("preflight");
        fs::write(root.join("rules.yaml"), b"legacy\n").unwrap();
        let payloads = vec![MigrationApplicationPayload {
            relative: "rules.yaml".to_owned(),
            bytes: b"framework\n".to_vec(),
        }];

        let error = preflight_application_targets(&root, &[], &payloads).unwrap_err();
        assert!(error.to_string().contains("refusing to overwrite"));
        preflight_application_targets(&root, &["rules.yaml".to_owned()], &payloads).unwrap();
        assert_eq!(fs::read(root.join("rules.yaml")).unwrap(), b"legacy\n");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn failed_application_rollback_restores_archived_sources() {
        let root = temporary_root("rollback");
        let source = root.join(".adf/config.yaml");
        let archive_base = root.join(".adf/migration-history/migration-test");
        let archive = archive_base.join("source/.adf/config.yaml");
        let application_root = root.join(".adf/migrations/migration-test");
        fs::create_dir_all(source.parent().unwrap()).unwrap();
        fs::write(&source, b"legacy\n").unwrap();
        fs::create_dir_all(archive.parent().unwrap()).unwrap();
        fs::rename(&source, &archive).unwrap();
        fs::write(&source, b"partial-framework\n").unwrap();
        fs::create_dir_all(&application_root).unwrap();
        fs::write(application_root.join("partial.yaml"), b"partial\n").unwrap();

        rollback_failed_application(
            &root,
            &application_root,
            &archive_base,
            std::slice::from_ref(&source),
            &[(source.clone(), archive)],
        )
        .unwrap();

        assert_eq!(fs::read(&source).unwrap(), b"legacy\n");
        assert!(!application_root.exists());
        assert!(!archive_base.exists());
        fs::remove_dir_all(root).unwrap();
    }
}
