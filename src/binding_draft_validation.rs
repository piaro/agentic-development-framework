//! Validation of a reviewed, non-authoritative Repository Observation Draft.

use crate::signal_catalog::SignalCatalogRegistry;
use serde::Serialize;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

pub const BINDING_DRAFT_SCHEMA_VERSION: &str = "6";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BindingDraftValidationIssue {
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artifact_ref: Option<String>,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BindingDraftValidationReport {
    pub status: String,
    pub artifacts: usize,
    pub binding_artifacts: usize,
    pub issues: Vec<BindingDraftValidationIssue>,
}

impl BindingDraftValidationReport {
    pub fn build(
        draft: &Value,
        current: &Value,
        decisions: &[Value],
        registry: &SignalCatalogRegistry,
    ) -> Self {
        let mut issues = Vec::new();
        validate_top_level(draft, &mut issues);
        validate_freshness(draft, current, &mut issues);
        let accepted_decisions = decisions
            .iter()
            .filter(|decision| decision["status"].as_str() == Some("accepted"))
            .filter_map(|decision| decision["id"].as_str())
            .collect::<BTreeSet<_>>();
        let inventory = artifact_map(&draft["artifacts"], "Draft artifacts", &mut issues);
        let bindings = artifact_map(
            &draft["binding_artifacts"],
            "Draft binding_artifacts",
            &mut issues,
        );
        for (path, artifact) in &bindings {
            validate_binding_artifact(
                path,
                artifact,
                inventory.get(path).copied(),
                &accepted_decisions,
                registry,
                &mut issues,
            );
        }
        for (path, artifact) in &inventory {
            if !bindings.contains_key(path) {
                issue(
                    &mut issues,
                    "missing-binding-artifact",
                    artifact["candidate_ref"].as_str(),
                    format!("source artifact has no retained Binding artifact: {path}"),
                );
            }
        }
        issues.sort_by(|left, right| {
            (
                left.kind.as_str(),
                left.artifact_ref.as_deref(),
                left.reason.as_str(),
            )
                .cmp(&(
                    right.kind.as_str(),
                    right.artifact_ref.as_deref(),
                    right.reason.as_str(),
                ))
        });
        issues.dedup();
        Self {
            status: if issues.is_empty() {
                "valid"
            } else {
                "invalid"
            }
            .to_owned(),
            artifacts: inventory.len(),
            binding_artifacts: bindings.len(),
            issues,
        }
    }

    pub fn is_valid(&self) -> bool {
        self.status == "valid"
    }

    pub fn render_text(&self) -> String {
        let mut lines = vec![
            format!("Binding Draft validation: {}", self.status),
            format!(
                "Artifacts: {}, binding artifacts: {}, issues: {}",
                self.artifacts,
                self.binding_artifacts,
                self.issues.len()
            ),
        ];
        for issue in &self.issues {
            let reference = issue
                .artifact_ref
                .as_deref()
                .map(|reference| format!(" [{reference}]"))
                .unwrap_or_default();
            lines.push(format!("- {}{}: {}", issue.kind, reference, issue.reason));
        }
        if self.is_valid() {
            lines.push(
                "Next: run agentic project promote-bindings --draft <path> to explicitly promote the reviewed Draft."
                    .to_owned(),
            );
        } else {
            lines.push("Next: resolve every listed issue and validate the Draft again.".to_owned());
        }
        lines.push("No authoritative project file was changed.".to_owned());
        lines.push("Binding candidates were not applied automatically.".to_owned());
        lines.join("\n") + "\n"
    }
}

pub fn analysis_roots(draft: &Value) -> Result<Vec<String>, String> {
    let roots = draft["analysis_roots"]
        .as_array()
        .ok_or_else(|| "Binding Draft analysis_roots must be an array".to_owned())?;
    if roots.is_empty() {
        return Err("Binding Draft analysis_roots must not be empty".to_owned());
    }
    roots
        .iter()
        .map(|root| {
            root.as_str()
                .filter(|root| !root.is_empty())
                .map(str::to_owned)
                .ok_or_else(|| {
                    "Binding Draft analysis_roots must contain non-empty strings".to_owned()
                })
        })
        .collect()
}

fn validate_top_level(draft: &Value, issues: &mut Vec<BindingDraftValidationIssue>) {
    let Some(object) = draft.as_object() else {
        issue(
            issues,
            "invalid-draft",
            None,
            "Binding Draft must be an object",
        );
        return;
    };
    exact_fields(
        object.keys().map(String::as_str),
        &[
            "schema_version",
            "kind",
            "analysis_roots",
            "base_observation_digest",
            "source_digests",
            "artifacts",
            "binding_artifacts",
            "next",
        ],
        "Binding Draft",
        None,
        issues,
    );
    if draft["schema_version"].as_str() != Some(BINDING_DRAFT_SCHEMA_VERSION) {
        issue(
            issues,
            "unsupported-draft-version",
            None,
            format!(
                "expected Draft schema {}, got {}",
                BINDING_DRAFT_SCHEMA_VERSION, draft["schema_version"]
            ),
        );
    }
    if draft["kind"].as_str() != Some("repository-observation-draft") {
        issue(
            issues,
            "invalid-draft",
            None,
            "kind must be repository-observation-draft",
        );
    }
}

fn validate_freshness(
    draft: &Value,
    current: &Value,
    issues: &mut Vec<BindingDraftValidationIssue>,
) {
    match (
        draft["base_observation_digest"].as_str(),
        current["base_observation_digest"].as_str(),
    ) {
        (Some(expected), Some(actual)) if is_sha256_digest(expected) && expected == actual => {}
        (Some(expected), Some(_)) if is_sha256_digest(expected) => issue(
            issues,
            "stale-observation",
            None,
            "Repository Observation changed after Draft generation",
        ),
        _ => issue(
            issues,
            "invalid-observation-digest",
            None,
            "base_observation_digest must be a lowercase SHA-256 digest",
        ),
    }
    if draft["artifacts"] != current["artifacts"] {
        issue(
            issues,
            "modified-observation-inventory",
            None,
            "observed artifacts or framework candidates differ from the current source observation",
        );
    }
    let expected = string_map(&draft["source_digests"]);
    let actual = string_map(&current["source_digests"]);
    let paths = expected
        .keys()
        .chain(actual.keys())
        .copied()
        .collect::<BTreeSet<_>>();
    for path in paths {
        if expected.get(path) != actual.get(path) {
            issue(
                issues,
                "stale-source",
                None,
                format!("source changed after Draft generation: {path}"),
            );
        }
    }
    if draft["source_digests"].as_object().is_none() {
        issue(
            issues,
            "invalid-draft",
            None,
            "source_digests must be an object",
        );
    } else if let Some(digests) = draft["source_digests"].as_object() {
        for (path, digest) in digests {
            if !digest.as_str().is_some_and(is_sha256_digest) {
                issue(
                    issues,
                    "invalid-source-digest",
                    None,
                    format!("source_digests.{path} must be a lowercase SHA-256 digest"),
                );
            }
        }
    }
}

fn validate_binding_artifact(
    path: &str,
    artifact: &Value,
    inventory: Option<&Value>,
    accepted_decisions: &BTreeSet<&str>,
    registry: &SignalCatalogRegistry,
    issues: &mut Vec<BindingDraftValidationIssue>,
) {
    let reference = artifact["ref"].as_str();
    let Some(object) = artifact.as_object() else {
        issue(
            issues,
            "invalid-binding-artifact",
            reference,
            format!("Binding artifact must be an object: {path}"),
        );
        return;
    };
    exact_fields(
        object.keys().map(String::as_str),
        &["ref", "path", "language", "bindings"],
        "Binding artifact",
        reference,
        issues,
    );
    let Some(inventory) = inventory else {
        issue(
            issues,
            "unknown-binding-artifact",
            reference,
            format!("Binding artifact is not present in Draft inventory: {path}"),
        );
        return;
    };
    if inventory["detector_status"] != "supported" {
        issue(
            issues,
            "unsupported-language",
            reference,
            format!("source artifact has no supported Detector: {path}"),
        );
    }
    if artifact["ref"] != inventory["candidate_ref"]
        || artifact["language"] != inventory["language"]
    {
        issue(
            issues,
            "binding-artifact-mismatch",
            reference,
            "ref or language does not match the observed source artifact",
        );
    }
    let Some(bindings) = artifact["bindings"].as_object() else {
        issue(
            issues,
            "invalid-binding-artifact",
            reference,
            "bindings must be an object",
        );
        return;
    };
    exact_fields(
        bindings.keys().map(String::as_str),
        &["symbols", "resources", "methods"],
        "artifact bindings",
        reference,
        issues,
    );
    let observed_symbols = string_set(&inventory["symbols"]);
    let observed_resources = string_set(&inventory["resources"]);
    let candidates = inventory["framework_candidates"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|candidate| {
            candidate["binding_key"]
                .as_str()
                .map(|key| (key, candidate))
        })
        .collect::<BTreeMap<_, _>>();
    let bound_symbols = validate_symbols(
        &bindings["symbols"],
        &observed_symbols,
        accepted_decisions,
        reference,
        issues,
    );
    let resource_refs = validate_resources(
        &bindings["resources"],
        &observed_resources,
        accepted_decisions,
        reference,
        issues,
    );
    validate_methods(
        &bindings["methods"],
        &candidates,
        &bound_symbols,
        &resource_refs,
        accepted_decisions,
        registry,
        reference,
        issues,
    );
    validate_required_observations(inventory, &bound_symbols, &resource_refs, reference, issues);
    validate_other_observations(
        inventory,
        &bindings["methods"],
        &resource_refs,
        reference,
        issues,
    );
    validate_unused_bindings(
        inventory,
        &bindings["methods"],
        &bound_symbols,
        &resource_refs,
        reference,
        issues,
    );
}

fn validate_symbols(
    value: &Value,
    observed: &BTreeSet<&str>,
    accepted: &BTreeSet<&str>,
    artifact_ref: Option<&str>,
    issues: &mut Vec<BindingDraftValidationIssue>,
) -> BTreeSet<String> {
    let Some(records) = value.as_object() else {
        issue(
            issues,
            "invalid-binding",
            artifact_ref,
            "symbol bindings must be an object",
        );
        return BTreeSet::new();
    };
    for (physical, record) in records {
        if !observed.contains(physical.as_str()) {
            issue(
                issues,
                "unknown-physical-binding",
                artifact_ref,
                format!("symbol was not observed: {physical}"),
            );
        }
        validate_record_fields(
            record,
            &["logical_ref", "owner", "authority_ref"],
            "symbol",
            physical,
            accepted,
            artifact_ref,
            issues,
        );
        if !record["logical_ref"]
            .as_str()
            .is_some_and(|value| value.starts_with("operation.") && value.len() > 10)
        {
            issue(
                issues,
                "invalid-logical-ref",
                artifact_ref,
                format!("symbol {physical} must map to an operation.* logical ref"),
            );
        }
    }
    records.keys().cloned().collect()
}

fn validate_resources(
    value: &Value,
    observed: &BTreeSet<&str>,
    accepted: &BTreeSet<&str>,
    artifact_ref: Option<&str>,
    issues: &mut Vec<BindingDraftValidationIssue>,
) -> BTreeMap<String, BTreeSet<String>> {
    let Some(records) = value.as_object() else {
        issue(
            issues,
            "invalid-binding",
            artifact_ref,
            "resource bindings must be an object",
        );
        return BTreeMap::new();
    };
    let mut logical_bindings = BTreeMap::new();
    for (physical, record) in records {
        if !observed.contains(physical.as_str()) {
            issue(
                issues,
                "unknown-physical-binding",
                artifact_ref,
                format!("resource was not observed: {physical}"),
            );
        }
        validate_record_fields(
            record,
            &["logical_refs", "owner", "authority_ref"],
            "resource",
            physical,
            accepted,
            artifact_ref,
            issues,
        );
        let Some(refs) = record["logical_refs"]
            .as_object()
            .filter(|refs| !refs.is_empty())
        else {
            issue(
                issues,
                "incomplete-binding",
                artifact_ref,
                format!("resource {physical} requires non-empty logical_refs"),
            );
            continue;
        };
        for (binding, logical_ref) in refs {
            if !logical_ref.as_str().is_some_and(|value| {
                !binding.is_empty()
                    && value.starts_with(&format!("{binding}."))
                    && value.len() > binding.len() + 1
            }) {
                issue(
                    issues,
                    "invalid-logical-ref",
                    artifact_ref,
                    format!("resource {physical} logical_refs.{binding} must be a {binding}.* ID"),
                );
            } else {
                logical_bindings
                    .entry(physical.clone())
                    .or_insert_with(BTreeSet::new)
                    .insert(binding.clone());
            }
        }
    }
    logical_bindings
}

#[allow(clippy::too_many_arguments)]
fn validate_methods(
    value: &Value,
    candidates: &BTreeMap<&str, &Value>,
    bound_symbols: &BTreeSet<String>,
    resource_refs: &BTreeMap<String, BTreeSet<String>>,
    accepted: &BTreeSet<&str>,
    registry: &SignalCatalogRegistry,
    artifact_ref: Option<&str>,
    issues: &mut Vec<BindingDraftValidationIssue>,
) {
    let Some(records) = value.as_object() else {
        issue(
            issues,
            "invalid-binding",
            artifact_ref,
            "method bindings must be an object",
        );
        return;
    };
    for (physical, record) in records {
        let candidate = candidates.get(physical.as_str()).copied();
        if candidate.is_none() {
            issue(
                issues,
                "unknown-physical-binding",
                artifact_ref,
                format!("method Binding was not offered by the Draft: {physical}"),
            );
        }
        validate_record_fields(
            record,
            &["fact_kinds", "owner", "authority_ref"],
            "method",
            physical,
            accepted,
            artifact_ref,
            issues,
        );
        let Some(kinds) = record["fact_kinds"]
            .as_array()
            .filter(|kinds| !kinds.is_empty())
        else {
            issue(
                issues,
                "incomplete-binding",
                artifact_ref,
                format!("method {physical} requires non-empty fact_kinds"),
            );
            continue;
        };
        let resource = candidate.and_then(|candidate| candidate["resource"].as_str());
        if let Some(candidate) = candidate {
            if !candidate["symbol"]
                .as_str()
                .is_some_and(|symbol| bound_symbols.contains(symbol))
            {
                issue(
                    issues,
                    "missing-symbol-binding",
                    artifact_ref,
                    format!("method {physical} requires its observed symbol Binding"),
                );
            }
            if !candidate["resource"]
                .as_str()
                .is_some_and(|resource| resource_refs.contains_key(resource))
            {
                issue(
                    issues,
                    "missing-resource-binding",
                    artifact_ref,
                    format!("method {physical} requires its observed resource Binding"),
                );
            }
        }
        let mut seen = BTreeSet::new();
        for kind in kinds {
            let Some(kind) = kind.as_str().filter(|kind| seen.insert(*kind)) else {
                issue(
                    issues,
                    "invalid-fact-kind",
                    artifact_ref,
                    format!("method {physical} fact_kinds must be unique non-empty strings"),
                );
                continue;
            };
            let Some(definition) = registry.repository_fact_definition(kind) else {
                issue(
                    issues,
                    "unknown-fact-kind",
                    artifact_ref,
                    format!("method {physical} uses unknown fact kind: {kind}"),
                );
                continue;
            };
            if let Some(resource) = resource {
                for binding in definition
                    .bindings
                    .iter()
                    .filter(|binding| binding.binding != "operation")
                {
                    if !resource_refs
                        .get(resource)
                        .is_some_and(|refs| refs.contains(&binding.binding))
                    {
                        issue(
                            issues,
                            "missing-resource-logical-ref",
                            artifact_ref,
                            format!(
                                "method {physical} fact kind {kind} requires resource {resource} logical_refs.{}",
                                binding.binding
                            ),
                        );
                    }
                }
            }
        }
    }
}

fn validate_required_observations(
    inventory: &Value,
    bound_symbols: &BTreeSet<String>,
    resource_refs: &BTreeMap<String, BTreeSet<String>>,
    artifact_ref: Option<&str>,
    issues: &mut Vec<BindingDraftValidationIssue>,
) {
    for observation in inventory["observations"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|observation| {
            matches!(
                observation["kind"].as_str(),
                Some("db_write" | "message_publish")
            )
        })
    {
        let symbol = observation["symbol"].as_str().unwrap_or_default();
        let resource = observation["resource"].as_str().unwrap_or_default();
        let required_binding = if observation["kind"] == "db_write" {
            "data"
        } else {
            "integration"
        };
        if !bound_symbols.contains(symbol) {
            issue(
                issues,
                "missing-symbol-binding",
                artifact_ref,
                format!("observed call requires symbol Binding: {symbol}"),
            );
        }
        if !resource_refs
            .get(resource)
            .is_some_and(|refs| refs.contains(required_binding))
        {
            issue(
                issues,
                "missing-resource-logical-ref",
                artifact_ref,
                format!(
                    "observed call requires resource {resource} logical_refs.{required_binding}"
                ),
            );
        }
    }
}

fn validate_other_observations(
    inventory: &Value,
    methods: &Value,
    resource_refs: &BTreeMap<String, BTreeSet<String>>,
    artifact_ref: Option<&str>,
    issues: &mut Vec<BindingDraftValidationIssue>,
) {
    let retained_methods = methods
        .as_object()
        .into_iter()
        .flat_map(|methods| methods.keys())
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    for observation in inventory["observations"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|observation| observation["kind"] == "other_method_call")
    {
        let resource = observation["resource"].as_str().unwrap_or_default();
        if !resource_refs.contains_key(resource) {
            continue;
        }
        let method = observation["method"].as_str().unwrap_or_default();
        let binding_key = format!("{resource}.{method}");
        if !retained_methods.contains(binding_key.as_str()) {
            issue(
                issues,
                "unsupported-observation",
                artifact_ref,
                format!(
                    "bound resource {resource} has an unclassified observed method: {binding_key}"
                ),
            );
        }
    }
}

fn validate_unused_bindings(
    inventory: &Value,
    methods: &Value,
    bound_symbols: &BTreeSet<String>,
    resource_refs: &BTreeMap<String, BTreeSet<String>>,
    artifact_ref: Option<&str>,
    issues: &mut Vec<BindingDraftValidationIssue>,
) {
    let mut used_symbols = BTreeSet::new();
    let mut used_resources = BTreeSet::new();
    for observation in inventory["observations"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|observation| observation["kind"] != "other_method_call")
    {
        if let Some(symbol) = observation["symbol"].as_str() {
            used_symbols.insert(symbol);
        }
        if let Some(resource) = observation["resource"].as_str() {
            used_resources.insert(resource);
        }
    }
    let retained_methods = methods
        .as_object()
        .into_iter()
        .flat_map(|methods| methods.keys())
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    for candidate in inventory["framework_candidates"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|candidate| {
            candidate["binding_key"]
                .as_str()
                .is_some_and(|key| retained_methods.contains(key))
        })
    {
        if let Some(symbol) = candidate["symbol"].as_str() {
            used_symbols.insert(symbol);
        }
        if let Some(resource) = candidate["resource"].as_str() {
            used_resources.insert(resource);
        }
    }
    for symbol in bound_symbols {
        if !used_symbols.contains(symbol.as_str()) {
            issue(
                issues,
                "unused-binding",
                artifact_ref,
                format!("symbol Binding has no retained observed call: {symbol}"),
            );
        }
    }
    for resource in resource_refs.keys() {
        if !used_resources.contains(resource.as_str()) {
            issue(
                issues,
                "unused-binding",
                artifact_ref,
                format!("resource Binding has no retained observed call: {resource}"),
            );
        }
    }
}

fn validate_record_fields(
    record: &Value,
    expected: &[&str],
    label: &str,
    physical: &str,
    accepted: &BTreeSet<&str>,
    artifact_ref: Option<&str>,
    issues: &mut Vec<BindingDraftValidationIssue>,
) {
    let Some(object) = record.as_object() else {
        issue(
            issues,
            "invalid-binding",
            artifact_ref,
            format!("{label} {physical} Binding must be an object"),
        );
        return;
    };
    exact_fields(
        object.keys().map(String::as_str),
        expected,
        &format!("{label} {physical} Binding"),
        artifact_ref,
        issues,
    );
    if record["owner"]
        .as_str()
        .is_none_or(|value| value.is_empty())
    {
        issue(
            issues,
            "incomplete-binding",
            artifact_ref,
            format!("{label} {physical} requires a non-empty owner"),
        );
    }
    match record["authority_ref"].as_str() {
        Some(authority) if accepted.contains(authority) => {}
        Some(authority) => issue(
            issues,
            "unaccepted-binding-authority",
            artifact_ref,
            format!("{label} {physical} authority is not an accepted Decision: {authority}"),
        ),
        None => issue(
            issues,
            "incomplete-binding",
            artifact_ref,
            format!("{label} {physical} requires an authority_ref"),
        ),
    }
}

fn artifact_map<'a>(
    value: &'a Value,
    label: &str,
    issues: &mut Vec<BindingDraftValidationIssue>,
) -> BTreeMap<&'a str, &'a Value> {
    let Some(artifacts) = value.as_array() else {
        issue(
            issues,
            "invalid-draft",
            None,
            format!("{label} must be an array"),
        );
        return BTreeMap::new();
    };
    let mut by_path = BTreeMap::new();
    for artifact in artifacts {
        let Some(path) = artifact["path"].as_str().filter(|path| !path.is_empty()) else {
            issue(
                issues,
                "invalid-draft",
                artifact["ref"].as_str(),
                format!("{label} entry requires a non-empty path"),
            );
            continue;
        };
        if by_path.insert(path, artifact).is_some() {
            issue(
                issues,
                "duplicate-artifact",
                artifact["ref"].as_str(),
                format!("duplicate artifact path: {path}"),
            );
        }
    }
    by_path
}

