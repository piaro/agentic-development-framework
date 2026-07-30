//! Deterministic compilation of human-maintained Requirements and Rules.
//!
//! This module rejects only structural configuration errors that can be
//! decided without domain interpretation. It does not decide whether a
//! Requirement is sufficient or whether a Rule should apply to a project.

use crate::canonical_digest;
use crate::schema::SchemaRegistry;
use crate::signal_catalog::{SignalDefinition, signal_definition};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

pub const RULE_COMPILER_VERSION: &str = "1";

const ALLOWED_PHASES: [&str; 2] = ["before-build", "before-merge"];
const ALLOWED_ROLES: [&str; 4] = ["Analyst", "Human", "Builder", "Challenger"];
const ALLOWED_CONTEXT_SELECTORS: [&str; 11] = [
    "change",
    "repository-artifacts",
    "affected-code",
    "matching-contracts",
    "matching-decisions",
    "dependency-results",
    "matching-evidence",
    "contracts",
    "decisions",
    "results",
    "evidence",
];

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Assurance {
    #[default]
    Attestation,
    EvidenceBacked,
}

impl Assurance {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Attestation => "attestation",
            Self::EvidenceBacked => "evidence-backed",
        }
    }

    pub fn is_attestation(value: &Self) -> bool {
        *value == Self::Attestation
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RequirementDefinition {
    pub id: String,
    pub phase: String,
    pub role: String,
    pub result_schema: String,
    pub depends_on: Vec<String>,
    pub context: Vec<String>,
    #[serde(default, skip_serializing_if = "Assurance::is_attestation")]
    pub assurance: Assurance,
    pub definition_digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ActivationRule {
    pub id: String,
    #[serde(rename = "requirement")]
    pub requirement_id: String,
    pub condition: String,
    pub signal: Option<String>,
    pub repository_phase: Option<String>,
    pub subjects: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleIndex {
    pub requirements: BTreeMap<String, RequirementDefinition>,
    pub rules: Vec<ActivationRule>,
    pub digest: String,
}

impl RuleIndex {
    /// Return the language-neutral representation reviewed in shared golden data.
    pub fn compatibility_value(&self) -> Value {
        let mut value = normalized_value(&self.requirements, &self.rules);
        value
            .as_object_mut()
            .expect("normalized Rule Index is an object")
            .insert("digest".to_owned(), Value::String(self.digest.clone()));
        value
    }
}

/// Validate one source and build an order-independent Rule Index identity.
pub fn compile_rule_index(
    source: &Value,
    schema_registry: &SchemaRegistry,
) -> Result<RuleIndex, RuleCompileError> {
    let source = source
        .as_object()
        .ok_or_else(|| RuleCompileError::new("rule source must be a mapping"))?;
    let mut requirements = BTreeMap::new();

    for raw in optional_array(source, "requirements")? {
        let raw = require_object(raw, "requirement")?;
        let requirement_id = required_string(raw, "id")?;
        if requirements.contains_key(requirement_id) {
            return Err(RuleCompileError::new(format!(
                "duplicate requirement: {requirement_id}"
            )));
        }
        let phase = required_string(raw, "phase")?;
        if !ALLOWED_PHASES.contains(&phase) {
            return Err(RuleCompileError::new(format!("invalid phase: {phase}")));
        }
        let role = required_string(raw, "role")?;
        if !ALLOWED_ROLES.contains(&role) {
            return Err(RuleCompileError::new(format!("invalid role: {role}")));
        }
        let result_schema = required_string(raw, "result_schema")?;
        if !schema_registry.supports_result_schema(result_schema) {
            return Err(RuleCompileError::new(format!(
                "{requirement_id} refers to unsupported Result schema: {result_schema}"
            )));
        }
        if !schema_registry.supports_result_role(result_schema, role) {
            return Err(RuleCompileError::new(format!(
                "{requirement_id} cannot use role {role} with {result_schema}"
            )));
        }
        let assurance = match optional_string(raw, "assurance")? {
            None | Some("attestation") => Assurance::Attestation,
            Some("evidence-backed") => Assurance::EvidenceBacked,
            Some(value) => {
                return Err(RuleCompileError::new(format!(
                    "unsupported assurance: {value}"
                )));
            }
        };
        if assurance == Assurance::EvidenceBacked && result_schema != "result.evidence" {
            return Err(RuleCompileError::new(format!(
                "{requirement_id} uses evidence-backed assurance with {result_schema}; \
                 result.evidence is required"
            )));
        }

        let mut depends_on = optional_string_array(raw, "depends_on")?;
        let mut context = optional_string_array(raw, "context")?;
        let unknown_context: BTreeSet<&str> = context
            .iter()
            .map(String::as_str)
            .filter(|selector| !ALLOWED_CONTEXT_SELECTORS.contains(selector))
            .collect();
        if !unknown_context.is_empty() {
            return Err(RuleCompileError::new(format!(
                "unknown context selector: {}",
                unknown_context.into_iter().collect::<Vec<_>>().join(", ")
            )));
        }

        // Sorting these semantic sets makes YAML list order irrelevant while
        // retaining duplicates exactly as the Python prototype currently does.
        depends_on.sort();
        context.sort();
        let mut definition_body = json!({
            "id": requirement_id,
            "phase": phase,
            "role": role,
            "result_schema": result_schema,
            "depends_on": depends_on,
            "context": context,
        });
        if assurance != Assurance::Attestation {
            definition_body
                .as_object_mut()
                .expect("Requirement definition is an object")
                .insert(
                    "assurance".to_owned(),
                    Value::String(assurance.as_str().to_owned()),
                );
        }
        let definition_digest = canonical_digest(&definition_body)
            .map_err(|error| RuleCompileError::new(error.to_string()))?;
        requirements.insert(
            requirement_id.to_owned(),
            RequirementDefinition {
                id: requirement_id.to_owned(),
                phase: phase.to_owned(),
                role: role.to_owned(),
                result_schema: result_schema.to_owned(),
                depends_on: value_string_array(&definition_body["depends_on"]),
                context: value_string_array(&definition_body["context"]),
                assurance,
                definition_digest,
            },
        );
    }

    for requirement in requirements.values() {
        for dependency in &requirement.depends_on {
            if !requirements.contains_key(dependency) {
                return Err(RuleCompileError::new(format!(
                    "{} refers to unknown dependency: {dependency}",
                    requirement.id
                )));
            }
        }
    }
    assert_acyclic(&requirements)?;

    let mut rules = Vec::new();
    let mut rule_ids = BTreeSet::new();
    for raw in optional_array(source, "rules")? {
        let raw = require_object(raw, "rule")?;
        let rule_id = required_string(raw, "id")?;
        if !rule_ids.insert(rule_id.to_owned()) {
            return Err(RuleCompileError::new(format!("duplicate rule: {rule_id}")));
        }
        let requirement_id = required_string(raw, "requirement")?;
        if !requirements.contains_key(requirement_id) {
            return Err(RuleCompileError::new(format!(
                "{rule_id} refers to unknown requirement: {requirement_id}"
            )));
        }
        let condition = optional_string(raw, "when")?.unwrap_or("signal");
        if !["always", "signal"].contains(&condition) {
            return Err(RuleCompileError::new(format!(
                "unsupported rule condition: {condition}"
            )));
        }
        let signal = optional_string(raw, "signal")?.map(str::to_owned);
        if condition == "signal" && signal.as_deref().is_none_or(str::is_empty) {
            return Err(RuleCompileError::new(format!("{rule_id} requires signal")));
        }
        if condition == "always" && signal.is_some() {
            return Err(RuleCompileError::new(format!(
                "{rule_id} cannot use signal with always"
            )));
        }
        let signal_definition = signal
            .as_deref()
            .map(|signal| {
                signal_definition(signal).ok_or_else(|| {
                    RuleCompileError::new(format!("{rule_id} refers to unknown signal: {signal}"))
                })
            })
            .transpose()?;
        let subjects = optional_string_array(raw, "subjects")?;
        validate_subjects(rule_id, &subjects, signal_definition)?;
        rules.push(ActivationRule {
            id: rule_id.to_owned(),
            requirement_id: requirement_id.to_owned(),
            condition: condition.to_owned(),
            signal,
            repository_phase: optional_string(raw, "repository_phase")?.map(str::to_owned),
            subjects,
        });
    }

    let normalized = normalized_value(&requirements, &rules);
    let digest =
        canonical_digest(&normalized).map_err(|error| RuleCompileError::new(error.to_string()))?;
    Ok(RuleIndex {
        requirements,
        rules,
        digest,
    })
}

fn validate_subjects(
    rule_id: &str,
    subjects: &[String],
    signal: Option<&SignalDefinition>,
) -> Result<(), RuleCompileError> {
    for subject in subjects {
        if subject == "change.id" {
            continue;
        }
        let Some(binding) = subject.strip_prefix("binding.") else {
            return Err(RuleCompileError::new(format!(
                "{rule_id} has unsupported subject: {subject}"
            )));
        };
        let Some(signal) = signal else {
            return Err(RuleCompileError::new(format!(
                "{rule_id} cannot use binding subject without signal: {subject}"
            )));
        };
        if !signal.bindings.contains(&binding) {
            return Err(RuleCompileError::new(format!(
                "{rule_id} refers to unknown {signal_id} binding: {binding}",
                signal_id = signal.id
            )));
        }
    }
    Ok(())
}

fn normalized_value(
    requirements: &BTreeMap<String, RequirementDefinition>,
    rules: &[ActivationRule],
) -> Value {
    let mut sorted_rules: Vec<&ActivationRule> = rules.iter().collect();
    sorted_rules.sort_by(|left, right| left.id.cmp(&right.id));
    json!({
        "requirements": requirements.values().collect::<Vec<_>>(),
        "rules": sorted_rules,
    })
}

fn assert_acyclic(
    requirements: &BTreeMap<String, RequirementDefinition>,
) -> Result<(), RuleCompileError> {
    fn visit(
        requirement_id: &str,
        requirements: &BTreeMap<String, RequirementDefinition>,
        visiting: &mut BTreeSet<String>,
        visited: &mut BTreeSet<String>,
    ) -> Result<(), RuleCompileError> {
        if visiting.contains(requirement_id) {
            return Err(RuleCompileError::new(format!(
                "dependency cycle at: {requirement_id}"
            )));
        }
        if visited.contains(requirement_id) {
            return Ok(());
        }
        visiting.insert(requirement_id.to_owned());
        for dependency in &requirements[requirement_id].depends_on {
            visit(dependency, requirements, visiting, visited)?;
        }
        visiting.remove(requirement_id);
        visited.insert(requirement_id.to_owned());
        Ok(())
    }

    let mut visiting = BTreeSet::new();
    let mut visited = BTreeSet::new();
    for requirement_id in requirements.keys() {
        visit(requirement_id, requirements, &mut visiting, &mut visited)?;
    }
    Ok(())
}

fn optional_array<'a>(
    object: &'a Map<String, Value>,
    field: &str,
) -> Result<&'a [Value], RuleCompileError> {
    match object.get(field) {
        None => Ok(&[]),
        Some(Value::Array(values)) => Ok(values),
        Some(_) => Err(RuleCompileError::new(format!("{field} must be an array"))),
    }
}

fn optional_string_array(
    object: &Map<String, Value>,
    field: &str,
) -> Result<Vec<String>, RuleCompileError> {
    optional_array(object, field)?
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| RuleCompileError::new(format!("{field} items must be strings")))
        })
        .collect()
}

