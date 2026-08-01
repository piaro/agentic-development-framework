//! Deterministic quality benchmark for source and framework detectors.
//!
//! A corpus owns both reviewed ground truth and explicit score thresholds.
//! The runner is offline and read-only: it never downloads repositories or
//! turns detector output into authoritative project bindings.

use crate::canonical_digest;
use crate::framework_detection::{FrameworkCandidate, FrameworkCatalog};
use crate::project_config::repository_path;
use crate::source_detection::{SourceObservation, SourceObservationKind, detector_for_path};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::fmt;
use std::fs;
use std::path::Path;

pub const DETECTOR_BENCHMARK_REPORT_SCHEMA_VERSION: &str = "1";
pub const DETECTOR_BENCHMARK_CORPUS_SCHEMA_VERSION: &str = "1";
const SCORE_SCALE: usize = 10_000;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
struct BenchmarkCorpus {
    schema_version: String,
    id: String,
    minimum_scores: BenchmarkThresholds,
    projects: Vec<BenchmarkProject>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BenchmarkThresholds {
    pub observation_precision_bps: usize,
    pub observation_recall_bps: usize,
    pub framework_precision_bps: usize,
    pub framework_recall_bps: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
struct BenchmarkProject {
    id: String,
    root: String,
    provenance: BenchmarkProvenance,
    manifests: Vec<String>,
    cases: Vec<BenchmarkCase>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
struct BenchmarkProvenance {
    kind: String,
    repository: Option<String>,
    revision: Option<String>,
    license: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
struct BenchmarkCase {
    path: String,
    language: String,
    expected_observations: Vec<ObservationRecord>,
    expected_framework_candidates: Vec<FrameworkCandidateRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationRecord {
    pub kind: String,
    pub symbol: String,
    pub resource: String,
    pub method: String,
    pub line: usize,
}

impl From<SourceObservation> for ObservationRecord {
    fn from(observation: SourceObservation) -> Self {
        Self {
            kind: kind_name(observation.kind).to_owned(),
            symbol: observation.symbol,
            resource: observation.resource,
            method: observation.method,
            line: observation.line,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FrameworkCandidateRecord {
    pub framework: String,
    pub symbol: String,
    pub resource: String,
    pub method: String,
    pub line: usize,
    pub suggested_kind: Option<String>,
    pub method_binding_required: bool,
}

impl From<FrameworkCandidate> for FrameworkCandidateRecord {
    fn from(candidate: FrameworkCandidate) -> Self {
        Self {
            framework: candidate.framework.to_owned(),
            symbol: candidate.symbol,
            resource: candidate.resource,
            method: candidate.method,
            line: candidate.line,
            suggested_kind: candidate
                .suggested_kind
                .map(|kind| kind.as_str().to_owned()),
            method_binding_required: candidate.method_binding_required,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BenchmarkComparison<T> {
    pub expected: usize,
    pub detected: usize,
    pub matched: usize,
    pub missing: Vec<T>,
    pub unexpected: Vec<T>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BenchmarkCaseReport {
    pub project_id: String,
    pub path: String,
    pub language: String,
    pub status: String,
    pub parse_error: Option<String>,
    pub observations: BenchmarkComparison<ObservationRecord>,
    pub framework_candidates: BenchmarkComparison<FrameworkCandidateRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BenchmarkMetric {
    pub expected: usize,
    pub detected: usize,
    pub matched: usize,
    pub precision_bps: usize,
    pub recall_bps: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DetectorBenchmarkSummary {
    pub projects: usize,
    pub cases: usize,
    pub passed_cases: usize,
    pub failed_cases: usize,
    pub parse_failures: usize,
    pub observations: BenchmarkMetric,
    pub framework_candidates: BenchmarkMetric,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DetectorBenchmarkReport {
    pub schema_version: String,
    pub corpus_id: String,
    pub corpus_digest: String,
    pub status: String,
    pub minimum_scores: BenchmarkThresholds,
    pub summary: DetectorBenchmarkSummary,
    pub cases: Vec<BenchmarkCaseReport>,
}

impl DetectorBenchmarkReport {
    pub fn passed(&self) -> bool {
        self.status == "passed"
    }

    pub fn as_value(&self) -> Value {
        serde_json::to_value(self).expect("Detector Benchmark Report is serializable")
    }

    pub fn render_text(&self) -> String {
        let mut lines = vec![
            format!("Detector benchmark: {}", self.status),
            format!("corpus: {}", self.corpus_id),
            format!("corpus_digest: {}", self.corpus_digest),
            format!(
                "cases: total={} passed={} failed={} parse_failures={}",
                self.summary.cases,
                self.summary.passed_cases,
                self.summary.failed_cases,
                self.summary.parse_failures
            ),
            format!(
                "observations: precision={} recall={} matched={}/{} detected={}",
                render_score(self.summary.observations.precision_bps),
                render_score(self.summary.observations.recall_bps),
                self.summary.observations.matched,
                self.summary.observations.expected,
                self.summary.observations.detected
            ),
            format!(
                "framework_candidates: precision={} recall={} matched={}/{} detected={}",
                render_score(self.summary.framework_candidates.precision_bps),
                render_score(self.summary.framework_candidates.recall_bps),
                self.summary.framework_candidates.matched,
                self.summary.framework_candidates.expected,
                self.summary.framework_candidates.detected
            ),
        ];
        for case in self.cases.iter().filter(|case| case.status != "passed") {
            lines.push(format!(
                "- {}:{} [{}] parse_error={} observation_missing={} observation_unexpected={} framework_missing={} framework_unexpected={}",
                case.project_id,
                case.path,
                case.status,
                case.parse_error.as_deref().unwrap_or("-"),
                case.observations.missing.len(),
                case.observations.unexpected.len(),
                case.framework_candidates.missing.len(),
                case.framework_candidates.unexpected.len()
            ));
        }
        lines.join("\n") + "\n"
    }
}

pub fn run_detector_benchmark(
    corpus_root: &Path,
) -> Result<DetectorBenchmarkReport, DetectorBenchmarkError> {
    let root = corpus_root
        .canonicalize()
        .map_err(|error| benchmark_error(format!("cannot resolve benchmark corpus: {error}")))?;
    if !root.is_dir() {
        return Err(benchmark_error(format!(
            "benchmark corpus is not a directory: {}",
            root.display()
        )));
    }
    let manifest_path = repository_path(&root, "benchmark.yaml")
        .map_err(|error| benchmark_error(error.to_string()))?;
    let manifest_text = fs::read_to_string(&manifest_path)
        .map_err(|error| benchmark_error(format!("{}: {error}", manifest_path.display())))?;
    let corpus_value: Value = serde_yaml::from_str(&manifest_text)
        .map_err(|error| benchmark_error(format!("{}: {error}", manifest_path.display())))?;
    let corpus: BenchmarkCorpus = serde_json::from_value(corpus_value.clone())
        .map_err(|error| benchmark_error(format!("{}: {error}", manifest_path.display())))?;
    validate_corpus(&corpus)?;
    let corpus_digest = digest_corpus(&root, &corpus_value, &corpus)?;

    let mut reports = Vec::new();
    for project in &corpus.projects {
        let project_root = repository_path(&root, &project.root)
            .map_err(|error| benchmark_error(error.to_string()))?;
        if !project_root.is_dir() {
            return Err(benchmark_error(format!(
                "benchmark project root is not a directory: {}",
                project_root.display()
            )));
        }
        let mut catalog = FrameworkCatalog::default();
        for relative in &project.manifests {
            let path = repository_path(&project_root, relative)
                .map_err(|error| benchmark_error(error.to_string()))?;
            let content = fs::read_to_string(&path)
                .map_err(|error| benchmark_error(format!("{}: {error}", path.display())))?;
            catalog.record_manifest(relative, &content);
        }
        for case in &project.cases {
            reports.push(run_case(&project.id, &project_root, case, &catalog)?);
        }
    }
    reports.sort_by(|left, right| {
        (&left.project_id, &left.path).cmp(&(&right.project_id, &right.path))
    });

    let observation_metric = aggregate_metric(
        reports
            .iter()
            .map(|report| &report.observations)
            .collect::<Vec<_>>()
            .as_slice(),
    );
    let framework_metric = aggregate_metric(
        reports
            .iter()
            .map(|report| &report.framework_candidates)
            .collect::<Vec<_>>()
            .as_slice(),
    );
    let parse_failures = reports
        .iter()
        .filter(|report| report.parse_error.is_some())
        .count();
    let passed_cases = reports
        .iter()
        .filter(|report| report.status == "passed")
        .count();
    let summary = DetectorBenchmarkSummary {
        projects: corpus.projects.len(),
        cases: reports.len(),
        passed_cases,
        failed_cases: reports.len() - passed_cases,
        parse_failures,
        observations: observation_metric,
        framework_candidates: framework_metric,
    };
    let passed = parse_failures == 0
        && summary.observations.precision_bps >= corpus.minimum_scores.observation_precision_bps
        && summary.observations.recall_bps >= corpus.minimum_scores.observation_recall_bps
        && summary.framework_candidates.precision_bps
            >= corpus.minimum_scores.framework_precision_bps
        && summary.framework_candidates.recall_bps >= corpus.minimum_scores.framework_recall_bps;
    Ok(DetectorBenchmarkReport {
        schema_version: DETECTOR_BENCHMARK_REPORT_SCHEMA_VERSION.to_owned(),
        corpus_id: corpus.id,
        corpus_digest,
        status: if passed { "passed" } else { "failed" }.to_owned(),
        minimum_scores: corpus.minimum_scores,
        summary,
        cases: reports,
    })
}

fn run_case(
    project_id: &str,
    project_root: &Path,
    case: &BenchmarkCase,
    catalog: &FrameworkCatalog,
) -> Result<BenchmarkCaseReport, DetectorBenchmarkError> {
    let path = repository_path(project_root, &case.path)
        .map_err(|error| benchmark_error(error.to_string()))?;
    let source = fs::read_to_string(&path)
        .map_err(|error| benchmark_error(format!("{}: {error}", path.display())))?;
    let detector = detector_for_path(&case.path).ok_or_else(|| {
        benchmark_error(format!(
            "benchmark case has no registered language: {}:{}",
            project_id, case.path
        ))
    })?;
    if detector.language != case.language {
        return Err(benchmark_error(format!(
            "benchmark case language mismatch: {}:{} declares {}, extension resolves to {}",
            project_id, case.path, case.language, detector.language
        )));
    }
    let mut expected_observations = case.expected_observations.clone();
    expected_observations.sort();
    let mut expected_candidates = case.expected_framework_candidates.clone();
    expected_candidates.sort();

    let (parse_error, detected_observations, detected_candidates) = match detector.observe(&source)
    {
        Ok(observations) => {
            let candidates =
                catalog.candidates(&case.path, detector.language, &source, &observations);
            (
                None,
                observations
                    .into_iter()
                    .map(ObservationRecord::from)
                    .collect::<Vec<_>>(),
                candidates
                    .into_iter()
                    .map(FrameworkCandidateRecord::from)
                    .collect::<Vec<_>>(),
            )
        }
        Err(error) => (Some(error), Vec::new(), Vec::new()),
    };
    let observations = compare(expected_observations, detected_observations);
    let framework_candidates = compare(expected_candidates, detected_candidates);
    let status = if parse_error.is_none()
        && observations.missing.is_empty()
        && observations.unexpected.is_empty()
        && framework_candidates.missing.is_empty()
        && framework_candidates.unexpected.is_empty()
    {
        "passed"
    } else {
        "failed"
    };
    Ok(BenchmarkCaseReport {
        project_id: project_id.to_owned(),
        path: case.path.clone(),
        language: case.language.clone(),
        status: status.to_owned(),
        parse_error,
        observations,
        framework_candidates,
    })
}

fn compare<T: Ord + Clone>(mut expected: Vec<T>, mut detected: Vec<T>) -> BenchmarkComparison<T> {
    expected.sort();
    detected.sort();
    let expected_count = expected.len();
    let detected_count = detected.len();
    let mut left = 0;
    let mut right = 0;
    let mut matched = 0;
    let mut missing = Vec::new();
    let mut unexpected = Vec::new();
    while left < expected.len() && right < detected.len() {
        match expected[left].cmp(&detected[right]) {
            std::cmp::Ordering::Less => {
                missing.push(expected[left].clone());
                left += 1;
            }
            std::cmp::Ordering::Greater => {
                unexpected.push(detected[right].clone());
                right += 1;
            }
            std::cmp::Ordering::Equal => {
                matched += 1;
                left += 1;
                right += 1;
            }
        }
    }
    missing.extend_from_slice(&expected[left..]);
    unexpected.extend_from_slice(&detected[right..]);
    BenchmarkComparison {
        expected: expected_count,
        detected: detected_count,
        matched,
        missing,
        unexpected,
    }
}

fn aggregate_metric<T>(comparisons: &[&BenchmarkComparison<T>]) -> BenchmarkMetric {
    let expected = comparisons.iter().map(|item| item.expected).sum();
    let detected = comparisons.iter().map(|item| item.detected).sum();
    let matched = comparisons.iter().map(|item| item.matched).sum();
    BenchmarkMetric {
        expected,
        detected,
        matched,
        precision_bps: score(matched, detected),
        recall_bps: score(matched, expected),
    }
}

fn score(matched: usize, denominator: usize) -> usize {
    if denominator == 0 {
        SCORE_SCALE
    } else {
        matched.saturating_mul(SCORE_SCALE) / denominator
    }
}

fn render_score(score: usize) -> String {
    format!("{}.{:02}%", score / 100, score % 100)
}

fn validate_corpus(corpus: &BenchmarkCorpus) -> Result<(), DetectorBenchmarkError> {
    if corpus.schema_version != DETECTOR_BENCHMARK_CORPUS_SCHEMA_VERSION {
        return Err(benchmark_error(
            "unsupported detector benchmark corpus schema",
        ));
    }
    if corpus.id.is_empty() {
        return Err(benchmark_error("benchmark corpus id must not be empty"));
    }
    if corpus.projects.is_empty() {
        return Err(benchmark_error(
            "benchmark corpus must contain at least one project",
        ));
    }
    for (name, value) in [
        (
            "observation_precision_bps",
            corpus.minimum_scores.observation_precision_bps,
        ),
        (
            "observation_recall_bps",
            corpus.minimum_scores.observation_recall_bps,
        ),
        (
            "framework_precision_bps",
            corpus.minimum_scores.framework_precision_bps,
        ),
        (
            "framework_recall_bps",
            corpus.minimum_scores.framework_recall_bps,
        ),
    ] {
        if value > SCORE_SCALE {
            return Err(benchmark_error(format!(
                "benchmark minimum score {name} exceeds {SCORE_SCALE}"
            )));
        }
    }
    let mut project_ids = BTreeSet::new();
    for project in &corpus.projects {
        require_non_empty(&project.id, "benchmark project id")?;
        if !project_ids.insert(project.id.as_str()) {
            return Err(benchmark_error(format!(
                "duplicate benchmark project id: {}",
                project.id
            )));
        }
        require_non_empty(&project.root, "benchmark project root")?;
        validate_provenance(project)?;
        let mut manifests = BTreeSet::new();
        for manifest in &project.manifests {
            require_non_empty(manifest, "benchmark project manifest path")?;
            if !manifests.insert(manifest.as_str()) {
                return Err(benchmark_error(format!(
                    "duplicate benchmark manifest in {}: {manifest}",
                    project.id
                )));
            }
        }
        if project.cases.is_empty() {
            return Err(benchmark_error(format!(
                "benchmark project has no cases: {}",
                project.id
            )));
        }
        let mut paths = BTreeSet::new();
        for case in &project.cases {
            require_non_empty(&case.path, "benchmark case path")?;
            require_non_empty(&case.language, "benchmark case language")?;
            if !paths.insert(case.path.as_str()) {
                return Err(benchmark_error(format!(
                    "duplicate benchmark case path in {}: {}",
                    project.id, case.path
                )));
            }
            validate_records(project, case)?;
        }
    }
    Ok(())
}

fn validate_records(
    project: &BenchmarkProject,
    case: &BenchmarkCase,
) -> Result<(), DetectorBenchmarkError> {
    for record in &case.expected_observations {
        validate_kind(&record.kind, false)?;
        for (label, value) in [
            ("symbol", &record.symbol),
            ("resource", &record.resource),
            ("method", &record.method),
        ] {
            require_non_empty(
                value,
                &format!(
                    "benchmark observation {label} in {}:{}",
                    project.id, case.path
                ),
            )?;
        }
        if record.line == 0 {
            return Err(benchmark_error(format!(
                "benchmark observation line must be positive: {}:{}",
                project.id, case.path
            )));
        }
    }
    for record in &case.expected_framework_candidates {
        for (label, value) in [
            ("framework", &record.framework),
            ("symbol", &record.symbol),
            ("resource", &record.resource),
            ("method", &record.method),
        ] {
            require_non_empty(
                value,
                &format!(
                    "benchmark framework candidate {label} in {}:{}",
                    project.id, case.path
                ),
            )?;
        }
        if let Some(kind) = record.suggested_kind.as_deref() {
            validate_kind(kind, true)?;
        }
        if record.line == 0 {
            return Err(benchmark_error(format!(
                "benchmark framework candidate line must be positive: {}:{}",
                project.id, case.path
            )));
        }
    }
    Ok(())
}

fn validate_provenance(project: &BenchmarkProject) -> Result<(), DetectorBenchmarkError> {
    require_non_empty(
        &project.provenance.license,
        "benchmark project provenance license",
    )?;
    match project.provenance.kind.as_str() {
        "authored-fixture" => {
            if project.provenance.repository.is_some() || project.provenance.revision.is_some() {
                return Err(benchmark_error(format!(
                    "authored benchmark fixture must not claim an external repository or revision: {}",
                    project.id
                )));
            }
        }
        "external-snapshot" => {
            let repository = project.provenance.repository.as_deref().ok_or_else(|| {
                benchmark_error(format!(
                    "external benchmark snapshot repository is missing: {}",
                    project.id
                ))
            })?;
            require_non_empty(repository, "external benchmark repository")?;
            let revision = project.provenance.revision.as_deref().ok_or_else(|| {
                benchmark_error(format!(
                    "external benchmark snapshot revision is missing: {}",
                    project.id
                ))
            })?;
            if revision.len() != 40
                || !revision
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
            {
                return Err(benchmark_error(format!(
                    "external benchmark revision must be a lowercase 40-character Git hash: {}",
                    project.id
                )));
            }
        }
        other => {
            return Err(benchmark_error(format!(
                "unsupported benchmark provenance kind in {}: {other}",
                project.id
            )));
        }
    }
    Ok(())
}

fn digest_corpus(
    root: &Path,
    manifest: &Value,
    corpus: &BenchmarkCorpus,
) -> Result<String, DetectorBenchmarkError> {
    let mut files = Vec::new();
    for project in &corpus.projects {
        let project_root = repository_path(root, &project.root)
            .map_err(|error| benchmark_error(error.to_string()))?;
        for relative in project
            .manifests
            .iter()
            .chain(project.cases.iter().map(|case| &case.path))
        {
            let path = repository_path(&project_root, relative)
                .map_err(|error| benchmark_error(error.to_string()))?;
            let content = fs::read_to_string(&path)
                .map_err(|error| benchmark_error(format!("{}: {error}", path.display())))?;
            files.push(json!({
                "path": format!("{}/{}", project.root, relative),
                "content": content,
            }));
        }
    }
    files.sort_by(|left, right| left["path"].as_str().cmp(&right["path"].as_str()));
    canonical_digest(&json!({
        "manifest": manifest,
        "files": files,
    }))
    .map_err(|error| benchmark_error(error.to_string()))
}

fn require_non_empty(value: &str, label: &str) -> Result<(), DetectorBenchmarkError> {
    if value.is_empty() {
        Err(benchmark_error(format!("{label} must not be empty")))
    } else {
        Ok(())
    }
}

fn validate_kind(kind: &str, suggested: bool) -> Result<(), DetectorBenchmarkError> {
    let valid = if suggested {
        matches!(
            kind,
            "db_write" | "message_publish" | "external_call" | "object_write"
        )
    } else {
        matches!(kind, "db_write" | "message_publish" | "other_method_call")
    };
    if valid {
        Ok(())
    } else {
        Err(benchmark_error(format!(
            "unsupported benchmark observation kind: {kind}"
        )))
    }
}

fn kind_name(kind: SourceObservationKind) -> &'static str {
    match kind {
        SourceObservationKind::DbWrite => "db_write",
        SourceObservationKind::MessagePublish => "message_publish",
        SourceObservationKind::OtherMethodCall => "other_method_call",
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetectorBenchmarkError {
    message: String,
}

impl fmt::Display for DetectorBenchmarkError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for DetectorBenchmarkError {}

fn benchmark_error(message: impl Into<String>) -> DetectorBenchmarkError {
    DetectorBenchmarkError {
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comparison_preserves_duplicate_calls_and_reports_both_directions() {
        let expected = vec![1, 1, 2];
        let detected = vec![1, 2, 2];
        let comparison = compare(expected, detected);
        assert_eq!(comparison.matched, 2);
        assert_eq!(comparison.missing, vec![1]);
        assert_eq!(comparison.unexpected, vec![2]);
    }

    #[test]
    fn scores_use_deterministic_integer_basis_points() {
        assert_eq!(score(2, 3), 6_666);
        assert_eq!(score(0, 0), 10_000);
        assert_eq!(render_score(9_875), "98.75%");
    }
}
