//! Rust compatibility implementation for the vNext technical boundaries.
//!
//! Each migrated module is checked against language-neutral golden data before
//! it can be connected to the existing CLI or treated as a distribution build.

pub mod application;
pub mod binary_install;
pub mod binding_validation;
mod c_detection;
pub mod cli_output;
pub mod context;
pub mod contract_health;
pub mod contract_health_gate;
mod contract_scope;
mod csharp_detection;
pub mod delivery;
pub mod detection;
pub mod distribution_trust;
pub mod explain;
pub mod filesystem_project;
mod framework_detection;
pub mod framework_lock;
mod gdscript_detection;
pub mod git_repository;
mod go_detection;
mod java_detection;
pub mod kernel;
mod kotlin_detection;
pub mod mcp_server;
mod php_detection;
pub mod project;
pub mod project_application;
pub mod project_config;
pub mod project_runtime;
pub mod project_setup;
mod python_detection;
pub mod release_publisher;
pub mod remote_delivery;
mod ruby_detection;
pub mod rules;
mod rust_detection;
mod scala_detection;
pub mod schema;
mod script_detection;
mod signal_catalog;
mod source_detection;
pub mod submission;
mod swift_detection;

use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

pub const CANONICALIZATION_VERSION: &str = "canonical-json-v1";
pub const APPLICATION_PROTOCOL_VERSION: &str = "1";
const GOLDEN_SUITE_ID: &str = "agentic-vnext-golden-v1";

/// Serialize a JSON value using exactly the bytes hashed by the Python runtime.
pub fn canonical_json(value: &Value) -> Result<String, CanonicalError> {
    let mut output = String::new();
    write_canonical(value, &mut output)?;
    Ok(output)
}

/// Return the lowercase SHA-256 identity of canonical UTF-8 JSON bytes.
pub fn canonical_digest(value: &Value) -> Result<String, CanonicalError> {
    let canonical = canonical_json(value)?;
    let digest = Sha256::digest(canonical.as_bytes());
    Ok(format!("sha256:{digest:x}"))
}

fn write_canonical(value: &Value, output: &mut String) -> Result<(), CanonicalError> {
    match value {
        Value::Null => output.push_str("null"),
        Value::Bool(value) => output.push_str(if *value { "true" } else { "false" }),
        Value::Number(value) => {
            // Floating-point spelling differs across serializers and languages.
            // v1 accepts only values that JSON represents as signed/unsigned integers.
            if !(value.is_i64() || value.is_u64()) {
                return Err(CanonicalError::FloatingPointNotSupported);
            }
            output.push_str(&value.to_string());
        }
        Value::String(value) => {
            output.push_str(
                &serde_json::to_string(value)
                    .expect("serializing an in-memory JSON string cannot fail"),
            );
        }
        Value::Array(values) => {
            output.push('[');
            for (index, item) in values.iter().enumerate() {
                if index > 0 {
                    output.push(',');
                }
                write_canonical(item, output)?;
            }
            output.push(']');
        }
        Value::Object(values) => {
            output.push('{');
            let mut keys: Vec<&String> = values.keys().collect();
            // Rust string ordering compares UTF-8 lexicographically. UTF-8
            // preserves Unicode scalar order, matching Python's key ordering.
            keys.sort();
            for (index, key) in keys.into_iter().enumerate() {
                if index > 0 {
                    output.push(',');
                }
                output.push_str(
                    &serde_json::to_string(key)
                        .expect("serializing an in-memory JSON key cannot fail"),
                );
                output.push(':');
                write_canonical(&values[key], output)?;
            }
            output.push('}');
        }
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CanonicalError {
    FloatingPointNotSupported,
}

impl fmt::Display for CanonicalError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::FloatingPointNotSupported => {
                formatter.write_str("canonical-json-v1: floating-point-not-supported")
            }
        }
    }
}

