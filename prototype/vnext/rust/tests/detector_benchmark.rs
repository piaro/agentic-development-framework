use agentic_vnext_rust::detector_benchmark::run_detector_benchmark;
use agentic_vnext_rust::schema::validate_json_document;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

#[test]
fn checked_in_major_framework_corpus_meets_its_reviewed_thresholds() {
    let root = corpus_root("major-frameworks-v1");
    let corpus: Value =
        serde_yaml::from_str(&fs::read_to_string(root.join("benchmark.yaml")).unwrap()).unwrap();
    validate_schema(&corpus, "benchmarks/v1/detector-corpus.schema.json");

    let report = run_detector_benchmark(&root).unwrap();
    assert!(report.passed(), "{}", report.render_text());
    assert_eq!(report.summary.projects, 10);
    assert_eq!(report.summary.cases, 20);
    assert_eq!(report.summary.observations.expected, 32);
    assert_eq!(report.summary.observations.precision_bps, 10_000);
    assert_eq!(report.summary.observations.recall_bps, 10_000);
    assert_eq!(report.summary.framework_candidates.expected, 29);
    assert_eq!(report.summary.framework_candidates.precision_bps, 10_000);
    assert_eq!(report.summary.framework_candidates.recall_bps, 10_000);
    validate_schema(
        &report.as_value(),
        "outputs/v1/detector-benchmark-report.schema.json",
    );
}

#[test]
fn checked_in_real_project_corpus_meets_its_reviewed_thresholds() {
    let root = corpus_root("real-projects-v1");
    let corpus: Value =
        serde_yaml::from_str(&fs::read_to_string(root.join("benchmark.yaml")).unwrap()).unwrap();
    validate_schema(&corpus, "benchmarks/v1/detector-corpus.schema.json");

    let report = run_detector_benchmark(&root).unwrap();
    assert!(report.passed(), "{}", report.render_text());
    assert_eq!(report.summary.projects, 4);
    assert_eq!(report.summary.cases, 4);
    assert_eq!(report.summary.parse_failures, 0);
    assert_eq!(report.summary.observations.expected, 31);
    assert_eq!(report.summary.observations.precision_bps, 10_000);
    assert_eq!(report.summary.observations.recall_bps, 10_000);
    assert_eq!(report.summary.framework_candidates.expected, 5);
    assert_eq!(report.summary.framework_candidates.precision_bps, 10_000);
    assert_eq!(report.summary.framework_candidates.recall_bps, 10_000);
    validate_schema(
        &report.as_value(),
        "outputs/v1/detector-benchmark-report.schema.json",
    );
}

#[test]
fn cli_returns_a_structured_nonzero_report_for_a_detector_regression() {
    let reviewed_digest = run_detector_benchmark(&corpus_root("major-frameworks-v1"))
        .unwrap()
        .corpus_digest;
    let temporary =
        std::env::temp_dir().join(format!("agentic-detector-benchmark-{}", std::process::id()));
    if temporary.exists() {
        fs::remove_dir_all(&temporary).unwrap();
    }
    copy_tree(&corpus_root("major-frameworks-v1"), &temporary);
    let source_path = temporary.join("projects/python-django-sqs/shop/service.py");
    let source = fs::read_to_string(&source_path).unwrap();
    fs::write(source_path, format!("{source}    audit.insert(order)\n")).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_agentic-vnext-rust"))
        .arg("benchmark")
        .arg(&temporary)
        .args(["--format", "json"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(output.stderr.is_empty());
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    validate_schema(&report, "outputs/v1/detector-benchmark-report.schema.json");
    assert_eq!(report["status"], "failed");
    assert_eq!(report["summary"]["observations"]["detected"], 33);
    assert_eq!(report["summary"]["observations"]["matched"], 32);
    assert_eq!(report["summary"]["failed_cases"], 1);
    assert_ne!(report["corpus_digest"], reviewed_digest);
    fs::remove_dir_all(temporary).unwrap();
}

fn corpus_root(corpus: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../benchmarks")
        .join(corpus)
}

fn validate_schema(value: &Value, relative: &str) {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../schemas")
        .join(relative);
    let schema: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    validate_json_document(value, &schema).unwrap();
}

fn copy_tree(source: &Path, target: &Path) {
    fs::create_dir_all(target).unwrap();
    for entry in fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let target_path = target.join(entry.file_name());
        if entry.path().is_dir() {
            copy_tree(&entry.path(), &target_path);
        } else {
            fs::copy(entry.path(), target_path).unwrap();
        }
    }
}