fn required_string<'a>(
    object: &'a Map<String, Value>,
    field: &str,
) -> Result<&'a str, RuleCompileError> {
    object
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| RuleCompileError::new(format!("{field} must be a string")))
}

fn optional_string<'a>(
    object: &'a Map<String, Value>,
    field: &str,
) -> Result<Option<&'a str>, RuleCompileError> {
    match object.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => Ok(Some(value)),
        Some(_) => Err(RuleCompileError::new(format!("{field} must be a string"))),
    }
}

fn require_object<'a>(
    value: &'a Value,
    label: &str,
) -> Result<&'a Map<String, Value>, RuleCompileError> {
    value
        .as_object()
        .ok_or_else(|| RuleCompileError::new(format!("{label} must be an object")))
}

fn value_string_array(value: &Value) -> Vec<String> {
    value
        .as_array()
        .expect("definition field is an array")
        .iter()
        .map(|item| {
            item.as_str()
                .expect("definition array contains strings")
                .to_owned()
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleCompileError {
    message: String,
}

impl RuleCompileError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for RuleCompileError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for RuleCompileError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::SchemaRegistry;
    use std::path::PathBuf;

    fn registry() -> SchemaRegistry {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../schemas/v1");
        SchemaRegistry::load(root).unwrap()
    }

    #[test]
    fn rejects_dependency_cycle() {
        let source = json!({
            "requirements": [
                {
                    "id": "a",
                    "phase": "before-build",
                    "role": "Analyst",
                    "result_schema": "result.analysis",
                    "depends_on": ["b"]
                },
                {
                    "id": "b",
                    "phase": "before-build",
                    "role": "Analyst",
                    "result_schema": "result.analysis",
                    "depends_on": ["a"]
                }
            ]
        });
        assert_eq!(
            compile_rule_index(&source, &registry())
                .unwrap_err()
                .to_string(),
            "dependency cycle at: a"
        );
    }

    #[test]
    fn rejects_role_schema_mismatch() {
        let source = json!({
            "requirements": [{
                "id": "invalid",
                "phase": "before-build",
                "role": "Human",
                "result_schema": "result.analysis"
            }]
        });
        assert_eq!(
            compile_rule_index(&source, &registry())
                .unwrap_err()
                .to_string(),
            "invalid cannot use role Human with result.analysis"
        );
    }

    #[test]
    fn compiles_explicit_evidence_backed_assurance() {
        let source = json!({
            "requirements": [{
                "id": "tests-passed",
                "phase": "before-merge",
                "role": "Builder",
                "result_schema": "result.evidence",
                "assurance": "evidence-backed"
            }]
        });
        let index = compile_rule_index(&source, &registry()).unwrap();
        assert_eq!(
            index.requirements["tests-passed"].assurance,
            Assurance::EvidenceBacked
        );
        assert_eq!(
            index.compatibility_value()["requirements"][0]["assurance"],
            "evidence-backed"
        );
    }

    #[test]
    fn rejects_evidence_assurance_for_attestation_result_schema() {
        let source = json!({
            "requirements": [{
                "id": "analysis-complete",
                "phase": "before-build",
                "role": "Analyst",
                "result_schema": "result.analysis",
                "assurance": "evidence-backed"
            }]
        });
        assert_eq!(
            compile_rule_index(&source, &registry())
                .unwrap_err()
                .to_string(),
            "analysis-complete uses evidence-backed assurance with result.analysis; \
            result.evidence is required"
        );
    }

    #[test]
    fn rejects_a_rule_for_an_unknown_signal() {
        let source = json!({
            "requirements": [{
                "id": "analysis",
                "phase": "before-build",
                "role": "Analyst",
                "result_schema": "result.analysis"
            }],
            "rules": [{
                "id": "unknown-signal",
                "signal": "persistent-data-wirte",
                "requirement": "analysis",
                "subjects": ["binding.operation"]
            }]
        });
        assert_eq!(
            compile_rule_index(&source, &registry())
                .unwrap_err()
                .to_string(),
            "unknown-signal refers to unknown signal: persistent-data-wirte"
        );
    }

    #[test]
    fn rejects_a_subject_for_an_undefined_signal_binding() {
        let source = json!({
            "requirements": [{
                "id": "analysis",
                "phase": "before-build",
                "role": "Analyst",
                "result_schema": "result.analysis"
            }],
            "rules": [{
                "id": "invalid-binding",
                "signal": "persistent-data-write",
                "requirement": "analysis",
                "subjects": ["binding.integration"]
            }]
        });
        assert_eq!(
            compile_rule_index(&source, &registry())
                .unwrap_err()
                .to_string(),
            "invalid-binding refers to unknown persistent-data-write binding: integration"
        );
    }
}
