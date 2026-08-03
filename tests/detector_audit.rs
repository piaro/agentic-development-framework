use agentic_vnext_rust::canonical_digest;
use agentic_vnext_rust::detector_audit::{DetectorAuditReport, run_repository_detector_audit};
use agentic_vnext_rust::detector_audit_baseline::check_repository_detector_audit_baseline;
use agentic_vnext_rust::schema::validate_json_document;
use serde_json::{Value, json};
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

#[test]
fn reviewed_baseline_matches_a_clean_repository_and_reports_regressions() {
    let root = test_repository("baseline-complete");
    fs::write(
        root.join("service.py"),
        "def run(order):\n    orders.insert(order)\n",
    )
    .unwrap();
    commit_all(&root);
    let audit = run_repository_detector_audit(&root, true).unwrap();
    let baseline_path = write_baseline("complete", &audit);
    validate_schema_at(
        &serde_yaml::from_str(&fs::read_to_string(&baseline_path).unwrap()).unwrap(),
        "benchmarks/v1/repository-audit-baseline.schema.json",
    );

    let report = check_repository_detector_audit_baseline(&root, &baseline_path).unwrap();
    assert!(report.matched(), "{}", report.render_text());
    assert_eq!(report.audit_status, "complete");
    validate_schema_at(
        &report.as_value(),
        "outputs/v1/repository-audit-baseline-report.schema.json",
    );

    let mut baseline: Value =
        serde_yaml::from_str(&fs::read_to_string(&baseline_path).unwrap()).unwrap();
    baseline["report_digest"] = json!(format!("sha256:{}", "0".repeat(64)));
    fs::write(&baseline_path, serde_yaml::to_string(&baseline).unwrap()).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_agentic-vnext-rust"))
        .args(["detector-audit-check", root.to_str().unwrap()])
        .arg("--baseline")
        .arg(&baseline_path)
        .args(["--format", "json"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["status"], "mismatched");
    assert_eq!(report["mismatches"][0]["field"], "report_digest");

    fs::remove_file(baseline_path).unwrap();
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn matching_baseline_never_waives_a_known_coverage_gap() {
    let root = test_repository("baseline-blocked");
    fs::write(root.join("broken.py"), "def broken(:\n").unwrap();
    commit_all(&root);
    let audit = run_repository_detector_audit(&root, true).unwrap();
    let baseline_path = write_baseline("blocked", &audit);

    let output = Command::new(env!("CARGO_BIN_EXE_agentic-vnext-rust"))
        .args(["detector-audit-check", root.to_str().unwrap()])
        .arg("--baseline")
        .arg(&baseline_path)
        .args(["--format", "json"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let baseline_report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(baseline_report["status"], "matched");
    assert_eq!(baseline_report["audit_status"], "blocked");
    assert_eq!(baseline_report["actual_gaps"].as_array().unwrap().len(), 1);

    let audit_output = Command::new(env!("CARGO_BIN_EXE_agentic-vnext-rust"))
        .args(["detector-audit", root.to_str().unwrap(), "--format", "json"])
        .output()
        .unwrap();
    assert!(!audit_output.status.success());

    fs::remove_file(baseline_path).unwrap();
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn checked_in_repository_audit_baselines_follow_the_input_schema() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../benchmarks/repository-audits-v1");
    for name in [
        "django-oscar.yaml",
        "prisma-examples.yaml",
        "nats-go.yaml",
        "godot-demo-projects.yaml",
    ] {
        let value: Value =
            serde_yaml::from_str(&fs::read_to_string(root.join(name)).unwrap()).unwrap();
        validate_schema_at(
            &value,
            "benchmarks/v1/repository-audit-baseline.schema.json",
        );
    }
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

fn write_baseline(name: &str, audit: &DetectorAuditReport) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "agentic-detector-audit-baseline-{}-{name}.yaml",
        std::process::id()
    ));
    let value = json!({
        "schema_version": "1",
        "id": format!("test-{name}"),
        "repository": "https://example.com/repository",
        "revision": audit.revision,
        "content_digest": audit.content_digest,
        "report_digest": canonical_digest(&audit.as_value()).unwrap(),
        "expected_audit_status": audit.status,
        "expected_gaps": audit.gaps,
        "review": {
            "reviewed_at": "2026-08-02",
            "basis": "Test fixture reviewed against its complete source"
        }
    });
    fs::write(&path, serde_yaml::to_string(&value).unwrap()).unwrap();
    path
}

fn validate_schema(value: &Value) {
    validate_schema_at(
        value,
        "outputs/v1/repository-detector-audit-report.schema.json",
    );
}

fn validate_schema_at(value: &Value, relative: &str) {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../schemas")
        .join(relative);
    let schema: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    validate_json_document(value, &schema).unwrap();
}