fn exact_fields<'a>(
    actual: impl Iterator<Item = &'a str>,
    expected: &[&str],
    label: &str,
    artifact_ref: Option<&str>,
    issues: &mut Vec<BindingDraftValidationIssue>,
) {
    let actual = actual.collect::<BTreeSet<_>>();
    let expected = expected.iter().copied().collect::<BTreeSet<_>>();
    for field in actual.difference(&expected) {
        issue(
            issues,
            "invalid-field",
            artifact_ref,
            format!("{label} has unexpected field: {field}"),
        );
    }
    for field in expected.difference(&actual) {
        issue(
            issues,
            "invalid-field",
            artifact_ref,
            format!("{label} is missing field: {field}"),
        );
    }
}

fn string_map(value: &Value) -> BTreeMap<&str, &str> {
    value
        .as_object()
        .into_iter()
        .flat_map(|values| values.iter())
        .filter_map(|(key, value)| value.as_str().map(|value| (key.as_str(), value)))
        .collect()
}

fn string_set(value: &Value) -> BTreeSet<&str> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect()
}

fn is_sha256_digest(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|hex| {
        hex.len() == 64
            && hex
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    })
}

fn issue(
    issues: &mut Vec<BindingDraftValidationIssue>,
    kind: &str,
    artifact_ref: Option<&str>,
    reason: impl Into<String>,
) {
    issues.push(BindingDraftValidationIssue {
        kind: kind.to_owned(),
        artifact_ref: artifact_ref.map(str::to_owned),
        reason: reason.into(),
    });
}
