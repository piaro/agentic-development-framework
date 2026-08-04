use adf::framework_lock::validate_framework_lock;
use adf::rules::{Assurance, compile_rule_index};
use adf::schema::SchemaRegistry;
use serde_json::Value;
use std::fs;
use std::path::PathBuf;

#[test]
fn standard_evidence_requirements_are_evidence_backed() {
    let fixture_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("testdata/fixtures");
    let rules = read_yaml(fixture_root.join("db-sqs/rules.yaml"));
    let schemas =
        SchemaRegistry::load(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("schemas/v1")).unwrap();
    let index = compile_rule_index(&rules, &schemas).unwrap();

    for requirement_id in [
        "data-evidence-recorded",
        "distributed-effect-evidence-recorded",
        "security-evidence-recorded",
    ] {
        assert_eq!(
            index.requirements[requirement_id].assurance,
            Assurance::EvidenceBacked,
            "{requirement_id} must not accept a shallow attestation"
        );
    }

    for rule_id in [
        "authorization-change.prepare-contracts",
        "authorization-change.record-evidence",
        "sensitive-data.prepare-contracts",
        "sensitive-data.record-evidence",
    ] {
        assert!(
            index.rules.iter().any(|rule| rule.id == rule_id),
            "standard Security Rule is missing: {rule_id}"
        );
    }
    assert_eq!(
        index.requirements["security-contracts-ready"].role,
        "Analyst"
    );
    assert_eq!(
        index.requirements["security-design-challenged"].role,
        "Challenger"
    );
    assert_eq!(
        index.requirements["security-evidence-recorded"].phase,
        "before-merge"
    );
    assert_eq!(
        index.requirements["security-implementation-challenged"].depends_on,
        ["security-evidence-recorded"]
    );
    for (rule_id, signal) in [
        (
            "authorization-change.record-evidence",
            "authorization-control-change",
        ),
        ("sensitive-data.record-evidence", "sensitive-data-access"),
    ] {
        let rule = index.rules.iter().find(|rule| rule.id == rule_id).unwrap();
        assert_eq!(rule.signal.as_deref(), Some(signal));
        assert_eq!(rule.repository_phase.as_deref(), Some("post-build"));
        assert_eq!(rule.requirement_id, "security-evidence-recorded");
    }
    for (requirement_id, requirement) in &index.requirements {
        if !requirement_id.ends_with("-evidence-recorded") {
            assert_eq!(
                requirement.assurance,
                Assurance::Attestation,
                "{requirement_id} makes a semantic judgment and must not claim evidence-backed assurance"
            );
        }
    }

    for lock_path in [
        fixture_root.join("db-sqs/framework-lock.yaml"),
        fixture_root.join("cli-project/.adf/framework.lock"),
    ] {
        validate_framework_lock(&read_yaml(lock_path), &rules, &index, &schemas).unwrap();
    }
}

fn read_yaml(path: PathBuf) -> Value {
    serde_yaml::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}
