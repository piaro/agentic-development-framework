//! Read-only, repository-wide detector coverage audit.
//!
//! Unlike the reviewed benchmark corpus, an audit has no semantic ground
//! truth and therefore never claims precision or recall. It inventories every
//! Git-tracked source recognized by the language registry and reports parser
//! coverage, observation distribution, and framework candidates for review.

use crate::canonical_digest;
use crate::framework_detection::{FrameworkCatalog, is_framework_manifest_path};
use crate::source_detection::{SourceObservationKind, detector_for_path, source_pathspecs};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

pub const DETECTOR_AUDIT_REPORT_SCHEMA_VERSION: &str = "1";
const MAX_SOURCE_BYTES: u64 = 4 * 1024 * 1024;
const MAX_MANIFEST_BYTES: u64 = 1024 * 1024;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct AuditObservationCounts {
    pub total: usize,
    pub db_write: usize,
    pub message_publish: usize,
    pub other_method_call: usize,
}

impl AuditObservationCounts {
    fn record(&mut self, kind: SourceObservationKind) {
        self.total += 1;
        match kind {
            SourceObservationKind::DbWrite => self.db_write += 1,
            SourceObservationKind::MessagePublish => self.message_publish += 1,
            SourceObservationKind::OtherMethodCall => self.other_method_call += 1,
        }
    }

