//! Read-only inspection of a current CLI project before vNext migration.
//!
//! This module inventories mechanically observable state and prepares a
//! reviewable work plan. It never writes Project files or decides whether
//! legacy semantics are valid.

use serde::Serialize;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;
use std::io::ErrorKind;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

pub const MIGRATION_INSPECTION_SCHEMA_VERSION: &str = "1";
pub const MIGRATION_DRAFT_SCHEMA_VERSION: &str = "1";

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

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
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
    let root = project_root
        .as_ref()
        .canonicalize()
        .map_err(|error| migration_error(format!("cannot resolve project root: {error}")))?;
    if !root.is_dir() {
        return Err(migration_error("project root must be a directory"));
    }
    assert_git_top_level(&root)?;
    let revision = git(&root, &["rev-parse", "HEAD"])?;
    let status = git(
        &root,
        &["status", "--porcelain=v1", "--untracked-files=all"],
    )?;
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

    let config_path = root.join(".agentic/config.yaml");
    let config = read_control_yaml(&root, &config_path, ".agentic/config.yaml", &mut findings);
    let current_config = config
        .as_ref()
        .and_then(Value::as_object)
        .is_some_and(|object| object.contains_key("contract_roots"));
    let vnext_config = config
        .as_ref()
        .and_then(Value::as_object)
        .is_some_and(|object| object.contains_key("project_sources"));
    if current_config
        && config
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
    if vnext_config
        && config
            .as_ref()
            .and_then(|value| value["schema_version"].as_str())
            != Some("1")
    {
        findings.push(finding(
            "blocking",
            "unsupported-vnext-config",
            ".agentic/config.yaml",
            "vNext config schema_version must be the string 1.",
        ));
    }
    if config_path.is_file() && !current_config && !vnext_config {
        findings.push(finding(
            "blocking",
            "unknown-config-format",
            ".agentic/config.yaml",
            "The project config is neither the current CLI format nor the vNext format.",
        ));
    }

    let vnext_markers = [
        ".agentic/framework.lock",
        ".agentic/repository-observation.yaml",
        ".agentic/trusted-release-keys.yaml",
    ];
    let present_vnext_markers = vnext_markers
        .iter()
        .filter(|relative| regular_marker(&root, relative, &mut findings))
        .copied()
        .collect::<Vec<_>>();
    let source_state = if current_config && !present_vnext_markers.is_empty() {
        findings.push(finding(
            "blocking",
            "mixed-runtime-state",
            ".agentic",
            "Current CLI config and vNext activation files coexist; reconcile the active runtime before preparing a migration.",
        ));
        "mixed"
    } else if current_config {
        "current"
    } else if vnext_config {
        "vnext"
    } else if installation.present {
        findings.push(finding(
            "blocking",
            "missing-current-config",
            ".agentic/config.yaml",
            "A current CLI installation exists but its config is missing or unreadable.",
        ));
        "current-incomplete"
    } else if !present_vnext_markers.is_empty() {
        findings.push(finding(
            "blocking",
            "incomplete-vnext-project",
            ".agentic",
            "vNext activation files exist without a recognizable vNext config.",
        ));
        "vnext-incomplete"
    } else {
        findings.push(finding(
            "blocking",
            "uninitialized-project",
            ".agentic",
            "No current CLI or vNext project configuration was found.",
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

    if source_state == "vnext" {
        for marker in vnext_markers {
            if !present_vnext_markers.contains(&marker) {
                findings.push(finding(
                    "blocking",
                    "missing-vnext-activation-file",
                    marker,
                    "The vNext project is not fully activatable.",
                ));
            }
        }
    }

    let configured_contract_roots = if current_config {
        configured_string_list(&config, "contract_roots")
    } else {
        configured_vnext_string(&config, "contracts").map(|path| vec![path])
    };
    let configured_decision_roots = if current_config {
        configured_string(&config, "decision_root").map(|path| vec![path])
    } else {
        configured_vnext_string(&config, "decisions").map(|path| vec![path])
    };
    let contract_roots = normalized_roots(
        configured_contract_roots,
        "contracts",
        "contract_roots",
        current_config || vnext_config,
        &mut findings,
    );
    let decision_roots = normalized_roots(
        configured_decision_roots,
        "decisions",
        "decision_root",
        current_config || vnext_config,
        &mut findings,
    );
    let evidence_roots = normalized_roots(
        configured_string(&config, "evidence_root").map(|path| vec![path]),
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

    let components = migration_components(&inventory, current_config, vnext_config);
    findings.sort();
    let has_blocker = findings
        .iter()
        .any(|finding| finding.severity == "blocking");
    let readiness = if has_blocker {
        "blocked"
    } else if source_state == "vnext" {
        "already-vnext"
    } else {
        "review-required"
    }
    .to_owned();
    let next = match readiness.as_str() {
        "blocked" => {
            "Resolve every blocking finding, then run migration inspect again. No files were changed."
        }
        "already-vnext" => {
            "Run the vNext validation commands; no legacy migration draft is required."
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
    match inspection.readiness.as_str() {
        "blocked" => {
            return Err(migration_error(
                "migration draft is blocked; resolve the findings from `agentic migration inspect` first",
            ));
        }
        "already-vnext" => {
            return Err(migration_error(
                "migration draft is not required because the project already uses vNext",
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

fn draft_action(component: &MigrationComponent) -> Result<MigrationDraftAction, MigrationError> {
    let (operation, requires_human_review, instruction, checks): (&str, bool, &str, &[&str]) =
        match component.id.as_str() {
            "project-source-inventory" => (
                "inventory-only",
                false,
                "Use the inventoried repository-relative roots as migration inputs; do not copy the legacy config into the vNext config.",
                &["Every configured source root is represented in the migration review."],
            ),
            "legacy-policies" => (
                "replace-after-review",
                true,
                "Review each legacy policy and express only accepted semantics as vNext Rules.",
                &[
                    "Every legacy policy is mapped to a reviewed Rule or explicitly retired.",
                    "The resulting rules validate against the selected Framework Release schema.",
                ],
            ),
            "contracts" => (
                "transform-after-review",
                true,
                "Map each current Contract to reviewed vNext Contract clauses without inferring equivalent semantics from field names.",
                &[
                    "Every source Contract is mapped or explicitly retired.",
                    "Every generated Contract validates against the vNext Contract schema.",
                ],
            ),
            "decisions" => (
                "transform-after-review",
                true,
                "Map accepted Decision outcomes and references to reviewed vNext Decisions.",
                &[
                    "Every source Decision is mapped or explicitly retained as history only.",
                    "Resolution references point to existing vNext records.",
                ],
            ),
            "change-workflow" => (
                "transform-after-review",
                true,
                "Review active and historical change state, then create only the vNext Records still required for ongoing work.",
                &[
                    "Every active legacy change has an explicit migration or closure decision.",
                    "Historical workflow data is preserved outside active vNext state when required.",
                ],
            ),
            "evidence" => (
                "transform-after-review",
                true,
                "Attach accepted legacy Evidence to explicit vNext requirement instances and Contract clauses.",
                &[
                    "Every retained Evidence item has an explicit vNext requirement and clause reference.",
                    "Unmapped Evidence is reported before activation.",
                ],
            ),
            "repository-observation" => (
                "generate-from-revision",
                true,
                "Run the vNext Detector against the fixed source revision and review every Binding candidate.",
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
    })
}

fn migration_components(
    inventory: &BTreeMap<String, MigrationInventory>,
    current_config: bool,
    vnext_config: bool,
) -> Vec<MigrationComponent> {
    if vnext_config {
        return vec![component(
            "vnext-activation",
            "already-present",
            &[".agentic/config.yaml", ".agentic/framework.lock"],
            &[],
            1,
            "The active project already uses the vNext project format.",
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
            "Current risk and evidence policies do not have a field-for-field vNext Rule mapping.",
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
            "Current Contract kinds and vNext clauses have different semantic schemas.",
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
            &[".agentic/changes"],
            inventory["changes"].files,
            "Current assessments, challenges, and resolved locks are workflow history rather than vNext Records.",
        ),
        component(
            "evidence",
            "review-required",
            &inventory["evidence"]
                .roots
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            &[".agentic/changes/<change-id>/evidence"],
            inventory["evidence"].files,
            "Current evidence requirements must be tied to vNext requirement instances and Contract clauses.",
        ),
        component(
            "repository-observation",
            "generated",
            &[],
            &[".agentic/repository-observation.yaml"],
            0,
            "Detector coverage and Binding candidates must be generated from the target revision, then reviewed.",
        ),
        component(
            "framework-release",
            "release-supplied",
            &[],
            &[
                ".agentic/framework.lock",
                ".agentic/trusted-release-keys.yaml",
            ],
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

fn configured_vnext_string(config: &Option<Value>, field: &str) -> Option<String> {
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
                ".agentic/config.yaml",
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
            ".agentic/config.yaml",
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
                "vNext activation file metadata cannot be read.",
            ));
            return false;
        }
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        findings.push(finding(
            "blocking",
            "unsafe-activation-file",
            relative,
            "vNext activation file must be a regular file, not a symlink.",
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
