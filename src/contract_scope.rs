//! Shared Contract and clause scope resolution.
//!
//! A clause-level `applies_to` overrides the Contract-level value. Legacy
//! clauses without it inherit the Contract scope.

use serde_json::Value;
use std::collections::BTreeSet;

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