    fn add(&mut self, other: &Self) {
        self.total += other.total;
        self.db_write += other.db_write;
        self.message_publish += other.message_publish;
        self.other_method_call += other.other_method_call;
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct AuditFrameworkCandidateCounts {
    pub total: usize,
    pub method_binding_required: usize,
    pub empty_suggestions: usize,
}

impl AuditFrameworkCandidateCounts {
    fn add(&mut self, other: &Self) {
        self.total += other.total;
        self.method_binding_required += other.method_binding_required;
        self.empty_suggestions += other.empty_suggestions;
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DetectorAuditFile {
    pub path: String,
    pub language: String,
    pub status: String,
    pub detail: Option<String>,
    pub observations: AuditObservationCounts,
    pub framework_candidates: AuditFrameworkCandidateCounts,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DetectorAuditLanguage {
    pub language: String,
    pub detector_status: String,
    pub files: usize,
    pub parsed_files: usize,
    pub failed_files: usize,
    pub observations: AuditObservationCounts,
    pub framework_candidates: AuditFrameworkCandidateCounts,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DetectorAuditFramework {
    pub framework: String,
    pub candidates: usize,
    pub method_binding_required: usize,
    pub empty_suggestions: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct DetectorAuditCandidate {
    pub path: String,
    pub language: String,
    pub framework: String,
    pub symbol: String,
    pub resource: String,
    pub method: String,
    pub line: usize,
    pub suggested_fact_kinds: Vec<String>,
    pub method_binding_required: bool,
    pub evidence: Vec<String>,
    pub rationale: String,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Deserialize, Serialize)]
pub struct DetectorAuditGap {
    pub category: String,
    pub path: String,
    pub language: Option<String>,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DetectorAuditSummary {
    pub tracked_source_files: usize,
    pub supported_source_files: usize,
    pub unsupported_source_files: usize,
    pub parsed_files: usize,
    pub parse_failures: usize,
    pub source_read_failures: usize,
    pub manifests_discovered: usize,
    pub manifests_loaded: usize,
    pub manifest_failures: usize,
    pub observations: AuditObservationCounts,
    pub framework_candidates: AuditFrameworkCandidateCounts,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DetectorAuditReport {
    pub schema_version: String,
    pub status: String,
    pub revision: String,
    pub working_tree_clean: bool,
    pub content_digest: String,
    pub summary: DetectorAuditSummary,
    pub languages: Vec<DetectorAuditLanguage>,
    pub frameworks: Vec<DetectorAuditFramework>,
    pub candidate_records: Vec<DetectorAuditCandidate>,
    pub files: Vec<DetectorAuditFile>,
    pub gaps: Vec<DetectorAuditGap>,
}

impl DetectorAuditReport {
    pub fn complete(&self) -> bool {
        self.status == "complete"
    }

    pub fn as_value(&self) -> Value {
        serde_json::to_value(self).expect("Detector Audit Report is serializable")
    }

    pub fn render_text(&self) -> String {
        let mut lines = vec![
            format!("Detector audit: {}", self.status),
            format!("revision: {}", self.revision),
            format!("working_tree_clean: {}", self.working_tree_clean),
            format!("content_digest: {}", self.content_digest),
            format!(
                "sources: tracked={} supported={} unsupported={} parsed={} parse_failures={} read_failures={}",
                self.summary.tracked_source_files,
                self.summary.supported_source_files,
                self.summary.unsupported_source_files,
                self.summary.parsed_files,
                self.summary.parse_failures,
                self.summary.source_read_failures,
            ),
            format!(
                "observations: total={} db_write={} message_publish={} other_method_call={}",
                self.summary.observations.total,
                self.summary.observations.db_write,
                self.summary.observations.message_publish,
                self.summary.observations.other_method_call,
            ),
            format!(
                "framework_candidates: total={} method_binding_required={} empty_suggestions={}",
                self.summary.framework_candidates.total,
                self.summary.framework_candidates.method_binding_required,
                self.summary.framework_candidates.empty_suggestions,
            ),
        ];
        for framework in &self.frameworks {
            lines.push(format!(
                "framework: {} candidates={} method_binding_required={} empty_suggestions={}",
                framework.framework,
                framework.candidates,
                framework.method_binding_required,
                framework.empty_suggestions,
            ));
        }
        for gap in &self.gaps {
            lines.push(format!("- {}: {} ({})", gap.category, gap.path, gap.detail));
        }
        lines.join("\n") + "\n"
    }
}

pub fn run_repository_detector_audit(
    repository_root: &Path,
    require_clean: bool,
) -> Result<DetectorAuditReport, DetectorAuditError> {
    let root = repository_root
        .canonicalize()
        .map_err(|error| audit_error(format!("cannot resolve repository root: {error}")))?;
    if !root.is_dir() {
        return Err(audit_error(format!(
            "repository root is not a directory: {}",
            root.display()
        )));
    }
    let top_level = PathBuf::from(git_text(&root, &["rev-parse", "--show-toplevel"])?);
    let top_level = top_level
        .canonicalize()
        .map_err(|error| audit_error(format!("cannot resolve Git top-level: {error}")))?;
    if top_level != root {
        return Err(audit_error(format!(
            "repository root is not Git top-level: {}",
            root.display()
        )));
    }
    let revision = git_text(&root, &["rev-parse", "HEAD"])?;
    let working_tree_clean =
        git_text(&root, &["status", "--porcelain", "--untracked-files=all"])?.is_empty();
    if require_clean && !working_tree_clean {
        return Err(audit_error("Git working tree is not clean"));
    }

    let tracked_paths = git_paths(&root, &[])?;
    let manifest_paths = tracked_paths
        .iter()
        .filter(|path| is_framework_manifest_path(path))
        .cloned()
        .collect::<Vec<_>>();
    let source_paths = git_paths(&root, &source_pathspecs())?;
    let mut catalog = FrameworkCatalog::default();
    let mut digest_files = Vec::new();
    let mut gaps = Vec::new();
    let mut manifests_loaded = 0;

    for relative in &manifest_paths {
        match read_regular_utf8(&root, relative, MAX_MANIFEST_BYTES) {
            Ok(content) => {
                catalog.record_manifest(relative, &content);
                digest_files.push(json!({"path": relative, "content": content}));
                manifests_loaded += 1;
            }
            Err(detail) => gaps.push(DetectorAuditGap {
                category: "manifest-read-error".to_owned(),
                path: relative.clone(),
                language: None,
                detail,
            }),
        }
    }

    let mut files = Vec::new();
    let mut frameworks = BTreeMap::<String, DetectorAuditFramework>::new();
    let mut candidate_records = Vec::new();
    for relative in &source_paths {
        let detector = detector_for_path(relative)
            .expect("Git source pathspecs and the detector registry stay aligned");
        let language = detector.language.to_owned();
        if !detector.is_supported() {
            let detail = format!("language {} is not supported", detector.language);
            gaps.push(DetectorAuditGap {
                category: "unsupported-language".to_owned(),
                path: relative.clone(),
                language: Some(language.clone()),
                detail: detail.clone(),
            });
            files.push(DetectorAuditFile {
                path: relative.clone(),
                language,
                status: "unsupported".to_owned(),
                detail: Some(detail),
                observations: AuditObservationCounts::default(),
                framework_candidates: AuditFrameworkCandidateCounts::default(),
            });
            continue;
        }
        let source = match read_regular_utf8(&root, relative, MAX_SOURCE_BYTES) {
            Ok(source) => source,
            Err(detail) => {
                gaps.push(DetectorAuditGap {
                    category: "source-read-error".to_owned(),
                    path: relative.clone(),
                    language: Some(language.clone()),
                    detail: detail.clone(),
                });
                files.push(DetectorAuditFile {
                    path: relative.clone(),
                    language,
                    status: "read-error".to_owned(),
                    detail: Some(detail),
                    observations: AuditObservationCounts::default(),
                    framework_candidates: AuditFrameworkCandidateCounts::default(),
                });
                continue;
            }
        };
        digest_files.push(json!({"path": relative, "content": source}));
        let observations = match detector.observe(&source) {
            Ok(observations) => observations,
            Err(detail) => {
                gaps.push(DetectorAuditGap {
                    category: "parse-error".to_owned(),
                    path: relative.clone(),
                    language: Some(language.clone()),
                    detail: detail.clone(),
                });
                files.push(DetectorAuditFile {
                    path: relative.clone(),
                    language,
                    status: "parse-error".to_owned(),
                    detail: Some(detail),
                    observations: AuditObservationCounts::default(),
                    framework_candidates: AuditFrameworkCandidateCounts::default(),
                });
                continue;
            }
        };
        let candidates = catalog.candidates(relative, detector.language, &source, &observations);
        let mut observation_counts = AuditObservationCounts::default();
        for observation in &observations {
            observation_counts.record(observation.kind);
        }
        let candidate_counts = AuditFrameworkCandidateCounts {
            total: candidates.len(),
            method_binding_required: candidates
                .iter()
                .filter(|candidate| candidate.method_binding_required)
                .count(),
            empty_suggestions: candidates
                .iter()
                .filter(|candidate| candidate.suggested_fact_kinds.is_empty())
                .count(),
        };
        for candidate in &candidates {
            let framework = frameworks
                .entry(candidate.framework.to_owned())
                .or_insert_with(|| DetectorAuditFramework {
                    framework: candidate.framework.to_owned(),
                    candidates: 0,
                    method_binding_required: 0,
                    empty_suggestions: 0,
                });
            framework.candidates += 1;
            if candidate.method_binding_required {
                framework.method_binding_required += 1;
            }
            if candidate.suggested_fact_kinds.is_empty() {
                framework.empty_suggestions += 1;
            }
            candidate_records.push(DetectorAuditCandidate {
                path: relative.clone(),
                language: detector.language.to_owned(),
                framework: candidate.framework.to_owned(),
                symbol: candidate.symbol.clone(),
                resource: candidate.resource.clone(),
                method: candidate.method.clone(),
                line: candidate.line,
                suggested_fact_kinds: candidate
                    .suggested_fact_kinds
                    .iter()
                    .map(|kind| kind.as_str().to_owned())
                    .collect(),
                method_binding_required: candidate.method_binding_required,
                evidence: candidate.evidence.clone(),
                rationale: candidate.rationale.to_owned(),
            });
        }
        files.push(DetectorAuditFile {
            path: relative.clone(),
            language,
            status: "parsed".to_owned(),
            detail: None,
            observations: observation_counts,
            framework_candidates: candidate_counts,
        });
    }

    files.sort_by(|left, right| left.path.cmp(&right.path));
    gaps.sort_by(|left, right| {
        (&left.category, &left.path, &left.detail).cmp(&(
            &right.category,
            &right.path,
            &right.detail,
        ))
    });
    digest_files.sort_by(|left, right| left["path"].as_str().cmp(&right["path"].as_str()));
    let content_digest = canonical_digest(&json!({
        "revision": revision,
        "tracked_source_paths": source_paths,
        "framework_manifest_paths": manifest_paths,
        "files": digest_files,
    }))
    .map_err(|error| audit_error(error.to_string()))?;

    let mut languages = BTreeMap::<String, DetectorAuditLanguage>::new();
    let mut observations = AuditObservationCounts::default();
    let mut framework_candidates = AuditFrameworkCandidateCounts::default();
    for file in &files {
        observations.add(&file.observations);
        framework_candidates.add(&file.framework_candidates);
        let language =
            languages
                .entry(file.language.clone())
                .or_insert_with(|| DetectorAuditLanguage {
                    language: file.language.clone(),
                    detector_status: if file.status == "unsupported" {
                        "unsupported".to_owned()
                    } else {
                        "supported".to_owned()
                    },
                    files: 0,
                    parsed_files: 0,
                    failed_files: 0,
                    observations: AuditObservationCounts::default(),
                    framework_candidates: AuditFrameworkCandidateCounts::default(),
                });
        language.files += 1;
        if file.status == "parsed" {
            language.parsed_files += 1;
        } else {
            language.failed_files += 1;
        }
        language.observations.add(&file.observations);
        language
            .framework_candidates
            .add(&file.framework_candidates);
    }
    let languages = languages.into_values().collect::<Vec<_>>();
    let frameworks = frameworks.into_values().collect::<Vec<_>>();
    candidate_records.sort();
    let summary = DetectorAuditSummary {
        tracked_source_files: files.len(),
        supported_source_files: files
            .iter()
            .filter(|file| file.status != "unsupported")
            .count(),
        unsupported_source_files: files
            .iter()
            .filter(|file| file.status == "unsupported")
            .count(),
        parsed_files: files.iter().filter(|file| file.status == "parsed").count(),
        parse_failures: gaps
            .iter()
            .filter(|gap| gap.category == "parse-error")
            .count(),
        source_read_failures: gaps
            .iter()
            .filter(|gap| gap.category == "source-read-error")
            .count(),
        manifests_discovered: manifest_paths.len(),
        manifests_loaded,
        manifest_failures: gaps
            .iter()
            .filter(|gap| gap.category == "manifest-read-error")
            .count(),
        observations,
        framework_candidates,
    };
    Ok(DetectorAuditReport {
        schema_version: DETECTOR_AUDIT_REPORT_SCHEMA_VERSION.to_owned(),
        status: if gaps.is_empty() {
            "complete"
        } else {
            "blocked"
        }
        .to_owned(),
        revision,
        working_tree_clean,
        content_digest,
        summary,
        languages,
        frameworks,
        candidate_records,
        files,
        gaps,
    })
}

fn read_regular_utf8(root: &Path, relative: &str, limit: u64) -> Result<String, String> {
    let path = root.join(relative);
    let resolved = path.canonicalize().map_err(|error| error.to_string())?;
    if resolved != path || !resolved.starts_with(root) {
        return Err("tracked path contains a symlink or escapes the repository".to_owned());
    }
    let metadata = fs::symlink_metadata(&path).map_err(|error| error.to_string())?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err("tracked path is not a regular file".to_owned());
    }
    if metadata.len() > limit {
        return Err(format!("file exceeds the {} byte audit limit", limit));
    }
    fs::read_to_string(&path).map_err(|error| error.to_string())
}

fn git_text(root: &Path, arguments: &[&str]) -> Result<String, DetectorAuditError> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(arguments)
        .output()
        .map_err(|error| audit_error(format!("cannot execute Git: {error}")))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        return Err(audit_error(if stderr.is_empty() { stdout } else { stderr }));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn git_paths(root: &Path, pathspecs: &[String]) -> Result<Vec<String>, DetectorAuditError> {
    let mut command = Command::new("git");
    command
        .arg("-C")
        .arg(root)
        .args(["ls-files", "-z", "--cached"]);
    if !pathspecs.is_empty() {
        command.arg("--").args(pathspecs);
    }
    let output = command
        .output()
        .map_err(|error| audit_error(format!("cannot execute Git: {error}")))?;
    if !output.status.success() {
        return Err(audit_error(
            String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        ));
    }
    let mut paths = output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
        .map(|path| {
            String::from_utf8(path.to_vec())
                .map_err(|_| audit_error("Git source path is not valid UTF-8"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    paths.sort();
    paths.dedup();
    Ok(paths)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetectorAuditError {
    message: String,
}

impl fmt::Display for DetectorAuditError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for DetectorAuditError {}

fn audit_error(message: impl Into<String>) -> DetectorAuditError {
    DetectorAuditError {
        message: message.into(),
    }
}
