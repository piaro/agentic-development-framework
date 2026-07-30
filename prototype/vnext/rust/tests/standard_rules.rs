use agentic_vnext_rust::framework_lock::validate_framework_lock;
use agentic_vnext_rust::rules::{Assurance, compile_rule_index};
use agentic_vnext_rust::schema::SchemaRegistry;
use serde_json::Value;
use std::fs;
use std::path::PathBuf;

#[test]
fn standard_evidence_requirements_are_evidence_backed() {
    let fixture_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixtures");
    let rules = read_yaml(fixture_root.join("db-sqs/rules.yaml"));
    let schemas =
        SchemaRegistry::load(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../schemas/v1"))
            .unwrap();
    let index = compile_rule_index(&rules, &schemas).unwrap();

    for requirement_id in [
        "data-evidence-recorded",
        "distributed-effect-evidence-recorded",
    ] {
        assert_eq!(
            index.requirements[requirement_id].assurance,
            Assurance::EvidenceBacked,
            "{requirement_id} must not accept a shallow attestation"
        );
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
        fixture_root.join("cli-project/.agentic/framework.lock"),
    ] {
        validate_framework_lock(&read_yaml(lock_path), &rules, &index, &schemas).unwrap();
    }
}

fn read_yaml(path: PathBuf) -> Value {
    serde_yaml::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}
