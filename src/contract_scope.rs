//! Shared Contract and clause scope resolution.
//!
//! A clause-level `applies_to` overrides the Contract-level value. Legacy
//! clauses without it inherit the Contract scope.

use serde_json::Value;
use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ClauseEvidenceMode {
    /// Contracts written before selective Evidence retain their old behavior.
    LegacyDirect,
    Direct,
    Inherited,
    Review,
}

impl ClauseEvidenceMode {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::LegacyDirect | Self::Direct => "direct",
            Self::Inherited => "inherited",
            Self::Review => "review",
        }
    }
}

pub(crate) fn effective_clause_applies_to(contract: &Value, clause: &Value) -> Vec<String> {
    clause["applies_to"]
        .as_array()
        .or_else(|| contract["applies_to"].as_array())
        .map(|targets| {
            targets
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

pub(crate) fn effective_clause_evidence_mode(
    contract: &Value,
    clause: &Value,
) -> ClauseEvidenceMode {
    match clause["evidence_mode"]
        .as_str()
        .or_else(|| contract["evidence_mode"].as_str())
    {
        Some("direct") => ClauseEvidenceMode::Direct,
        Some("inherited") => ClauseEvidenceMode::Inherited,
        Some("review") => ClauseEvidenceMode::Review,
        _ => ClauseEvidenceMode::LegacyDirect,
    }
}

pub(crate) fn clause_matches_subjects(
    contract: &Value,
    clause: &Value,
    subjects: &BTreeSet<&str>,
) -> bool {
    effective_clause_applies_to(contract, clause)
        .iter()
        .any(|target| subjects.contains(target.as_str()))
}

pub(crate) fn contract_matches_subjects(contract: &Value, subjects: &BTreeSet<&str>) -> bool {
    if let Some(clauses) = contract["clauses"]
        .as_array()
        .filter(|clauses| !clauses.is_empty())
    {
        return clauses
            .iter()
            .any(|clause| clause_matches_subjects(contract, clause, subjects));
    }
    contract["applies_to"].as_array().is_some_and(|targets| {
        targets
            .iter()
            .filter_map(Value::as_str)
            .any(|target| subjects.contains(target))
    })
}

pub(crate) fn contract_uses_clause_scopes(contract: &Value) -> bool {
    contract["clauses"].as_array().is_some_and(|clauses| {
        clauses
            .iter()
            .any(|clause| clause.get("applies_to").is_some())
    })
}
