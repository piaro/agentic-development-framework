use adf::{
    verify_application_suite, verify_canonicalization_suite, verify_context_suite,
    verify_detection_suite, verify_explain_suite, verify_filesystem_project_suite,
    verify_framework_lock_suite, verify_kernel_suite, verify_persistent_application_suite,
    verify_project_snapshot_suite, verify_result_submission_suite, verify_rule_compilation_suite,
    verify_schema_suite,
};
use std::path::PathBuf;

#[test]
fn shared_canonicalization_golden_matches() {
    let golden_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("testdata/golden/v1");
    let report = verify_canonicalization_suite(golden_root).unwrap();
    assert_eq!(report.valid_cases, 4);
    assert_eq!(report.invalid_cases, 1);
}

#[test]
fn shared_schema_validation_golden_matches() {
    let golden_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("testdata/golden/v1");
    let report = verify_schema_suite(golden_root).unwrap();
    assert_eq!(report.valid_cases, 7);
    assert_eq!(report.invalid_cases, 7);
}

#[test]
fn shared_rule_compilation_golden_matches() {
    let golden_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("testdata/golden/v1");
    let report = verify_rule_compilation_suite(golden_root).unwrap();
    assert_eq!(report.valid_variants, 2);
    assert_eq!(report.invalid_cases, 12);
}

#[test]
fn shared_typed_fact_detection_golden_matches() {
    let golden_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("testdata/golden/v1");
    let report = verify_detection_suite(golden_root).unwrap();
    assert_eq!(report.valid_variants, 4);
    assert_eq!(report.invalid_cases, 5);
}

#[test]
fn shared_thin_kernel_golden_matches() {
    let golden_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("testdata/golden/v1");
    let report = verify_kernel_suite(golden_root).unwrap();
    assert_eq!(report.cases, 12);
}

#[test]
fn shared_context_compiler_golden_matches() {
    let golden_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("testdata/golden/v1");
    let report = verify_context_suite(golden_root).unwrap();
    assert_eq!(report.cases, 7);
}

#[test]
fn shared_project_snapshot_golden_matches() {
    let golden_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("testdata/golden/v1");
    let report = verify_project_snapshot_suite(golden_root).unwrap();
    assert_eq!(report.valid_cases, 4);
    assert_eq!(report.invalid_cases, 1);
}

#[test]
fn shared_framework_lock_golden_matches() {
    let golden_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("testdata/golden/v1");
    let report = verify_framework_lock_suite(golden_root).unwrap();
    assert_eq!(report.valid_cases, 1);
    assert_eq!(report.invalid_cases, 3);
}

#[test]
fn shared_result_submission_golden_matches() {
    let golden_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("testdata/golden/v1");
    let report = verify_result_submission_suite(golden_root).unwrap();
    assert_eq!(report.valid_cases, 1);
    assert_eq!(report.invalid_cases, 13);
}

#[test]
fn shared_application_lifecycle_golden_matches() {
    let golden_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("testdata/golden/v1");
    let report = verify_application_suite(golden_root).unwrap();
    assert_eq!(report.initial_cases, 1);
    assert_eq!(report.lifecycle_steps, 12);
}

#[test]
fn shared_filesystem_project_golden_matches() {
    let golden_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("testdata/golden/v1");
    let report = verify_filesystem_project_suite(golden_root).unwrap();
    assert_eq!(report.formats, 2);
    assert_eq!(report.invalid_source_roots, 2);
}

#[test]
fn shared_persistent_application_golden_matches() {
    let golden_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("testdata/golden/v1");
    let report = verify_persistent_application_suite(golden_root).unwrap();
    assert_eq!(report.formats, 2);
    assert_eq!(report.restart_checkpoints, 20);
}

#[test]
fn shared_explain_report_golden_matches() {
    let golden_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("testdata/golden/v1");
    let report = verify_explain_suite(golden_root).unwrap();
    assert_eq!(report.checkpoints, 13);
}
