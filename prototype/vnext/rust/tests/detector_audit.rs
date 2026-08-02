use agentic_vnext_rust::detector_audit::run_repository_detector_audit;
use agentic_vnext_rust::schema::validate_json_document;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

#[test]
fn repository_audit_reports_coverage_and_review_distribution() {
    let root = test_repository("complete");
    fs::write(root.join("requirements.txt"), "Django==5.2\n").unwrap();
    fs::write(
        root.join("service.py"),
        "class Service:\n    def run(self, record):\n        record.save()\n        record.update()\n",
    )
    .unwrap();
    commit_all(&root);

    let report = run_repository_detector_audit(&root, true).unwrap();
    assert!(report.complete(), "{}", report.render_text());
    assert!(report.working_tree_clean);
    assert_eq!(report.summary.tracked_source_files, 1);
    assert_eq!(report.summary.supported_source_files, 1);
    assert_eq!(report.summary.parsed_files, 1);
    assert_eq!(report.summary.manifests_discovered, 1);
    assert_eq!(report.summary.manifests_loaded, 1);
    assert_eq!(report.summary.observations.total, 2);
    assert_eq!(report.summary.observations.db_write, 1);
    assert_eq!(report.summary.observations.other_method_call, 1);
    assert_eq!(report.summary.framework_candidates.total, 2);
    assert_eq!(
        report.summary.framework_candidates.method_binding_required,
        1
    );
    assert_eq!(report.frameworks.len(), 1);
    assert_eq!(report.frameworks[0].framework, "django-orm");
    assert_eq!(report.frameworks[0].candidates, 2);
    validate_schema(&report.as_value());

    fs::write(root.join("untracked.txt"), "dirty\n").unwrap();
    let error = run_repository_detector_audit(&root, true).unwrap_err();
    assert_eq!(error.to_string(), "Git working tree is not clean");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn repository_audit_blocks_on_parse_and_language_gaps() {
    let root = test_repository("blocked");
    fs::write(root.join("broken.py"), "def broken(:\n").unwrap();
    fs::write(root.join("engine.cpp"), "void run() {}\n").unwrap();
    commit_all(&root);

    let output = Command::new(env!("CARGO_BIN_EXE_agentic-vnext-rust"))
        .args(["detector-audit", root.to_str().unwrap(), "--format", "json"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(output.stderr.is_empty());
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    validate_schema(&report);
    assert_eq!(report["status"], "blocked");
    assert_eq!(report["summary"]["tracked_source_files"], 2);
    assert_eq!(report["summary"]["unsupported_source_files"], 1);
    assert_eq!(report["summary"]["parse_failures"], 1);
    assert_eq!(report["gaps"].as_array().unwrap().len(), 2);
    fs::remove_dir_all(root).unwrap();
}

fn test_repository(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "agentic-detector-audit-{}-{name}",
        std::process::id()
    ));
    if root.exists() {
        fs::remove_dir_all(&root).unwrap();
    }
    fs::create_dir_all(&root).unwrap();
    run_git(&root, &["init", "--quiet"]);
    run_git(&root, &["config", "user.email", "audit@example.com"]);
    run_git(&root, &["config", "user.name", "Detector Audit Test"]);
    root
}

fn commit_all(root: &Path) {
    run_git(root, &["add", "-A"]);
    run_git(root, &["commit", "--quiet", "-m", "add audit fixture"]);
}

fn run_git(root: &Path, arguments: &[&str]) {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(arguments)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {:?} failed: {}",
        arguments,
        String::from_utf8_lossy(&output.stderr)
    );
}

fn validate_schema(value: &Value) {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../schemas/outputs/v1/repository-detector-audit-report.schema.json");
    let schema: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    validate_json_document(value, &schema).unwrap();
}