impl std::error::Error for CanonicalError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerificationReport {
    pub valid_cases: usize,
    pub invalid_cases: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaVerificationReport {
    pub valid_cases: usize,
    pub invalid_cases: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleVerificationReport {
    pub valid_variants: usize,
    pub invalid_cases: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetectionVerificationReport {
    pub valid_variants: usize,
    pub invalid_cases: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KernelVerificationReport {
    pub cases: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextVerificationReport {
    pub cases: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectSnapshotVerificationReport {
    pub valid_cases: usize,
    pub invalid_cases: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameworkLockVerificationReport {
    pub valid_cases: usize,
    pub invalid_cases: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResultSubmissionVerificationReport {
    pub valid_cases: usize,
    pub invalid_cases: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplicationVerificationReport {
    pub initial_cases: usize,
    pub lifecycle_steps: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilesystemProjectVerificationReport {
    pub formats: usize,
    pub invalid_source_roots: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersistentApplicationVerificationReport {
    pub formats: usize,
    pub restart_checkpoints: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExplainVerificationReport {
    pub checkpoints: usize,
}

/// Read the shared manifest and verify only the implemented canonical boundary.
pub fn verify_canonicalization_suite(
    golden_root: impl AsRef<Path>,
) -> Result<VerificationReport, GoldenError> {
    let root = golden_root
        .as_ref()
        .canonicalize()
        .map_err(|error| GoldenError::Io(format!("cannot resolve golden root: {error}")))?;
    let manifest: Manifest = read_json(&root.join("manifest.json"))?;
    if manifest.suite_id != GOLDEN_SUITE_ID {
        return Err(GoldenError::Mismatch(format!(
            "suite id: expected {GOLDEN_SUITE_ID}, got {}",
            manifest.suite_id
        )));
    }
    let entry = manifest
        .cases
        .iter()
        .find(|entry| entry.kind == "canonicalization")
        .ok_or_else(|| GoldenError::Mismatch("canonicalization case is missing".to_owned()))?;
    let case_path = resolve_inside(&root, &entry.path)?;
    let case_set: CanonicalizationCaseSet = read_json(&case_path)?;
    if case_set.canonicalization_version != CANONICALIZATION_VERSION {
        return Err(GoldenError::Mismatch(format!(
            "canonicalization version: expected {CANONICALIZATION_VERSION}, got {}",
            case_set.canonicalization_version
        )));
    }
    if case_set.number_scope != "integers-only" {
        return Err(GoldenError::Mismatch(format!(
            "number scope: expected integers-only, got {}",
            case_set.number_scope
        )));
    }

    for case in &case_set.cases {
        let actual_json = canonical_json(&case.value)
            .map_err(|error| GoldenError::Mismatch(format!("{}: {error}", case.case_id)))?;
        if actual_json != case.canonical_json {
            return Err(GoldenError::Mismatch(format!(
                "{} canonical JSON: expected {:?}, got {:?}",
                case.case_id, case.canonical_json, actual_json
            )));
        }
        let actual_digest = canonical_digest(&case.value)
            .map_err(|error| GoldenError::Mismatch(format!("{}: {error}", case.case_id)))?;
        if actual_digest != case.digest {
            return Err(GoldenError::Mismatch(format!(
                "{} digest: expected {}, got {}",
                case.case_id, case.digest, actual_digest
            )));
        }
    }

    for case in &case_set.invalid_cases {
        match canonical_json(&case.value) {
            Err(error) if error.to_string().contains(&case.error) => {}
            Err(error) => {
                return Err(GoldenError::Mismatch(format!(
                    "{}: expected error {}, got {}",
                    case.case_id, case.error, error
                )));
            }
            Ok(value) => {
                return Err(GoldenError::Mismatch(format!(
                    "{}: expected failure, got {value}",
                    case.case_id
                )));
            }
        }
    }

    Ok(VerificationReport {
        valid_cases: case_set.cases.len(),
        invalid_cases: case_set.invalid_cases.len(),
    })
}

/// Verify the shared Record and Result payload Schema boundary.
pub fn verify_schema_suite(
    golden_root: impl AsRef<Path>,
) -> Result<SchemaVerificationReport, GoldenError> {
    let root = golden_root
        .as_ref()
        .canonicalize()
        .map_err(|error| GoldenError::Io(format!("cannot resolve golden root: {error}")))?;
    let manifest: Manifest = read_json(&root.join("manifest.json"))?;
    if manifest.suite_id != GOLDEN_SUITE_ID {
        return Err(GoldenError::Mismatch(format!(
            "suite id: expected {GOLDEN_SUITE_ID}, got {}",
            manifest.suite_id
        )));
    }
    let entry = manifest
        .cases
        .iter()
        .find(|entry| entry.kind == "schema-validation")
        .ok_or_else(|| GoldenError::Mismatch("schema-validation case is missing".to_owned()))?;
    let case_path = resolve_inside(&root, &entry.path)?;
    let case_set: SchemaCaseSet = read_json(&case_path)?;
    let workspace_root = root
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| GoldenError::InvalidPath("golden root has no workspace".to_owned()))?
        .canonicalize()
        .map_err(|error| GoldenError::Io(error.to_string()))?;
    let schema_root = resolve_relative_within(&root, &case_set.schema_root, &workspace_root)?;
    let registry = schema::SchemaRegistry::load(schema_root)
        .map_err(|error| GoldenError::Mismatch(error.to_string()))?;
    if case_set.schema_bundle_version != schema::SCHEMA_BUNDLE_VERSION {
        return Err(GoldenError::Mismatch(format!(
            "Schema bundle version: expected {}, got {}",
            schema::SCHEMA_BUNDLE_VERSION,
            case_set.schema_bundle_version
        )));
    }
    if registry.digest() != case_set.schema_bundle_digest {
        return Err(GoldenError::Mismatch(format!(
            "Schema bundle digest: expected {}, got {}",
            case_set.schema_bundle_digest,
            registry.digest()
        )));
    }

    let mut valid_cases = 0;
    let mut invalid_cases = 0;
    for case in &case_set.cases {
        match registry.validate(&case.record_kind, &case.record) {
            Ok(()) if case.valid => valid_cases += 1,
            Ok(()) => {
                return Err(GoldenError::Mismatch(format!(
                    "{}: expected invalid, got valid",
                    case.case_id
                )));
            }
            Err(error) if case.valid => {
                return Err(GoldenError::Mismatch(format!(
                    "{}: expected valid, got {error}",
                    case.case_id
                )));
            }
            Err(error) => {
                invalid_cases += 1;
                if case.error_path.as_deref() != Some(error.path.as_str()) {
                    return Err(GoldenError::Mismatch(format!(
                        "{}: expected error at {:?}, got {}",
                        case.case_id, case.error_path, error.path
                    )));
                }
            }
        }
    }
    Ok(SchemaVerificationReport {
        valid_cases,
        invalid_cases,
    })
}

/// Verify normalized Rule Index output and deterministic configuration errors.
pub fn verify_rule_compilation_suite(
    golden_root: impl AsRef<Path>,
) -> Result<RuleVerificationReport, GoldenError> {
    let root = golden_root
        .as_ref()
        .canonicalize()
        .map_err(|error| GoldenError::Io(format!("cannot resolve golden root: {error}")))?;
    let manifest: Manifest = read_json(&root.join("manifest.json"))?;
    if manifest.suite_id != GOLDEN_SUITE_ID {
        return Err(GoldenError::Mismatch(format!(
            "suite id: expected {GOLDEN_SUITE_ID}, got {}",
            manifest.suite_id
        )));
    }
    let entry = manifest
        .cases
        .iter()
        .find(|entry| entry.kind == "rule-compilation")
        .ok_or_else(|| GoldenError::Mismatch("rule-compilation case is missing".to_owned()))?;
    let case_path = resolve_inside(&root, &entry.path)?;
    let case_set: RuleCompilationCaseSet = read_json(&case_path)?;
    if case_set.compiler_version != rules::RULE_COMPILER_VERSION {
        return Err(GoldenError::Mismatch(format!(
            "Rule Compiler version: expected {}, got {}",
            rules::RULE_COMPILER_VERSION,
            case_set.compiler_version
        )));
    }

    let workspace_root = root
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| GoldenError::InvalidPath("golden root has no workspace".to_owned()))?
        .canonicalize()
        .map_err(|error| GoldenError::Io(error.to_string()))?;
    let schema_root = resolve_relative_within(&root, &case_set.schema_root, &workspace_root)?;
    let registry = schema::SchemaRegistry::load(schema_root)
        .map_err(|error| GoldenError::Mismatch(error.to_string()))?;
    if registry.digest() != case_set.schema_bundle_digest {
        return Err(GoldenError::Mismatch(format!(
            "Rule Compiler Schema bundle digest: expected {}, got {}",
            case_set.schema_bundle_digest,
            registry.digest()
        )));
    }

    let mut valid_variants = 0;
    for case in &case_set.cases {
        for (variant_index, source) in case.source_variants.iter().enumerate() {
            let index = rules::compile_rule_index(source, &registry).map_err(|error| {
                GoldenError::Mismatch(format!(
                    "{} variant {variant_index}: expected valid, got {error}",
                    case.case_id
                ))
            })?;
            let actual = index.compatibility_value();
            if actual != case.expected {
                return Err(GoldenError::Mismatch(format!(
                    "{} variant {variant_index}: expected {}, got {}",
                    case.case_id, case.expected, actual
                )));
            }
            valid_variants += 1;
        }
    }

    for case in &case_set.invalid_cases {
        match rules::compile_rule_index(&case.source, &registry) {
            Err(error) if error.to_string() == case.error => {}
            Err(error) => {
                return Err(GoldenError::Mismatch(format!(
                    "{}: expected error {:?}, got {:?}",
                    case.case_id,
                    case.error,
                    error.to_string()
                )));
            }
            Ok(index) => {
                return Err(GoldenError::Mismatch(format!(
                    "{}: expected failure, got {}",
                    case.case_id,
                    index.compatibility_value()
                )));
            }
        }
    }

    Ok(RuleVerificationReport {
        valid_variants,
        invalid_cases: case_set.invalid_cases.len(),
    })
}

/// Verify typed fact conversion, candidate fingerprints, and report digest.
pub fn verify_detection_suite(
    golden_root: impl AsRef<Path>,
) -> Result<DetectionVerificationReport, GoldenError> {
    let root = golden_root
        .as_ref()
        .canonicalize()
        .map_err(|error| GoldenError::Io(format!("cannot resolve golden root: {error}")))?;
    let manifest: Manifest = read_json(&root.join("manifest.json"))?;
    if manifest.suite_id != GOLDEN_SUITE_ID {
        return Err(GoldenError::Mismatch(format!(
            "suite id: expected {GOLDEN_SUITE_ID}, got {}",
            manifest.suite_id
        )));
    }
    let entry = manifest
        .cases
        .iter()
        .find(|entry| entry.kind == "typed-fact-detection")
        .ok_or_else(|| GoldenError::Mismatch("typed-fact-detection case is missing".to_owned()))?;
    let case_path = resolve_inside(&root, &entry.path)?;
    let case_set: DetectionCaseSet = read_json(&case_path)?;
    if case_set.detector_id != detection::DETECTOR_ID {
        return Err(GoldenError::Mismatch(format!(
            "Detector ID: expected {}, got {}",
            detection::DETECTOR_ID,
            case_set.detector_id
        )));
    }
    if case_set.detector_version != detection::DETECTOR_VERSION {
        return Err(GoldenError::Mismatch(format!(
            "Detector version: expected {}, got {}",
            detection::DETECTOR_VERSION,
            case_set.detector_version
        )));
    }

    let mut valid_variants = 0;
    for case in &case_set.cases {
        for (variant_index, input) in case.input_variants.iter().enumerate() {
            let report = detection::detect_typed_facts(
                &input.change_id,
                &input.facts,
                &input.coverage,
                &input.artifact_digests,
            )
            .map_err(|error| {
                GoldenError::Mismatch(format!(
                    "{} variant {variant_index}: expected valid, got {error}",
                    case.case_id
                ))
            })?;
            let actual = report.compatibility_value();
            if actual != case.expected {
                return Err(GoldenError::Mismatch(format!(
                    "{} variant {variant_index}: expected {}, got {}",
                    case.case_id, case.expected, actual
                )));
            }
            valid_variants += 1;
        }
    }
    for case in &case_set.invalid_cases {
        match detection::detect_typed_facts(
            &case.input.change_id,
            &case.input.facts,
            &case.input.coverage,
            &case.input.artifact_digests,
        ) {
            Err(error) if error.to_string() == case.error => {}
            Err(error) => {
                return Err(GoldenError::Mismatch(format!(
                    "{}: expected error {:?}, got {:?}",
                    case.case_id,
                    case.error,
                    error.to_string()
                )));
            }
            Ok(report) => {
                return Err(GoldenError::Mismatch(format!(
                    "{}: expected failure, got {}",
                    case.case_id,
                    report.compatibility_value()
                )));
            }
        }
    }
    Ok(DetectionVerificationReport {
        valid_variants,
        invalid_cases: case_set.invalid_cases.len(),
    })
}

/// Verify all pure Thin Kernel state transitions in the shared fixture.
pub fn verify_kernel_suite(
    golden_root: impl AsRef<Path>,
) -> Result<KernelVerificationReport, GoldenError> {
    let root = golden_root
        .as_ref()
        .canonicalize()
        .map_err(|error| GoldenError::Io(format!("cannot resolve golden root: {error}")))?;
    let manifest: Manifest = read_json(&root.join("manifest.json"))?;
    if manifest.suite_id != GOLDEN_SUITE_ID {
        return Err(GoldenError::Mismatch(format!(
            "suite id: expected {GOLDEN_SUITE_ID}, got {}",
            manifest.suite_id
        )));
    }
    let entry = manifest
        .cases
        .iter()
        .find(|entry| entry.kind == "kernel-decision")
        .ok_or_else(|| GoldenError::Mismatch("kernel-decision case is missing".to_owned()))?;
    let case_path = resolve_inside(&root, &entry.path)?;
    let case_set: KernelCaseSet = read_json(&case_path)?;
    if case_set.kernel_version != kernel::KERNEL_VERSION {
        return Err(GoldenError::Mismatch(format!(
            "Kernel version: expected {}, got {}",
            kernel::KERNEL_VERSION,
            case_set.kernel_version
        )));
    }

    let workspace_root = root
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| GoldenError::InvalidPath("golden root has no workspace".to_owned()))?
        .canonicalize()
        .map_err(|error| GoldenError::Io(error.to_string()))?;
    let schema_root = resolve_relative_within(&root, &case_set.schema_root, &workspace_root)?;
    let registry = schema::SchemaRegistry::load(schema_root)
        .map_err(|error| GoldenError::Mismatch(error.to_string()))?;
    let rule_index = rules::compile_rule_index(&case_set.common.rule_source, &registry)
        .map_err(|error| GoldenError::Mismatch(error.to_string()))?;
    let thin_kernel = kernel::ThinKernel;

    for case in &case_set.cases {
        let results = case
            .input
            .result_refs
            .iter()
            .map(|record_id| {
                case_set
                    .common
                    .result_records
                    .get(record_id)
                    .cloned()
                    .ok_or_else(|| {
                        GoldenError::Mismatch(format!(
                            "{}: unknown Result fixture {record_id:?}",
                            case.case_id
                        ))
                    })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let snapshot = kernel::ProjectSnapshot {
            change_id: case_set.common.change_id.clone(),
            change: Value::Object(Default::default()),
            contracts: case.input.contracts.clone(),
            decisions: case.input.decisions.clone(),
            results,
            evidence: Vec::new(),
            repository: json!({
                "phase": case.input.repository_phase,
                "artifacts": [],
                "facts": case_set.common.facts,
                "coverage": case
                    .input
                    .coverage
                    .as_ref()
                    .unwrap_or(&case_set.common.coverage),
            }),
            artifact_digests: case_set.common.artifact_digests.clone(),
            digest: String::new(),
        };
        let detection = detection::detect_typed_facts(
            &snapshot.change_id,
            &case_set.common.facts,
            &snapshot.repository["coverage"],
            &snapshot.artifact_digests,
        )
        .map_err(|error| {
            GoldenError::Mismatch(format!("{}: Detector failed: {error}", case.case_id))
        })?;
        let decision = thin_kernel.evaluate(&snapshot, &rule_index, &detection);
        let actual = decision.compatibility_checkpoint();
        if actual != case.expected {
            return Err(GoldenError::Mismatch(format!(
                "{}: expected {}, got {}",
                case.case_id, case.expected, actual
            )));
        }
    }
    Ok(KernelVerificationReport {
        cases: case_set.cases.len(),
    })
}

/// Verify selector resolution, source manifests, and Generated Context digest.
pub fn verify_context_suite(
    golden_root: impl AsRef<Path>,
) -> Result<ContextVerificationReport, GoldenError> {
    let root = golden_root
        .as_ref()
        .canonicalize()
        .map_err(|error| GoldenError::Io(format!("cannot resolve golden root: {error}")))?;
    let manifest: Manifest = read_json(&root.join("manifest.json"))?;
    if manifest.suite_id != GOLDEN_SUITE_ID {
        return Err(GoldenError::Mismatch(format!(
            "suite id: expected {GOLDEN_SUITE_ID}, got {}",
            manifest.suite_id
        )));
    }
    let entry = manifest
        .cases
        .iter()
        .find(|entry| entry.kind == "context-compilation")
        .ok_or_else(|| GoldenError::Mismatch("context-compilation case is missing".to_owned()))?;
    let case_path = resolve_inside(&root, &entry.path)?;
    let case_set: ContextCaseSet = read_json(&case_path)?;
    if case_set.context_compiler_version != context::CONTEXT_COMPILER_VERSION {
        return Err(GoldenError::Mismatch(format!(
            "Context Compiler version: expected {}, got {}",
            context::CONTEXT_COMPILER_VERSION,
            case_set.context_compiler_version
        )));
    }
    let snapshot_source = &case_set.common.snapshot;
    let snapshot = kernel::ProjectSnapshot {
        change_id: snapshot_source.change_id.clone(),
        change: snapshot_source.change.clone(),
        contracts: snapshot_source.contracts.clone(),
        decisions: snapshot_source.decisions.clone(),
        results: snapshot_source.results.clone(),
        evidence: snapshot_source.evidence.clone(),
        repository: snapshot_source.repository.clone(),
        artifact_digests: snapshot_source.artifact_digests.clone(),
        digest: String::new(),
    };
    let detection = detection::detect_typed_facts(
        &snapshot.change_id,
        snapshot.repository["facts"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or(&[]),
        &snapshot.repository["coverage"],
        &snapshot.artifact_digests,
    )
    .map_err(|error| GoldenError::Mismatch(format!("Context Detector failed: {error}")))?;
    let compiler = context::ContextCompiler;

    for case in &case_set.cases {
        let all_instances = resolve_context_instances(
            &case_set.common.requirement_instances,
            &case.all_instance_refs,
            &case.case_id,
        )?;
        let action = case
            .action
            .as_ref()
            .map(|source| {
                Ok(kernel::NextAction {
                    id: source.id.clone(),
                    role: source.role.clone(),
                    action: source.action.clone(),
                    requirement_instances: resolve_context_instances(
                        &case_set.common.requirement_instances,
                        &source.instance_refs,
                        &case.case_id,
                    )?,
                    reason: source.reason.clone(),
                    expected_result_schema: source.expected_result_schema.clone(),
                    candidate_fingerprints: source.candidate_fingerprints.clone(),
                })
            })
            .transpose()?;
        let decision = kernel::KernelDecision {
            state: case.state.clone(),
            action,
            requirement_instances: all_instances,
            diagnostics: Vec::new(),
        };
        let actual = compiler
            .compile(&decision, &snapshot, &detection)
            .map(|generated| generated.compatibility_checkpoint())
            .unwrap_or(Value::Null);
        if actual != case.expected {
            return Err(GoldenError::Mismatch(format!(
                "{}: expected {}, got {}",
                case.case_id, case.expected, actual
            )));
        }
    }
    Ok(ContextVerificationReport {
        cases: case_set.cases.len(),
    })
}

/// Verify record normalization and Project Snapshot content identities.
pub fn verify_project_snapshot_suite(
    golden_root: impl AsRef<Path>,
) -> Result<ProjectSnapshotVerificationReport, GoldenError> {
    let root = golden_root
        .as_ref()
        .canonicalize()
        .map_err(|error| GoldenError::Io(format!("cannot resolve golden root: {error}")))?;
    let manifest: Manifest = read_json(&root.join("manifest.json"))?;
    if manifest.suite_id != GOLDEN_SUITE_ID {
        return Err(GoldenError::Mismatch(format!(
            "suite id: expected {GOLDEN_SUITE_ID}, got {}",
            manifest.suite_id
        )));
    }
    let entry = manifest
        .cases
        .iter()
        .find(|entry| entry.kind == "project-snapshot")
        .ok_or_else(|| GoldenError::Mismatch("project-snapshot case is missing".to_owned()))?;
    let case_path = resolve_inside(&root, &entry.path)?;
    let case_set: ProjectSnapshotCaseSet = read_json(&case_path)?;
    if case_set.snapshot_protocol_version != project::PROJECT_SNAPSHOT_PROTOCOL_VERSION {
        return Err(GoldenError::Mismatch(format!(
            "Project Snapshot protocol version: expected {}, got {}",
            project::PROJECT_SNAPSHOT_PROTOCOL_VERSION,
            case_set.snapshot_protocol_version
        )));
    }

    let workspace_root = root
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| GoldenError::InvalidPath("golden root has no workspace".to_owned()))?
        .canonicalize()
        .map_err(|error| GoldenError::Io(error.to_string()))?;
    let schema_root = resolve_relative_within(&root, &case_set.schema_root, &workspace_root)?;
    let registry = schema::SchemaRegistry::load(schema_root)
        .map_err(|error| GoldenError::Mismatch(error.to_string()))?;
    let base_path = resolve_inside(&root, &case_set.base_case)?;
    let base: Value = read_json(&base_path)?;
    let base_project = base["input"]["project"].clone();

    for case in &case_set.cases {
        let mut project_source = base_project.clone();
        let project_object = project_source.as_object_mut().ok_or_else(|| {
            GoldenError::Mismatch("base Project fixture must be an object".to_owned())
        })?;
        if case.reverse_record_collections {
            for collection in ["changes", "contracts", "decisions", "results", "evidence"] {
                if let Some(records) = project_object
                    .get_mut(collection)
                    .and_then(Value::as_array_mut)
                {
                    records.reverse();
                }
            }
        }
        if case.append_unrelated_records {
            for (collection, additions) in &case_set.unrelated_records {
                let records = project_object
                    .entry(collection)
                    .or_insert_with(|| Value::Array(Vec::new()))
                    .as_array_mut()
                    .ok_or_else(|| {
                        GoldenError::Mismatch(format!(
                            "Project collection {collection:?} must be an array"
                        ))
                    })?;
                records.extend(additions.iter().cloned());
            }
        }
        if case.append_multi_change_records {
            for (collection, additions) in &case_set.multi_change_records {
                let records = project_object
                    .entry(collection)
                    .or_insert_with(|| Value::Array(Vec::new()))
                    .as_array_mut()
                    .ok_or_else(|| {
                        GoldenError::Mismatch(format!(
                            "Project collection {collection:?} must be an array"
                        ))
                    })?;
                records.extend(additions.iter().cloned());
            }
        }
        let snapshot = project::build_project_snapshot(&project_source, &case.change_id, &registry)
            .map_err(|error| {
                GoldenError::Mismatch(format!("{}: expected valid, got {error}", case.case_id))
            })?;
        let actual = project::compatibility_checkpoint(&snapshot);
        let expected = case.expected.as_ref().unwrap_or(&case_set.expected);
        if actual != *expected {
            return Err(GoldenError::Mismatch(format!(
                "{}: expected {}, got {}",
                case.case_id, expected, actual
            )));
        }
    }

    for case in &case_set.invalid_cases {
        match project::build_project_snapshot(&base_project, &case.change_id, &registry) {
            Err(error) if error.to_string() == case.error => {}
            Err(error) => {
                return Err(GoldenError::Mismatch(format!(
                    "{}: expected error {:?}, got {:?}",
                    case.case_id,
                    case.error,
                    error.to_string()
                )));
            }
            Ok(snapshot) => {
                return Err(GoldenError::Mismatch(format!(
                    "{}: expected failure, got {}",
                    case.case_id,
                    project::compatibility_checkpoint(&snapshot)
                )));
            }
        }
    }
    Ok(ProjectSnapshotVerificationReport {
        valid_cases: case_set.cases.len(),
        invalid_cases: case_set.invalid_cases.len(),
    })
}

/// Verify Framework lock generation, digesting, and strict compatibility checks.
pub fn verify_framework_lock_suite(
    golden_root: impl AsRef<Path>,
) -> Result<FrameworkLockVerificationReport, GoldenError> {
    let root = golden_root
        .as_ref()
        .canonicalize()
        .map_err(|error| GoldenError::Io(format!("cannot resolve golden root: {error}")))?;
    let manifest: Manifest = read_json(&root.join("manifest.json"))?;
    if manifest.suite_id != GOLDEN_SUITE_ID {
        return Err(GoldenError::Mismatch(format!(
            "suite id: expected {GOLDEN_SUITE_ID}, got {}",
            manifest.suite_id
        )));
    }
    let entry = manifest
        .cases
        .iter()
        .find(|entry| entry.kind == "framework-lock")
        .ok_or_else(|| GoldenError::Mismatch("framework-lock case is missing".to_owned()))?;
    let case_path = resolve_inside(&root, &entry.path)?;
    let case_set: FrameworkLockCaseSet = read_json(&case_path)?;
    if case_set.framework_lock_schema_version != framework_lock::FRAMEWORK_LOCK_SCHEMA_VERSION {
        return Err(GoldenError::Mismatch(format!(
            "Framework lock Schema version: expected {}, got {}",
            framework_lock::FRAMEWORK_LOCK_SCHEMA_VERSION,
            case_set.framework_lock_schema_version
        )));
    }

    let workspace_root = root
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| GoldenError::InvalidPath("golden root has no workspace".to_owned()))?
        .canonicalize()
        .map_err(|error| GoldenError::Io(error.to_string()))?;
    let schema_root = resolve_relative_within(&root, &case_set.schema_root, &workspace_root)?;
    let registry = schema::SchemaRegistry::load(schema_root)
        .map_err(|error| GoldenError::Mismatch(error.to_string()))?;
    let base_path = resolve_inside(&root, &case_set.base_case)?;
    let base: Value = read_json(&base_path)?;
    let rule_source = base["input"]["rule_source"].clone();
    let reviewed_lock = base["input"]["framework_lock"].clone();
    let rule_index = rules::compile_rule_index(&rule_source, &registry)
        .map_err(|error| GoldenError::Mismatch(format!("Rule compilation failed: {error}")))?;

    let generated_lock = framework_lock::build_framework_lock(&rule_source, &rule_index, &registry)
        .map_err(|error| GoldenError::Mismatch(error.to_string()))?;
    if generated_lock != reviewed_lock {
        return Err(GoldenError::Mismatch(format!(
            "generated Framework lock: expected {reviewed_lock}, got {generated_lock}"
        )));
    }
    let validated = framework_lock::validate_framework_lock(
        &reviewed_lock,
        &rule_source,
        &rule_index,
        &registry,
    )
    .map_err(|error| GoldenError::Mismatch(error.to_string()))?;
    if validated.digest != case_set.expected_digest {
        return Err(GoldenError::Mismatch(format!(
            "Framework lock digest: expected {}, got {}",
            case_set.expected_digest, validated.digest
        )));
    }

    for case in &case_set.invalid_cases {
        let mut invalid_lock = reviewed_lock.clone();
        mutate_json_path(
            &mut invalid_lock,
            &case.operation,
            &case.path,
            case.value.clone(),
        )
        .map_err(|message| GoldenError::Mismatch(format!("{}: {message}", case.case_id)))?;
        match framework_lock::validate_framework_lock(
            &invalid_lock,
            &rule_source,
            &rule_index,
            &registry,
        ) {
            Err(error) if error.to_string().contains(&case.error_path) => {}
            Err(error) => {
                return Err(GoldenError::Mismatch(format!(
                    "{}: expected error containing {:?}, got {:?}",
                    case.case_id,
                    case.error_path,
                    error.to_string()
                )));
            }
            Ok(lock) => {
                return Err(GoldenError::Mismatch(format!(
                    "{}: expected failure, got {}",
                    case.case_id, lock.digest
                )));
            }
        }
    }

    Ok(FrameworkLockVerificationReport {
        valid_cases: 1,
        invalid_cases: case_set.invalid_cases.len(),
    })
}

/// Verify Result construction independently from storage and reevaluation.
pub fn verify_result_submission_suite(
    golden_root: impl AsRef<Path>,
) -> Result<ResultSubmissionVerificationReport, GoldenError> {
    let root = golden_root
        .as_ref()
        .canonicalize()
        .map_err(|error| GoldenError::Io(format!("cannot resolve golden root: {error}")))?;
    let manifest: Manifest = read_json(&root.join("manifest.json"))?;
    if manifest.suite_id != GOLDEN_SUITE_ID {
        return Err(GoldenError::Mismatch(format!(
            "suite id: expected {GOLDEN_SUITE_ID}, got {}",
            manifest.suite_id
        )));
    }
    let entry = manifest
        .cases
        .iter()
        .find(|entry| entry.kind == "result-submission")
        .ok_or_else(|| GoldenError::Mismatch("result-submission case is missing".to_owned()))?;
    let case_path = resolve_inside(&root, &entry.path)?;
    let case_set: ResultSubmissionCaseSet = read_json(&case_path)?;
    if case_set.result_submission_protocol_version != submission::RESULT_SUBMISSION_PROTOCOL_VERSION
    {
        return Err(GoldenError::Mismatch(format!(
            "Result submission protocol version: expected {}, got {}",
            submission::RESULT_SUBMISSION_PROTOCOL_VERSION,
            case_set.result_submission_protocol_version
        )));
    }

    let workspace_root = root
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| GoldenError::InvalidPath("golden root has no workspace".to_owned()))?
        .canonicalize()
        .map_err(|error| GoldenError::Io(error.to_string()))?;
    let schema_root = resolve_relative_within(&root, &case_set.schema_root, &workspace_root)?;
    let registry = schema::SchemaRegistry::load(schema_root)
        .map_err(|error| GoldenError::Mismatch(error.to_string()))?;
    let base_path = resolve_inside(&root, &case_set.base_case)?;
    let base: Value = read_json(&base_path)?;
    let scenario_path = resolve_inside(&root, &case_set.scenario_case)?;
    let scenario: Value = read_json(&scenario_path)?;
    let step = scenario["steps"]
        .as_array()
        .and_then(|steps| steps.get(case_set.step_index))
        .ok_or_else(|| GoldenError::Mismatch("submission scenario step is missing".to_owned()))?;
    let project = base["input"]["project"].clone();
    let change_id = base["change_id"]
        .as_str()
        .ok_or_else(|| GoldenError::Mismatch("base Change ID is missing".to_owned()))?;
    let snapshot = project::build_project_snapshot(&project, change_id, &registry)
        .map_err(|error| GoldenError::Mismatch(error.to_string()))?;
    let rule_index = rules::compile_rule_index(&base["input"]["rule_source"], &registry)
        .map_err(|error| GoldenError::Mismatch(error.to_string()))?;
    let detection = detection::detect_typed_facts(
        change_id,
        snapshot.repository["facts"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or(&[]),
        &snapshot.repository["coverage"],
        &snapshot.artifact_digests,
    )
    .map_err(|error| GoldenError::Mismatch(error.to_string()))?;
    let decision = kernel::ThinKernel.evaluate(&snapshot, &rule_index, &detection);
    let action = decision
        .action
        .as_ref()
        .ok_or_else(|| GoldenError::Mismatch("submission base produced no Action".to_owned()))?;
    let context = context::ContextCompiler
        .compile(&decision, &snapshot, &detection)
        .ok_or_else(|| GoldenError::Mismatch("submission base produced no Context".to_owned()))?;
    let submission = submission::ResultSubmission {
        change_id: change_id.to_owned(),
        action_id: action.id.clone(),
        context_digest: context.digest.clone(),
        role: action.role.clone(),
        result_schema: action.expected_result_schema.clone(),
        payload: step["input"]["payload"].clone(),
        output_refs: step["input"]["output_refs"]
            .as_array()
            .map(|values| {
                values
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default(),
    };
    let actual = submission::prepare_result(&context, &snapshot, &submission, &registry)
        .map_err(|error| GoldenError::Mismatch(error.to_string()))?;
    if actual != case_set.expected {
        return Err(GoldenError::Mismatch(format!(
            "prepared Result Record: expected {}, got {}",
            case_set.expected, actual
        )));
    }

    for case in &case_set.invalid_cases {
        let mut mutated = json!({
            "project": project,
            "submission": submission,
        });
        mutate_mixed_json_path(
            &mut mutated,
            &case.operation,
            &case.path,
            case.value.clone(),
        )
        .map_err(|message| GoldenError::Mismatch(format!("{}: {message}", case.case_id)))?;
        let current = project::build_project_snapshot(&mutated["project"], change_id, &registry)
            .map_err(|error| GoldenError::Mismatch(error.to_string()))?;
        let submitted: submission::ResultSubmission =
            serde_json::from_value(mutated["submission"].clone()).map_err(|error| {
                GoldenError::Mismatch(format!("{}: invalid submission: {error}", case.case_id))
            })?;
        match submission::prepare_result(&context, &current, &submitted, &registry) {
            Err(error) if error.to_string().contains(&case.error) => {}
            Err(error) => {
                return Err(GoldenError::Mismatch(format!(
                    "{}: expected error containing {:?}, got {:?}",
                    case.case_id,
                    case.error,
                    error.to_string()
                )));
            }
            Ok(result) => {
                return Err(GoldenError::Mismatch(format!(
                    "{}: expected failure, got {result}",
                    case.case_id
                )));
            }
        }
    }

    Ok(ResultSubmissionVerificationReport {
        valid_cases: 1,
        invalid_cases: case_set.invalid_cases.len(),
    })
}

/// Replay the complete `next` / `submit` module order against shared fixtures.
pub fn verify_application_suite(
    golden_root: impl AsRef<Path>,
) -> Result<ApplicationVerificationReport, GoldenError> {
    let root = golden_root
        .as_ref()
        .canonicalize()
        .map_err(|error| GoldenError::Io(format!("cannot resolve golden root: {error}")))?;
    let manifest: Manifest = read_json(&root.join("manifest.json"))?;
    if manifest.suite_id != GOLDEN_SUITE_ID {
        return Err(GoldenError::Mismatch(format!(
            "suite id: expected {GOLDEN_SUITE_ID}, got {}",
            manifest.suite_id
        )));
    }
    let initial_entry = manifest
        .cases
        .iter()
        .find(|entry| entry.kind == "application")
        .ok_or_else(|| GoldenError::Mismatch("application case is missing".to_owned()))?;
    let scenario_entry = manifest
        .cases
        .iter()
        .find(|entry| entry.kind == "application-scenario")
        .ok_or_else(|| GoldenError::Mismatch("application-scenario case is missing".to_owned()))?;
    let initial_path = resolve_inside(&root, &initial_entry.path)?;
    let initial: Value = read_json(&initial_path)?;
    let scenario_path = resolve_inside(&root, &scenario_entry.path)?;
    let scenario: Value = read_json(&scenario_path)?;

    let workspace_root = root
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| GoldenError::InvalidPath("golden root has no workspace".to_owned()))?
        .canonicalize()
        .map_err(|error| GoldenError::Io(error.to_string()))?;
    let schema_root = resolve_relative_within(
        &root,
        initial["schema_root"].as_str().ok_or_else(|| {
            GoldenError::Mismatch("application Schema root is missing".to_owned())
        })?,
        &workspace_root,
    )?;
    let registry = schema::SchemaRegistry::load(schema_root)
        .map_err(|error| GoldenError::Mismatch(error.to_string()))?;
    verify_initial_application_case(&initial, &registry)?;

    let base_path = resolve_inside(
        &root,
        scenario["base_case"]
            .as_str()
            .ok_or_else(|| GoldenError::Mismatch("scenario base case is missing".to_owned()))?,
    )?;
    let base: Value = read_json(&base_path)?;
    let change_id = base["change_id"]
        .as_str()
        .ok_or_else(|| GoldenError::Mismatch("base Change ID is missing".to_owned()))?;
    let mut application = application::InMemoryApplication::new(
        base["input"]["project"].clone(),
        &base["input"]["rule_source"],
        &base["input"]["framework_lock"],
        &registry,
    )
    .map_err(|error| GoldenError::Mismatch(error.to_string()))?;
    let mut response = application
        .next(change_id)
        .map_err(|error| GoldenError::Mismatch(error.to_string()))?;
    let steps = scenario["steps"]
        .as_array()
        .ok_or_else(|| GoldenError::Mismatch("scenario steps must be an array".to_owned()))?;
    for (index, step) in steps.iter().enumerate() {
        let operation = step["operation"].as_str().ok_or_else(|| {
            GoldenError::Mismatch(format!("scenario step {index} operation is missing"))
        })?;
        match operation {
            "submit-current" => {
                let action = response.decision.action.as_ref().ok_or_else(|| {
                    GoldenError::Mismatch(format!("scenario step {index}: no current Action"))
                })?;
                let context = response.context.as_ref().ok_or_else(|| {
                    GoldenError::Mismatch(format!("scenario step {index}: no current Context"))
                })?;
                let submission = submission::ResultSubmission {
                    change_id: change_id.to_owned(),
                    action_id: action.id.clone(),
                    context_digest: context.digest.clone(),
                    role: action.role.clone(),
                    result_schema: action.expected_result_schema.clone(),
                    payload: step["input"]["payload"].clone(),
                    output_refs: string_array(&step["input"]["output_refs"]),
                };
                response = application.submit(&submission).map_err(|error| {
                    GoldenError::Mismatch(format!("scenario step {index} submit failed: {error}"))
                })?;
            }
            "upsert-decision" => application
                .upsert_decision(step["input"].clone(), None)
                .map_err(|error| GoldenError::Mismatch(error.to_string()))?,
            "upsert-contract" => application
                .upsert_contract(step["input"].clone(), step["expected_digest"].as_str())
                .map_err(|error| GoldenError::Mismatch(error.to_string()))?,
            "update-repository" => {
                application
                    .update_repository(step["input"].clone())
                    .map_err(|error| GoldenError::Mismatch(error.to_string()))?;
                response = application
                    .next(change_id)
                    .map_err(|error| GoldenError::Mismatch(error.to_string()))?;
            }
            "complete-build" => {
                response =
                    complete_current_build(&mut application, &response, change_id, &step["input"])?;
            }
            other => {
                return Err(GoldenError::Mismatch(format!(
                    "scenario step {index}: unsupported operation {other:?}"
                )));
            }
        }
        if let Some(expected) = step.get("expected") {
            let actual = application_checkpoint(&application, change_id, &response)?;
            if &actual != expected {
                return Err(GoldenError::Mismatch(format!(
                    "scenario step {index}: expected {expected}, got {actual}"
                )));
            }
        }
    }

    Ok(ApplicationVerificationReport {
        initial_cases: 1,
        lifecycle_steps: steps.len(),
    })
}

/// Verify persistent Record layout, exclusive append, and restart recovery.
pub fn verify_filesystem_project_suite(
    golden_root: impl AsRef<Path>,
) -> Result<FilesystemProjectVerificationReport, GoldenError> {
    let root = golden_root
        .as_ref()
        .canonicalize()
        .map_err(|error| GoldenError::Io(format!("cannot resolve golden root: {error}")))?;
    let manifest: Manifest = read_json(&root.join("manifest.json"))?;
    if manifest.suite_id != GOLDEN_SUITE_ID {
        return Err(GoldenError::Mismatch(format!(
            "suite id: expected {GOLDEN_SUITE_ID}, got {}",
            manifest.suite_id
        )));
    }
    let entry = manifest
        .cases
        .iter()
        .find(|entry| entry.kind == "filesystem-project")
        .ok_or_else(|| GoldenError::Mismatch("filesystem-project case is missing".to_owned()))?;
    let case_path = resolve_inside(&root, &entry.path)?;
    let case_set: FilesystemProjectCaseSet = read_json(&case_path)?;
    if case_set.filesystem_project_protocol_version
        != filesystem_project::FILESYSTEM_PROJECT_PROTOCOL_VERSION
    {
        return Err(GoldenError::Mismatch(format!(
            "Filesystem Project protocol version: expected {}, got {}",
            filesystem_project::FILESYSTEM_PROJECT_PROTOCOL_VERSION,
            case_set.filesystem_project_protocol_version
        )));
    }

    let workspace_root = root
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| GoldenError::InvalidPath("golden root has no workspace".to_owned()))?
        .canonicalize()
        .map_err(|error| GoldenError::Io(error.to_string()))?;
    let schema_root = resolve_relative_within(&root, &case_set.schema_root, &workspace_root)?;
    let registry = schema::SchemaRegistry::load(schema_root)
        .map_err(|error| GoldenError::Mismatch(error.to_string()))?;
    let base: Value = read_json(&resolve_inside(&root, &case_set.base_case)?)?;
    let result_case: Value = read_json(&resolve_inside(&root, &case_set.result_case)?)?;
    let scenario: Value = read_json(&resolve_inside(&root, &case_set.scenario_case)?)?;
    let mut project = base["input"]["project"].clone();
    project["changes"]
        .as_array_mut()
        .ok_or_else(|| GoldenError::Mismatch("Project Changes must be an array".to_owned()))?
        .push(
            case_set
                .shared_contract_concurrency
                .clause_updates
                .second_change
                .clone(),
        );
    let repository = project["repository"].clone();
    let change_id = base["change_id"]
        .as_str()
        .ok_or_else(|| GoldenError::Mismatch("base Change ID is missing".to_owned()))?;
    let result = &result_case["expected"];
    let decision = &scenario["steps"][case_set.decision_step]["input"];
    let contract = &scenario["steps"][case_set.contract_step]["input"];

    for format_case in &case_set.formats {
        let temporary = GoldenTempDir::new(&format_case.document_format)?;
        let format = match format_case.document_format.as_str() {
            "yaml" => filesystem_project::DocumentFormat::Yaml,
            "markdown" => filesystem_project::DocumentFormat::Markdown,
            other => {
                return Err(GoldenError::Mismatch(format!(
                    "unsupported document format {other:?}"
                )));
            }
        };
        let mut store = filesystem_project::FileProjectStore::initialize(
            temporary.path(),
            &project,
            format,
            &registry,
        )
        .map_err(|error| GoldenError::Mismatch(error.to_string()))?;
        verify_store_snapshot(
            &store,
            change_id,
            &case_set.expected_initial_snapshot_digest,
            &format!("{} initial", format_case.document_format),
        )?;
        verify_record_paths(
            &store,
            temporary.path(),
            change_id,
            &format_case.initial_paths,
            &format!("{} initial", format_case.document_format),
        )?;

        match filesystem_project::FileProjectStore::initialize(
            temporary.path(),
            &project,
            format,
            &registry,
        ) {
            Err(error) if error.to_string().contains("would overwrite") => {}
            Err(error) => {
                return Err(GoldenError::Mismatch(format!(
                    "{}: unexpected initialization error: {error}",
                    format_case.document_format
                )));
            }
            Ok(_) => {
                return Err(GoldenError::Mismatch(format!(
                    "{}: expected initialization conflict",
                    format_case.document_format
                )));
            }
        }
        verify_store_snapshot(
            &store,
            change_id,
            &case_set.expected_initial_snapshot_digest,
            &format!(
                "{} after initialization conflict",
                format_case.document_format
            ),
        )?;

        let prose_path = temporary.path().join(&format_case.contract_path);
        if format == filesystem_project::DocumentFormat::Markdown {
            let text = fs::read_to_string(&prose_path)
                .map_err(|error| GoldenError::Io(error.to_string()))?;
            let updated = text.replacen(
                "```agentic-contract\n",
                &format!("{}\n\n```agentic-contract\n", case_set.markdown_prose),
                1,
            );
            fs::write(&prose_path, updated).map_err(|error| GoldenError::Io(error.to_string()))?;
        }

        store
            .append_result(result)
            .map_err(|error| GoldenError::Mismatch(error.to_string()))?;
        verify_store_snapshot(
            &store,
            change_id,
            &case_set.expected_result_snapshot_digest,
            &format!("{} Result", format_case.document_format),
        )?;
        let mut concurrent = filesystem_project::FileProjectStore::open(
            temporary.path(),
            repository.clone(),
            &registry,
        )
        .map_err(|error| GoldenError::Mismatch(error.to_string()))?;
        match concurrent.append_result(result) {
            Err(error) if error.to_string().contains("record already exists") => {}
            Err(error) => {
                return Err(GoldenError::Mismatch(format!(
                    "{}: unexpected duplicate Result error: {error}",
                    format_case.document_format
                )));
            }
            Ok(()) => {
                return Err(GoldenError::Mismatch(format!(
                    "{}: duplicate Result was accepted",
                    format_case.document_format
                )));
            }
        }

        store
            .upsert_decision(decision, None)
            .and_then(|()| store.upsert_contract(contract, None))
            .map_err(|error| GoldenError::Mismatch(error.to_string()))?;
        verify_store_snapshot(
            &store,
            change_id,
            &case_set.expected_updated_snapshot_digest,
            &format!("{} updated", format_case.document_format),
        )?;
        verify_record_paths(
            &store,
            temporary.path(),
            change_id,
            &format_case.updated_paths,
            &format!("{} updated", format_case.document_format),
        )?;
        if format == filesystem_project::DocumentFormat::Markdown {
            let text = fs::read_to_string(&prose_path)
                .map_err(|error| GoldenError::Io(error.to_string()))?;
            if !text.contains(&case_set.markdown_prose) {
                return Err(GoldenError::Mismatch(
                    "Markdown prose was removed by Contract update".to_owned(),
                ));
            }
        }
        let temporary_files = files_with_extension(temporary.path(), "tmp")?;
        if !temporary_files.is_empty() {
            return Err(GoldenError::Mismatch(format!(
                "atomic update left temporary files: {}",
                temporary_files
                    .iter()
                    .map(|path| path.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            )));
        }
        let restarted = filesystem_project::FileProjectStore::open(
            temporary.path(),
            repository.clone(),
            &registry,
        )
        .map_err(|error| GoldenError::Mismatch(error.to_string()))?;
        verify_store_snapshot(
            &restarted,
            change_id,
            &case_set.expected_updated_snapshot_digest,
            &format!("{} restarted", format_case.document_format),
        )?;

        let concurrency = &case_set.shared_contract_concurrency;
        store
            .upsert_contract(&concurrency.initial, None)
            .map_err(|error| GoldenError::Mismatch(error.to_string()))?;
        let initial_digest = canonical_digest(&concurrency.initial)
            .map_err(|error| GoldenError::Mismatch(error.to_string()))?;
        let mut first_writer = filesystem_project::FileProjectStore::open(
            temporary.path(),
            repository.clone(),
            &registry,
        )
        .map_err(|error| GoldenError::Mismatch(error.to_string()))?;
        let mut stale_writer = filesystem_project::FileProjectStore::open(
            temporary.path(),
            repository.clone(),
            &registry,
        )
        .map_err(|error| GoldenError::Mismatch(error.to_string()))?;
        first_writer
            .upsert_contract(&concurrency.first_update, Some(&initial_digest))
            .map_err(|error| GoldenError::Mismatch(error.to_string()))?;
        match stale_writer.upsert_contract(&concurrency.stale_update, None) {
            Err(error)
                if error
                    .to_string()
                    .contains(&concurrency.missing_digest_error) => {}
            Err(error) => {
                return Err(GoldenError::Mismatch(format!(
                    "{}: unexpected missing digest error: {error}",
                    format_case.document_format
                )));
            }
            Ok(()) => {
                return Err(GoldenError::Mismatch(format!(
                    "{}: Shared Contract update without digest was accepted",
                    format_case.document_format
                )));
            }
        }
        match stale_writer.upsert_contract(&concurrency.stale_update, Some(&initial_digest)) {
            Err(error) if error.to_string().contains(&concurrency.stale_error) => {}
            Err(error) => {
                return Err(GoldenError::Mismatch(format!(
                    "{}: unexpected stale update error: {error}",
                    format_case.document_format
                )));
            }
            Ok(()) => {
                return Err(GoldenError::Mismatch(format!(
                    "{}: stale Shared Contract update was accepted",
                    format_case.document_format
                )));
            }
        }
        let current = store
            .snapshot(change_id)
            .map_err(|error| GoldenError::Mismatch(error.to_string()))?;
        let current_shared = current
            .contracts
            .iter()
            .find(|contract| contract["id"] == concurrency.initial["id"])
            .ok_or_else(|| {
                GoldenError::Mismatch("Shared Contract disappeared after stale update".to_owned())
            })?;
        if current_shared["clauses"][0]["text"].as_str() != Some(&concurrency.expected_text) {
            return Err(GoldenError::Mismatch(format!(
                "{}: stale update changed Shared Contract: {}",
                format_case.document_format, current_shared
            )));
        }

        let clause_updates = &concurrency.clause_updates;
        store
            .upsert_contract(&clause_updates.initial, None)
            .map_err(|error| GoldenError::Mismatch(error.to_string()))?;
        let second_change_id = clause_updates.second_change["id"]
            .as_str()
            .ok_or_else(|| GoldenError::Mismatch("second Change ID is missing".to_owned()))?;
        for visible_change_id in [change_id, second_change_id] {
            let snapshot = store
                .snapshot(visible_change_id)
                .map_err(|error| GoldenError::Mismatch(error.to_string()))?;
            if !snapshot
                .contracts
                .iter()
                .any(|contract| contract["id"] == clause_updates.initial["id"])
            {
                return Err(GoldenError::Mismatch(format!(
                    "{}: Shared Contract is not visible to {visible_change_id}",
                    format_case.document_format
                )));
            }
        }
        let clause_digests = clause_updates.initial["clauses"]
            .as_array()
            .ok_or_else(|| {
                GoldenError::Mismatch("clause update fixture clauses must be an array".to_owned())
            })?
            .iter()
            .map(|clause| {
                let clause_id = clause["id"].as_str().ok_or_else(|| {
                    GoldenError::Mismatch("clause update fixture ID is missing".to_owned())
                })?;
                let digest = canonical_digest(clause)
                    .map_err(|error| GoldenError::Mismatch(error.to_string()))?;
                Ok((clause_id.to_owned(), digest))
            })
            .collect::<Result<BTreeMap<_, _>, GoldenError>>()?;
        let first_expected = BTreeMap::from([(
            clause_updates.first_clause_id.clone(),
            clause_digests
                .get(&clause_updates.first_clause_id)
                .ok_or_else(|| GoldenError::Mismatch("first clause digest is missing".to_owned()))?
                .clone(),
        )]);
        let second_expected = BTreeMap::from([(
            clause_updates.second_clause_id.clone(),
            clause_digests
                .get(&clause_updates.second_clause_id)
                .ok_or_else(|| GoldenError::Mismatch("second clause digest is missing".to_owned()))?
                .clone(),
        )]);
        let mut first_clause_writer = filesystem_project::FileProjectStore::open(
            temporary.path(),
            repository.clone(),
            &registry,
        )
        .map_err(|error| GoldenError::Mismatch(error.to_string()))?;
        let mut second_clause_writer = filesystem_project::FileProjectStore::open(
            temporary.path(),
            repository.clone(),
            &registry,
        )
        .map_err(|error| GoldenError::Mismatch(error.to_string()))?;
        first_clause_writer
            .upsert_contract_clauses(&clause_updates.first_update, &first_expected)
            .map_err(|error| GoldenError::Mismatch(error.to_string()))?;
        second_clause_writer
            .upsert_contract_clauses(&clause_updates.non_overlapping_update, &second_expected)
            .map_err(|error| GoldenError::Mismatch(error.to_string()))?;
        match second_clause_writer
            .upsert_contract_clauses(&clause_updates.conflicting_update, &first_expected)
        {
            Err(error) if error.to_string().contains(&clause_updates.stale_error) => {}
            Err(error) => {
                return Err(GoldenError::Mismatch(format!(
                    "{}: unexpected same-clause conflict error: {error}",
                    format_case.document_format
                )));
            }
            Ok(()) => {
                return Err(GoldenError::Mismatch(format!(
                    "{}: stale same-clause update was accepted",
                    format_case.document_format
                )));
            }
        }
        let snapshot = store
            .snapshot(change_id)
            .map_err(|error| GoldenError::Mismatch(error.to_string()))?;
        let merged_contract = snapshot
            .contracts
            .iter()
            .find(|contract| contract["id"] == clause_updates.initial["id"])
            .ok_or_else(|| {
                GoldenError::Mismatch("clause-merged Shared Contract disappeared".to_owned())
            })?;
        let merged_texts = merged_contract["clauses"]
            .as_array()
            .ok_or_else(|| GoldenError::Mismatch("merged clauses must be an array".to_owned()))?
            .iter()
            .filter_map(|clause| {
                Some((
                    clause["id"].as_str()?.to_owned(),
                    clause["text"].as_str()?.to_owned(),
                ))
            })
            .collect::<BTreeMap<_, _>>();
        if merged_texts
            .get(&clause_updates.first_clause_id)
            .map(String::as_str)
            != Some(&clause_updates.expected_first_text)
            || merged_texts
                .get(&clause_updates.second_clause_id)
                .map(String::as_str)
                != Some(&clause_updates.expected_second_text)
        {
            return Err(GoldenError::Mismatch(format!(
                "{}: non-overlapping clause updates were not preserved: {merged_contract}",
                format_case.document_format
            )));
        }
    }

    let temporary = GoldenTempDir::new("invalid-source-roots")?;
    for invalid in &case_set.invalid_source_roots {
        match filesystem_project::FileProjectStore::open_with_options(
            temporary.path(),
            repository.clone(),
            &invalid.contract_root,
            filesystem_project::DEFAULT_DECISION_ROOT,
            filesystem_project::DocumentFormat::Auto,
            &registry,
        ) {
            Err(error) if error.to_string().contains(&invalid.error) => {}
            Err(error) => {
                return Err(GoldenError::Mismatch(format!(
                    "{}: expected error containing {:?}, got {error}",
                    invalid.case_id, invalid.error
                )));
            }
            Ok(_) => {
                return Err(GoldenError::Mismatch(format!(
                    "{}: unsafe source root was accepted",
                    invalid.case_id
                )));
            }
        }
    }

    Ok(FilesystemProjectVerificationReport {
        formats: case_set.formats.len(),
        invalid_source_roots: case_set.invalid_source_roots.len(),
    })
}

/// Replay the full lifecycle through the filesystem Store and process restarts.
pub fn verify_persistent_application_suite(
    golden_root: impl AsRef<Path>,
) -> Result<PersistentApplicationVerificationReport, GoldenError> {
    let root = golden_root
        .as_ref()
        .canonicalize()
        .map_err(|error| GoldenError::Io(format!("cannot resolve golden root: {error}")))?;
    let manifest: Manifest = read_json(&root.join("manifest.json"))?;
    if manifest.suite_id != GOLDEN_SUITE_ID {
        return Err(GoldenError::Mismatch(format!(
            "suite id: expected {GOLDEN_SUITE_ID}, got {}",
            manifest.suite_id
        )));
    }
    let entry = manifest
        .cases
        .iter()
        .find(|entry| entry.kind == "persistent-application")
        .ok_or_else(|| {
            GoldenError::Mismatch("persistent-application case is missing".to_owned())
        })?;
    let case_path = resolve_inside(&root, &entry.path)?;
    let case_set: PersistentApplicationCaseSet = read_json(&case_path)?;
    if case_set.persistent_application_protocol_version != APPLICATION_PROTOCOL_VERSION {
        return Err(GoldenError::Mismatch(format!(
            "Persistent Application protocol version: expected {APPLICATION_PROTOCOL_VERSION}, got {}",
            case_set.persistent_application_protocol_version
        )));
    }

    let workspace_root = root
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| GoldenError::InvalidPath("golden root has no workspace".to_owned()))?
        .canonicalize()
        .map_err(|error| GoldenError::Io(error.to_string()))?;
    let schema_root = resolve_relative_within(&root, &case_set.schema_root, &workspace_root)?;
    let registry = schema::SchemaRegistry::load(schema_root)
        .map_err(|error| GoldenError::Mismatch(error.to_string()))?;
    let scenario: Value = read_json(&resolve_inside(&root, &case_set.scenario_case)?)?;
    let base: Value = read_json(&resolve_inside(
        &root,
        scenario["base_case"]
            .as_str()
            .ok_or_else(|| GoldenError::Mismatch("scenario base case is missing".to_owned()))?,
    )?)?;
    let project = &base["input"]["project"];
    let rule_source = &base["input"]["rule_source"];
    let framework_lock = &base["input"]["framework_lock"];
    let change_id = base["change_id"]
        .as_str()
        .ok_or_else(|| GoldenError::Mismatch("base Change ID is missing".to_owned()))?;
    let steps = scenario["steps"]
        .as_array()
        .ok_or_else(|| GoldenError::Mismatch("scenario steps must be an array".to_owned()))?;
    let mut restart_checkpoints = 0;

    for document_format in &case_set.document_formats {
        let temporary = GoldenTempDir::new(&format!("persistent-{document_format}"))?;
        let format = match document_format.as_str() {
            "yaml" => filesystem_project::DocumentFormat::Yaml,
            "markdown" => filesystem_project::DocumentFormat::Markdown,
            other => {
                return Err(GoldenError::Mismatch(format!(
                    "unsupported persistent document format {other:?}"
                )));
            }
        };
        let mut current_repository = project["repository"].clone();
        let store = filesystem_project::FileProjectStore::initialize(
            temporary.path(),
            project,
            format,
            &registry,
        )
        .map_err(|error| GoldenError::Mismatch(error.to_string()))?;
        let mut application =
            application::Application::with_store(store, rule_source, framework_lock, &registry)
                .map_err(|error| GoldenError::Mismatch(error.to_string()))?;
        let mut response = application
            .next(change_id)
            .map_err(|error| GoldenError::Mismatch(error.to_string()))?;
        let mut format_checkpoints = 0;

        for (index, step) in steps.iter().enumerate() {
            let operation = step["operation"].as_str().ok_or_else(|| {
                GoldenError::Mismatch(format!("persistent step {index} operation is missing"))
            })?;
            match operation {
                "submit-current" => {
                    let action = response.decision.action.as_ref().ok_or_else(|| {
                        GoldenError::Mismatch(format!(
                            "{document_format} step {index}: no current Action"
                        ))
                    })?;
                    let context = response.context.as_ref().ok_or_else(|| {
                        GoldenError::Mismatch(format!(
                            "{document_format} step {index}: no current Context"
                        ))
                    })?;
                    let submission = submission::ResultSubmission {
                        change_id: change_id.to_owned(),
                        action_id: action.id.clone(),
                        context_digest: context.digest.clone(),
                        role: action.role.clone(),
                        result_schema: action.expected_result_schema.clone(),
                        payload: step["input"]["payload"].clone(),
                        output_refs: string_array(&step["input"]["output_refs"]),
                    };
                    response = application.submit(&submission).map_err(|error| {
                        GoldenError::Mismatch(format!(
                            "{document_format} step {index} submit failed: {error}"
                        ))
                    })?;
                }
                "upsert-decision" => application
                    .upsert_decision(step["input"].clone(), None)
                    .map_err(|error| GoldenError::Mismatch(error.to_string()))?,
                "upsert-contract" => application
                    .upsert_contract(step["input"].clone(), step["expected_digest"].as_str())
                    .map_err(|error| GoldenError::Mismatch(error.to_string()))?,
                "update-repository" => {
                    current_repository = step["input"].clone();
                    application
                        .update_repository(current_repository.clone())
                        .map_err(|error| GoldenError::Mismatch(error.to_string()))?;
                    response = application
                        .next(change_id)
                        .map_err(|error| GoldenError::Mismatch(error.to_string()))?;
                }
                "complete-build" => {
                    current_repository = step["input"]["repository"].clone();
                    response = complete_current_build(
                        &mut application,
                        &response,
                        change_id,
                        &step["input"],
                    )?;
                }
                other => {
                    return Err(GoldenError::Mismatch(format!(
                        "{document_format} step {index}: unsupported operation {other:?}"
                    )));
                }
            }
            let Some(expected) = step.get("expected") else {
                continue;
            };
            format_checkpoints += 1;
            let actual = application_checkpoint(&application, change_id, &response)?;
            if &actual != expected {
                return Err(GoldenError::Mismatch(format!(
                    "{document_format} persistent step {index}: expected {expected}, got {actual}"
                )));
            }

            // Issued Context is intentionally not persisted. Reopening the Store
            // and calling `next` must reproduce it from authoritative Records.
            let store = filesystem_project::FileProjectStore::open(
                temporary.path(),
                current_repository.clone(),
                &registry,
            )
            .map_err(|error| GoldenError::Mismatch(error.to_string()))?;
            application =
                application::Application::with_store(store, rule_source, framework_lock, &registry)
                    .map_err(|error| GoldenError::Mismatch(error.to_string()))?;
            response = application
                .next(change_id)
                .map_err(|error| GoldenError::Mismatch(error.to_string()))?;
            let restarted = application_checkpoint(&application, change_id, &response)?;
            if &restarted != expected {
                return Err(GoldenError::Mismatch(format!(
                    "{document_format} restarted step {index}: expected {expected}, got {restarted}"
                )));
            }
            restart_checkpoints += 1;
        }
        if format_checkpoints != case_set.expected_checkpoints {
            return Err(GoldenError::Mismatch(format!(
                "{document_format} checkpoint count: expected {}, got {format_checkpoints}",
                case_set.expected_checkpoints
            )));
        }
        let result_count = application
            .snapshot(change_id)
            .map_err(|error| GoldenError::Mismatch(error.to_string()))?
            .results
            .len();
        if result_count != case_set.expected_result_records {
            return Err(GoldenError::Mismatch(format!(
                "{document_format} Result count: expected {}, got {result_count}",
                case_set.expected_result_records
            )));
        }
    }

    Ok(PersistentApplicationVerificationReport {
        formats: case_set.document_formats.len(),
        restart_checkpoints,
    })
}

/// Verify machine-readable and compact text explanations across one lifecycle.
pub fn verify_explain_suite(
    golden_root: impl AsRef<Path>,
) -> Result<ExplainVerificationReport, GoldenError> {
    let root = golden_root
        .as_ref()
        .canonicalize()
        .map_err(|error| GoldenError::Io(format!("cannot resolve golden root: {error}")))?;
    let manifest: Manifest = read_json(&root.join("manifest.json"))?;
    if manifest.suite_id != GOLDEN_SUITE_ID {
        return Err(GoldenError::Mismatch(format!(
            "suite id: expected {GOLDEN_SUITE_ID}, got {}",
            manifest.suite_id
        )));
    }
    let entry = manifest
        .cases
        .iter()
        .find(|entry| entry.kind == "explain-report")
        .ok_or_else(|| GoldenError::Mismatch("explain-report case is missing".to_owned()))?;
    let case_set: ExplainCaseSet = read_json(&resolve_inside(&root, &entry.path)?)?;
    if case_set.schema_version != "1" {
        return Err(GoldenError::Mismatch(format!(
            "Explain golden Schema version: expected 1, got {}",
            case_set.schema_version
        )));
    }
    if case_set.explain_report_schema_version != explain::EXPLAIN_REPORT_SCHEMA_VERSION {
        return Err(GoldenError::Mismatch(format!(
            "Explain Report Schema version: expected {}, got {}",
            explain::EXPLAIN_REPORT_SCHEMA_VERSION,
            case_set.explain_report_schema_version
        )));
    }

    let vnext_root = root
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| GoldenError::InvalidPath("golden root has no vNext root".to_owned()))?
        .canonicalize()
        .map_err(|error| GoldenError::Io(error.to_string()))?;
    let report_schema: Value = read_json(&resolve_relative_within(
        &root,
        &case_set.schema_path,
        &vnext_root,
    )?)?;
    let base: Value = read_json(&resolve_inside(&root, &case_set.base_case)?)?;
    let scenario: Value = read_json(&resolve_inside(&root, &case_set.scenario_case)?)?;
    let schema_root = resolve_relative_within(
        &root,
        base["schema_root"]
            .as_str()
            .ok_or_else(|| GoldenError::Mismatch("Explain Schema root is missing".to_owned()))?,
        &vnext_root,
    )?;
    let registry = schema::SchemaRegistry::load(schema_root)
        .map_err(|error| GoldenError::Mismatch(error.to_string()))?;
    let change_id = base["change_id"]
        .as_str()
        .ok_or_else(|| GoldenError::Mismatch("Explain Change ID is missing".to_owned()))?;
    let mut application = application::InMemoryApplication::new(
        base["input"]["project"].clone(),
        &base["input"]["rule_source"],
        &base["input"]["framework_lock"],
        &registry,
    )
    .map_err(|error| GoldenError::Mismatch(error.to_string()))?;
    let mut response = application
        .next(change_id)
        .map_err(|error| GoldenError::Mismatch(error.to_string()))?;
    let mut checkpoints = std::collections::BTreeMap::new();
    for checkpoint in &case_set.checkpoints {
        if checkpoints
            .insert(checkpoint.after_step, checkpoint)
            .is_some()
        {
            return Err(GoldenError::Mismatch(
                "Explain Report has duplicate checkpoints".to_owned(),
            ));
        }
    }
    verify_explain_checkpoint(
        &application,
        change_id,
        checkpoints.get(&None).copied().ok_or_else(|| {
            GoldenError::Mismatch("initial Explain checkpoint is missing".to_owned())
        })?,
        &report_schema,
    )?;

    let steps = scenario["steps"].as_array().ok_or_else(|| {
        GoldenError::Mismatch("Explain scenario steps must be an array".to_owned())
    })?;
    for (index, step) in steps.iter().enumerate() {
        let operation = step["operation"].as_str().ok_or_else(|| {
            GoldenError::Mismatch(format!("Explain step {index} operation is missing"))
        })?;
        match operation {
            "submit-current" => {
                let action = response.decision.action.as_ref().ok_or_else(|| {
                    GoldenError::Mismatch(format!("Explain step {index}: no current Action"))
                })?;
                let context = response.context.as_ref().ok_or_else(|| {
                    GoldenError::Mismatch(format!("Explain step {index}: no current Context"))
                })?;
                let submission = submission::ResultSubmission {
                    change_id: change_id.to_owned(),
                    action_id: action.id.clone(),
                    context_digest: context.digest.clone(),
                    role: action.role.clone(),
                    result_schema: action.expected_result_schema.clone(),
                    payload: step["input"]["payload"].clone(),
                    output_refs: string_array(&step["input"]["output_refs"]),
                };
                response = application.submit(&submission).map_err(|error| {
                    GoldenError::Mismatch(format!("Explain step {index} submit failed: {error}"))
                })?;
            }
            "upsert-decision" => application
                .upsert_decision(step["input"].clone(), None)
                .map_err(|error| GoldenError::Mismatch(error.to_string()))?,
            "upsert-contract" => application
                .upsert_contract(step["input"].clone(), step["expected_digest"].as_str())
                .map_err(|error| GoldenError::Mismatch(error.to_string()))?,
            "update-repository" => {
                application
                    .update_repository(step["input"].clone())
                    .map_err(|error| GoldenError::Mismatch(error.to_string()))?;
                response = application
                    .next(change_id)
                    .map_err(|error| GoldenError::Mismatch(error.to_string()))?;
            }
            "complete-build" => {
                response =
                    complete_current_build(&mut application, &response, change_id, &step["input"])?;
            }
            other => {
                return Err(GoldenError::Mismatch(format!(
                    "Explain step {index}: unsupported operation {other:?}"
                )));
            }
        }
        let expected = checkpoints.get(&Some(index)).copied().ok_or_else(|| {
            GoldenError::Mismatch(format!("Explain checkpoint is missing: {index}"))
        })?;
        verify_explain_checkpoint(&application, change_id, expected, &report_schema)?;
    }

    Ok(ExplainVerificationReport {
        checkpoints: case_set.checkpoints.len(),
    })
}

fn verify_explain_checkpoint<Store: application::ProjectStore>(
    application: &application::Application<'_, Store>,
    change_id: &str,
    expected: &ExplainCheckpoint,
    report_schema: &Value,
) -> Result<(), GoldenError> {
    let before_digest = application
        .snapshot(change_id)
        .map_err(|error| GoldenError::Mismatch(error.to_string()))?
        .digest;
    let report = application
        .explain(change_id)
        .map_err(|error| GoldenError::Mismatch(error.to_string()))?;
    let body = report.as_value();
    schema::validate_json_document(&body, report_schema)
        .map_err(|error| GoldenError::Mismatch(error.to_string()))?;
    let after_digest = application
        .snapshot(change_id)
        .map_err(|error| GoldenError::Mismatch(error.to_string()))?
        .digest;
    if before_digest != after_digest {
        return Err(GoldenError::Mismatch(format!(
            "{}: Explain changed Snapshot from {before_digest} to {after_digest}",
            expected.label
        )));
    }
    let report_digest =
        canonical_digest(&body).map_err(|error| GoldenError::Mismatch(error.to_string()))?;
    let text_digest = canonical_digest(&Value::String(report.render_text()))
        .map_err(|error| GoldenError::Mismatch(error.to_string()))?;
    let actual = ExplainCheckpoint {
        label: expected.label.clone(),
        after_step: expected.after_step,
        state: report.state,
        candidate_count: report.candidates.len(),
        requirement_count: report.requirements.len(),
        authority_statuses: report
            .authority
            .iter()
            .map(|authority| authority.status.clone())
            .collect(),
        report_digest,
        text_digest,
    };
    if &actual != expected {
        return Err(GoldenError::Mismatch(format!(
            "{}: expected {expected:?}, got {actual:?}",
            expected.label
        )));
    }
    Ok(())
}

fn verify_store_snapshot(
    store: &filesystem_project::FileProjectStore<'_>,
    change_id: &str,
    expected_digest: &str,
    label: &str,
) -> Result<(), GoldenError> {
    let snapshot = store
        .snapshot(change_id)
        .map_err(|error| GoldenError::Mismatch(error.to_string()))?;
    if snapshot.digest != expected_digest {
        return Err(GoldenError::Mismatch(format!(
            "{label} Snapshot digest: expected {expected_digest}, got {}",
            snapshot.digest
        )));
    }
    Ok(())
}

fn verify_record_paths(
    store: &filesystem_project::FileProjectStore<'_>,
    root: &Path,
    change_id: &str,
    expected: &[String],
    label: &str,
) -> Result<(), GoldenError> {
    let actual = store
        .record_paths(change_id)
        .map_err(|error| GoldenError::Mismatch(error.to_string()))?
        .iter()
        .map(|path| relative_path_string(root, path))
        .collect::<Result<Vec<_>, _>>()?;
    if actual != expected {
        return Err(GoldenError::Mismatch(format!(
            "{label} paths: expected {expected:?}, got {actual:?}"
        )));
    }
    Ok(())
}

fn relative_path_string(root: &Path, path: &Path) -> Result<String, GoldenError> {
    let relative = path
        .strip_prefix(root)
        .map_err(|error| GoldenError::InvalidPath(error.to_string()))?;
    Ok(relative
        .components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/"))
}

fn files_with_extension(root: &Path, extension: &str) -> Result<Vec<PathBuf>, GoldenError> {
    let mut output = Vec::new();
    if !root.is_dir() {
        return Ok(output);
    }
    for entry in fs::read_dir(root).map_err(|error| GoldenError::Io(error.to_string()))? {
        let path = entry
            .map_err(|error| GoldenError::Io(error.to_string()))?
            .path();
        if path.is_dir() {
            output.extend(files_with_extension(&path, extension)?);
        } else if path.extension().and_then(|value| value.to_str()) == Some(extension) {
            output.push(path);
        }
    }
    output.sort();
    Ok(output)
}

struct GoldenTempDir {
    path: PathBuf,
}

impl GoldenTempDir {
    fn new(label: &str) -> Result<Self, GoldenError> {
        use std::sync::atomic::{AtomicU64, Ordering};
        static SEQUENCE: AtomicU64 = AtomicU64::new(0);
        let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "agentic-vnext-{label}-{}-{sequence}",
            std::process::id()
        ));
        if path.exists() {
            fs::remove_dir_all(&path).map_err(|error| GoldenError::Io(error.to_string()))?;
        }
        fs::create_dir_all(&path).map_err(|error| GoldenError::Io(error.to_string()))?;
        let path = path
            .canonicalize()
            .map_err(|error| GoldenError::Io(error.to_string()))?;
        Ok(Self { path })
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for GoldenTempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn verify_initial_application_case(
    case: &Value,
    schema_registry: &schema::SchemaRegistry,
) -> Result<(), GoldenError> {
    let change_id = case["change_id"]
        .as_str()
        .ok_or_else(|| GoldenError::Mismatch("application Change ID is missing".to_owned()))?;
    let mut application = application::InMemoryApplication::new(
        case["input"]["project"].clone(),
        &case["input"]["rule_source"],
        &case["input"]["framework_lock"],
        schema_registry,
    )
    .map_err(|error| GoldenError::Mismatch(error.to_string()))?;
    let response = application
        .next(change_id)
        .map_err(|error| GoldenError::Mismatch(error.to_string()))?;
    let snapshot = application
        .snapshot(change_id)
        .map_err(|error| GoldenError::Mismatch(error.to_string()))?;
    let decision = serde_json::to_value(&response.decision)
        .expect("Kernel Decision contains serializable values");
    let decision_digest =
        canonical_digest(&decision).map_err(|error| GoldenError::Mismatch(error.to_string()))?;
    let actual = json!({
        "snapshot_digest": snapshot.digest,
        "rule_index_digest": application.rule_index_digest(),
        "framework_lock_digest": application.framework_lock_digest(),
        "decision": decision,
        "decision_digest": decision_digest,
        "context_digest": response.context.as_ref().map(|context| &context.digest),
        "context_source_digests": response
            .context
            .as_ref()
            .map(|context| context.source_digests.clone())
            .unwrap_or_default(),
    });
    if actual != case["expected"] {
        return Err(GoldenError::Mismatch(format!(
            "initial Application: expected {}, got {actual}",
            case["expected"]
        )));
    }
    Ok(())
}

fn complete_current_build<Store: application::ProjectStore>(
    application: &mut application::Application<'_, Store>,
    response: &application::ApplicationResponse,
    change_id: &str,
    input: &Value,
) -> Result<application::ApplicationResponse, GoldenError> {
    let action = response.decision.action.as_ref().ok_or_else(|| {
        GoldenError::Mismatch("build completion has no current Action".to_owned())
    })?;
    let context = response.context.as_ref().ok_or_else(|| {
        GoldenError::Mismatch("build completion has no current Context".to_owned())
    })?;
    if action.expected_result_schema != "result.build" {
        return Err(GoldenError::Mismatch(format!(
            "build completion expected result.build, got {}",
            action.expected_result_schema
        )));
    }
    let submission = submission::ResultSubmission {
        change_id: change_id.to_owned(),
        action_id: action.id.clone(),
        context_digest: context.digest.clone(),
        role: action.role.clone(),
        result_schema: action.expected_result_schema.clone(),
        payload: input["payload"].clone(),
        output_refs: string_array(&input["output_refs"]),
    };
    application
        .update_repository(input["repository"].clone())
        .map_err(|error| GoldenError::Mismatch(error.to_string()))?;
    application
        .submit(&submission)
        .map_err(|error| GoldenError::Mismatch(format!("build submission failed: {error}")))
}

fn application_checkpoint<Store: application::ProjectStore>(
    application: &application::Application<'_, Store>,
    change_id: &str,
    response: &application::ApplicationResponse,
) -> Result<Value, GoldenError> {
    let snapshot = application
        .snapshot(change_id)
        .map_err(|error| GoldenError::Mismatch(error.to_string()))?;
    let decision = serde_json::to_value(&response.decision)
        .expect("Kernel Decision contains serializable values");
    let decision_digest =
        canonical_digest(&decision).map_err(|error| GoldenError::Mismatch(error.to_string()))?;
    let action = response.decision.action.as_ref().map(|action| {
        json!({
            "id": action.id,
            "role": action.role,
            "result_schema": action.expected_result_schema,
            "instance_keys": action
                .requirement_instances
                .iter()
                .map(|instance| instance.instance_key.as_str())
                .collect::<Vec<_>>(),
        })
    });
    Ok(json!({
        "state": response.decision.state,
        "snapshot_digest": snapshot.digest,
        "decision_digest": decision_digest,
        "context_digest": response.context.as_ref().map(|context| &context.digest),
        "action": action,
    }))
}

fn string_array(value: &Value) -> Vec<String> {
    value
        .as_array()
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn mutate_mixed_json_path(
    source: &mut Value,
    operation: &str,
    path: &[Value],
    value: Option<Value>,
) -> Result<(), String> {
    let (field, parents) = path
        .split_last()
        .ok_or_else(|| "mutation path must not be empty".to_owned())?;
    let mut current = source;
    for parent in parents {
        current = match parent {
            Value::String(field) => current
                .as_object_mut()
                .and_then(|object| object.get_mut(field))
                .ok_or_else(|| format!("mutation parent {field:?} is missing"))?,
            Value::Number(index) => {
                let index = index
                    .as_u64()
                    .ok_or_else(|| "mutation array index must be unsigned".to_owned())?
                    as usize;
                current
                    .as_array_mut()
                    .and_then(|array| array.get_mut(index))
                    .ok_or_else(|| format!("mutation array index {index} is missing"))?
            }
            _ => return Err("mutation path segment must be a string or integer".to_owned()),
        };
    }
    match field {
        Value::String(field) => {
            let object = current
                .as_object_mut()
                .ok_or_else(|| "mutation target parent must be an object".to_owned())?;
            match operation {
                "set" | "add" => {
                    object.insert(
                        field.clone(),
                        value.ok_or_else(|| "mutation requires value".to_owned())?,
                    );
                }
                "remove" => {
                    if object.remove(field).is_none() {
                        return Err(format!("remove target {field:?} is missing"));
                    }
                }
                other => return Err(format!("unsupported mutation operation {other:?}")),
            }
        }
        Value::Number(index) => {
            let index = index
                .as_u64()
                .ok_or_else(|| "mutation array index must be unsigned".to_owned())?
                as usize;
            let array = current
                .as_array_mut()
                .ok_or_else(|| "mutation target parent must be an array".to_owned())?;
            match operation {
                "set" => {
                    let target = array
                        .get_mut(index)
                        .ok_or_else(|| format!("mutation array index {index} is missing"))?;
                    *target = value.ok_or_else(|| "set mutation requires value".to_owned())?;
                }
                "remove" => {
                    if index >= array.len() {
                        return Err(format!("mutation array index {index} is missing"));
                    }
                    array.remove(index);
                }
                other => return Err(format!("unsupported array mutation operation {other:?}")),
            }
        }
        _ => return Err("mutation path segment must be a string or integer".to_owned()),
    }
    Ok(())
}

fn mutate_json_path(
    source: &mut Value,
    operation: &str,
    path: &[String],
    value: Option<Value>,
) -> Result<(), String> {
    let (field, parents) = path
        .split_last()
        .ok_or_else(|| "mutation path must not be empty".to_owned())?;
    let mut current = source;
    for parent in parents {
        current = current
            .as_object_mut()
            .and_then(|object| object.get_mut(parent))
            .ok_or_else(|| format!("mutation parent {parent:?} is missing"))?;
    }
    let object = current
        .as_object_mut()
        .ok_or_else(|| "mutation target parent must be an object".to_owned())?;
    match operation {
        "set" => {
            if !object.contains_key(field) {
                return Err(format!("set target {field:?} is missing"));
            }
            object.insert(
                field.clone(),
                value.ok_or_else(|| "set mutation requires value".to_owned())?,
            );
        }
        "add" => {
            if object.contains_key(field) {
                return Err(format!("add target {field:?} already exists"));
            }
            object.insert(
                field.clone(),
                value.ok_or_else(|| "add mutation requires value".to_owned())?,
            );
        }
        "remove" => {
            if object.remove(field).is_none() {
                return Err(format!("remove target {field:?} is missing"));
            }
        }
        other => return Err(format!("unsupported mutation operation {other:?}")),
    }
    Ok(())
}

fn resolve_context_instances(
    definitions: &std::collections::BTreeMap<String, kernel::RequirementInstance>,
    references: &[String],
    case_id: &str,
) -> Result<Vec<kernel::RequirementInstance>, GoldenError> {
    references
        .iter()
        .map(|reference| {
            definitions.get(reference).cloned().ok_or_else(|| {
                GoldenError::Mismatch(format!(
                    "{case_id}: unknown Requirement Instance fixture {reference:?}"
                ))
            })
        })
        .collect()
}

fn resolve_inside(root: &Path, relative: &str) -> Result<PathBuf, GoldenError> {
    let relative = Path::new(relative);
    if relative.is_absolute() {
        return Err(GoldenError::InvalidPath(
            "golden path must be relative".to_owned(),
        ));
    }
    let candidate = root
        .join(relative)
        .canonicalize()
        .map_err(|error| GoldenError::Io(format!("cannot resolve golden case: {error}")))?;
    if !candidate.starts_with(root) {
        return Err(GoldenError::InvalidPath(
            "golden path escapes suite root".to_owned(),
        ));
    }
    Ok(candidate)
}

fn resolve_relative_within(
    base: &Path,
    relative: &str,
    allowed_root: &Path,
) -> Result<PathBuf, GoldenError> {
    let relative = Path::new(relative);
    if relative.is_absolute() {
        return Err(GoldenError::InvalidPath(
            "shared asset path must be relative".to_owned(),
        ));
    }
    let candidate = base
        .join(relative)
        .canonicalize()
        .map_err(|error| GoldenError::Io(format!("cannot resolve shared asset: {error}")))?;
    if !candidate.starts_with(allowed_root) {
        return Err(GoldenError::InvalidPath(
            "shared asset path escapes vNext root".to_owned(),
        ));
    }
    Ok(candidate)
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T, GoldenError> {
    let bytes =
        fs::read(path).map_err(|error| GoldenError::Io(format!("{}: {error}", path.display())))?;
    serde_json::from_slice(&bytes)
        .map_err(|error| GoldenError::Json(format!("{}: {error}", path.display())))
}

#[derive(Debug)]
pub enum GoldenError {
    Io(String),
    Json(String),
    InvalidPath(String),
    Mismatch(String),
}

impl fmt::Display for GoldenError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(message) => write!(formatter, "golden I/O error: {message}"),
            Self::Json(message) => write!(formatter, "golden JSON error: {message}"),
            Self::InvalidPath(message) => write!(formatter, "golden path error: {message}"),
            Self::Mismatch(message) => write!(formatter, "golden mismatch: {message}"),
        }
    }
}

impl std::error::Error for GoldenError {}

#[derive(Deserialize)]
struct Manifest {
    suite_id: String,
    cases: Vec<ManifestEntry>,
}

#[derive(Deserialize)]
struct ManifestEntry {
    kind: String,
    path: String,
}

#[derive(Deserialize)]
struct CanonicalizationCaseSet {
    canonicalization_version: String,
    number_scope: String,
    cases: Vec<CanonicalizationCase>,
    invalid_cases: Vec<InvalidCanonicalizationCase>,
}

#[derive(Deserialize)]
struct CanonicalizationCase {
    case_id: String,
    value: Value,
    canonical_json: String,
    digest: String,
}

#[derive(Deserialize)]
struct InvalidCanonicalizationCase {
    case_id: String,
    value: Value,
    error: String,
}

#[derive(Deserialize)]
struct SchemaCaseSet {
    schema_bundle_version: String,
    schema_bundle_digest: String,
    schema_root: String,
    cases: Vec<SchemaCase>,
}

#[derive(Deserialize)]
struct SchemaCase {
    case_id: String,
    record_kind: String,
    record: Value,
    valid: bool,
    error_path: Option<String>,
}

#[derive(Deserialize)]
struct RuleCompilationCaseSet {
    compiler_version: String,
    schema_root: String,
    schema_bundle_digest: String,
    cases: Vec<RuleCompilationCase>,
    invalid_cases: Vec<InvalidRuleCompilationCase>,
}

#[derive(Deserialize)]
struct RuleCompilationCase {
    case_id: String,
    source_variants: Vec<Value>,
    expected: Value,
}

#[derive(Deserialize)]
struct InvalidRuleCompilationCase {
    case_id: String,
    source: Value,
    error: String,
}

#[derive(Deserialize)]
struct DetectionCaseSet {
    detector_id: String,
    detector_version: String,
    cases: Vec<DetectionCase>,
    invalid_cases: Vec<InvalidDetectionCase>,
}

#[derive(Deserialize)]
struct DetectionCase {
    case_id: String,
    input_variants: Vec<DetectionInput>,
    expected: Value,
}

#[derive(Deserialize)]
struct DetectionInput {
    change_id: String,
    facts: Vec<Value>,
    coverage: Value,
    artifact_digests: std::collections::BTreeMap<String, String>,
}

#[derive(Deserialize)]
struct InvalidDetectionCase {
    case_id: String,
    input: DetectionInput,
    error: String,
}

#[derive(Deserialize)]
struct KernelCaseSet {
    kernel_version: String,
    schema_root: String,
    common: KernelCommon,
    cases: Vec<KernelCase>,
}

#[derive(Deserialize)]
struct KernelCommon {
    change_id: String,
    artifact_digests: std::collections::BTreeMap<String, String>,
    facts: Vec<Value>,
    coverage: Value,
    rule_source: Value,
    result_records: std::collections::BTreeMap<String, Value>,
}

#[derive(Deserialize)]
struct KernelCase {
    case_id: String,
    input: KernelInput,
    expected: Value,
}

#[derive(Deserialize)]
struct KernelInput {
    repository_phase: String,
    result_refs: Vec<String>,
    #[serde(default)]
    coverage: Option<Value>,
    #[serde(default)]
    contracts: Vec<Value>,
    #[serde(default)]
    decisions: Vec<Value>,
}

#[derive(Deserialize)]
struct ContextCaseSet {
    context_compiler_version: String,
    common: ContextCommon,
    cases: Vec<ContextCase>,
}

#[derive(Deserialize)]
struct ContextCommon {
    snapshot: ContextSnapshotSource,
    requirement_instances: std::collections::BTreeMap<String, kernel::RequirementInstance>,
}

#[derive(Deserialize)]
struct ContextSnapshotSource {
    change_id: String,
    change: Value,
    contracts: Vec<Value>,
    decisions: Vec<Value>,
    results: Vec<Value>,
    evidence: Vec<Value>,
    repository: Value,
    artifact_digests: std::collections::BTreeMap<String, String>,
}

#[derive(Deserialize)]
struct ContextCase {
    case_id: String,
    state: String,
    all_instance_refs: Vec<String>,
    action: Option<ContextActionSource>,
    expected: Value,
}

#[derive(Deserialize)]
struct ContextActionSource {
    id: String,
    role: String,
    action: String,
    instance_refs: Vec<String>,
    reason: String,
    expected_result_schema: String,
    #[serde(default)]
    candidate_fingerprints: Vec<String>,
}

#[derive(Deserialize)]
struct ProjectSnapshotCaseSet {
    snapshot_protocol_version: String,
    schema_root: String,
    base_case: String,
    unrelated_records: std::collections::BTreeMap<String, Vec<Value>>,
    #[serde(default)]
    multi_change_records: std::collections::BTreeMap<String, Vec<Value>>,
    cases: Vec<ProjectSnapshotCase>,
    invalid_cases: Vec<InvalidProjectSnapshotCase>,
    expected: Value,
}

#[derive(Deserialize)]
struct ProjectSnapshotCase {
    case_id: String,
    change_id: String,
    #[serde(default)]
    reverse_record_collections: bool,
    #[serde(default)]
    append_unrelated_records: bool,
    #[serde(default)]
    append_multi_change_records: bool,
    #[serde(default)]
    expected: Option<Value>,
}

#[derive(Deserialize)]
struct InvalidProjectSnapshotCase {
    case_id: String,
    change_id: String,
    error: String,
}

#[derive(Deserialize)]
struct FrameworkLockCaseSet {
    framework_lock_schema_version: String,
    schema_root: String,
    base_case: String,
    expected_digest: String,
    invalid_cases: Vec<InvalidFrameworkLockCase>,
}

#[derive(Deserialize)]
struct InvalidFrameworkLockCase {
    case_id: String,
    operation: String,
    path: Vec<String>,
    value: Option<Value>,
    error_path: String,
}

#[derive(Deserialize)]
struct ResultSubmissionCaseSet {
    result_submission_protocol_version: String,
    schema_root: String,
    base_case: String,
    scenario_case: String,
    step_index: usize,
    expected: Value,
    invalid_cases: Vec<InvalidResultSubmissionCase>,
}

#[derive(Deserialize)]
struct InvalidResultSubmissionCase {
    case_id: String,
    operation: String,
    path: Vec<Value>,
    value: Option<Value>,
    error: String,
}

#[derive(Deserialize)]
struct FilesystemProjectCaseSet {
    filesystem_project_protocol_version: String,
    schema_root: String,
    base_case: String,
    result_case: String,
    scenario_case: String,
    decision_step: usize,
    contract_step: usize,
    expected_initial_snapshot_digest: String,
    expected_result_snapshot_digest: String,
    expected_updated_snapshot_digest: String,
    markdown_prose: String,
    shared_contract_concurrency: SharedContractConcurrencyCase,
    formats: Vec<FilesystemFormatCase>,
    invalid_source_roots: Vec<InvalidSourceRootCase>,
}

#[derive(Deserialize)]
struct SharedContractConcurrencyCase {
    initial: Value,
    first_update: Value,
    stale_update: Value,
    missing_digest_error: String,
    stale_error: String,
    expected_text: String,
    clause_updates: SharedContractClauseUpdateCase,
}

#[derive(Deserialize)]
struct SharedContractClauseUpdateCase {
    second_change: Value,
    initial: Value,
    first_clause_id: String,
    second_clause_id: String,
    first_update: Value,
    non_overlapping_update: Value,
    conflicting_update: Value,
    stale_error: String,
    expected_first_text: String,
    expected_second_text: String,
}

#[derive(Deserialize)]
struct FilesystemFormatCase {
    document_format: String,
    contract_path: String,
    initial_paths: Vec<String>,
    updated_paths: Vec<String>,
}

#[derive(Deserialize)]
struct InvalidSourceRootCase {
    case_id: String,
    contract_root: String,
    error: String,
}

#[derive(Deserialize)]
struct PersistentApplicationCaseSet {
    persistent_application_protocol_version: String,
    schema_root: String,
    scenario_case: String,
    document_formats: Vec<String>,
    expected_checkpoints: usize,
    expected_result_records: usize,
}

#[derive(Deserialize)]
struct ExplainCaseSet {
    schema_version: String,
    explain_report_schema_version: String,
    schema_path: String,
    base_case: String,
    scenario_case: String,
    checkpoints: Vec<ExplainCheckpoint>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct ExplainCheckpoint {
    label: String,
    after_step: Option<usize>,
    state: String,
    candidate_count: usize,
    requirement_count: usize,
    authority_statuses: Vec<String>,
    report_digest: String,
    text_digest: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn sorts_nested_keys_without_escaping_unicode() {
        let value = json!({
            "message": "注文✓",
            "items": [3, {"β": true, "a": null}]
        });
        assert_eq!(
            canonical_json(&value).unwrap(),
            r#"{"items":[3,{"a":null,"β":true}],"message":"注文✓"}"#
        );
    }

    #[test]
    fn rejects_floating_point_values() {
        assert_eq!(
            canonical_json(&json!({"ratio": 1.5})).unwrap_err(),
            CanonicalError::FloatingPointNotSupported
        );
    }
}
