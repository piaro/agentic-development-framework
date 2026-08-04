//! Explicit, project-owned policy for turning Contract Health into a CI gate.
//!
//! The health report remains read-only and diagnostic by default. A gate is
//! activated only when the caller supplies a policy that names the health
//! states which must stop CI.

use crate::contract_health::ContractHealthReport;
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeSet;
use std::fmt;
use std::fs;
use std::path::Path;

pub const CONTRACT_HEALTH_GATE_REPORT_SCHEMA_VERSION: &str = "1";
pub const CONTRACT_HEALTH_POLICY_SCHEMA_VERSION: &str = "1";
const ALLOWED_FAILURE_STATES: [&str; 3] = ["failed", "stale", "unverified"];

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ContractHealthPolicy {
    pub schema_version: String,
    pub fail_on: Vec<String>,
}

impl ContractHealthPolicy {
    pub fn load(path: &Path) -> Result<Self, ContractHealthGateError> {
        let text = fs::read_to_string(path)
            .map_err(|error| gate_error(format!("{}: {error}", path.display())))?;
        let value: Value = serde_yaml::from_str(&text)
            .map_err(|error| gate_error(format!("{}: {error}", path.display())))?;
        Self::from_value(&value).map_err(|error| gate_error(format!("{}: {error}", path.display())))
    }

    fn from_value(value: &Value) -> Result<Self, ContractHealthGateError> {
        let object = value
            .as_object()
            .ok_or_else(|| gate_error("Contract Health policy must be a mapping"))?;
        let actual = object.keys().map(String::as_str).collect::<BTreeSet<_>>();
        let expected = ["schema_version", "fail_on"]
            .into_iter()
            .collect::<BTreeSet<_>>();
        if actual != expected {
            let mut details = Vec::new();
            for field in actual.difference(&expected) {
                details.push(format!("unexpected field: {field}"));
            }
            for field in expected.difference(&actual) {
                details.push(format!("missing field: {field}"));
            }
            return Err(gate_error(format!(
                "invalid Contract Health policy: {}",
                details.join(", ")
            )));
        }
        if object["schema_version"].as_str() != Some(CONTRACT_HEALTH_POLICY_SCHEMA_VERSION) {
            return Err(gate_error("unsupported Contract Health policy schema"));
        }
        let fail_on = object["fail_on"]
            .as_array()
            .ok_or_else(|| gate_error("Contract Health policy fail_on must be an array"))?;
        if fail_on.is_empty() {
            return Err(gate_error(
                "Contract Health policy fail_on must contain at least one state",
            ));
        }
        let mut normalized = BTreeSet::new();
        for state in fail_on {
            let state = state.as_str().ok_or_else(|| {
                gate_error("Contract Health policy fail_on entries must be strings")
            })?;
            if !ALLOWED_FAILURE_STATES.contains(&state) {
                return Err(gate_error(format!(
                    "unsupported Contract Health failure state: {state}"
                )));
            }
            if !normalized.insert(state.to_owned()) {
                return Err(gate_error(format!(
                    "duplicate Contract Health failure state: {state}"
                )));
            }
        }
        Ok(Self {
            schema_version: CONTRACT_HEALTH_POLICY_SCHEMA_VERSION.to_owned(),
            fail_on: normalized.into_iter().collect(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ContractHealthGatePolicy {
    pub path: String,
    pub fail_on: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ContractHealthGateReport {
    pub schema_version: String,
    pub status: String,
    pub policy: ContractHealthGatePolicy,
    pub blocking_clause_refs: Vec<String>,
    pub contract_health: ContractHealthReport,
}

impl ContractHealthGateReport {
    pub fn build(
        policy_path: &str,
        policy: &ContractHealthPolicy,
        contract_health: ContractHealthReport,
    ) -> Self {
        let fail_on = policy
            .fail_on
            .iter()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        let blocking_clause_refs = contract_health
            .clauses
            .iter()
            .filter(|clause| fail_on.contains(clause.status.as_str()))
            .map(|clause| clause.clause_ref.clone())
            .collect::<Vec<_>>();
        let status = if blocking_clause_refs.is_empty() {
            "passed"
        } else {
            "failed"
        };
        Self {
            schema_version: CONTRACT_HEALTH_GATE_REPORT_SCHEMA_VERSION.to_owned(),
            status: status.to_owned(),
            policy: ContractHealthGatePolicy {
                path: policy_path.to_owned(),
                fail_on: policy.fail_on.clone(),
            },
            blocking_clause_refs,
            contract_health,
        }
    }

    pub fn passed(&self) -> bool {
        self.status == "passed"
    }

    pub fn as_value(&self) -> Value {
        serde_json::to_value(self).expect("Contract Health Gate Report is serializable")
    }

    pub fn render_text(&self) -> String {
        let blocking = if self.blocking_clause_refs.is_empty() {
            "-".to_owned()
        } else {
            self.blocking_clause_refs.join(", ")
        };
        format!(
            "Contract Health gate: {}\npolicy: {}\nfail_on: {}\nblocking_clauses: {}\n{}",
            self.status,
            self.policy.path,
            self.policy.fail_on.join(", "),
            blocking,
            self.contract_health.render_text()
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContractHealthGateError {
    message: String,
}

impl fmt::Display for ContractHealthGateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ContractHealthGateError {}

fn gate_error(message: impl Into<String>) -> ContractHealthGateError {
    ContractHealthGateError {
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract_health::{ClauseHealth, ContractHealthSummary};
    use serde_json::json;

    fn clause(reference: &str, status: &str) -> ClauseHealth {
        ClauseHealth {
            contract_id: "contract.orders".to_owned(),
            clause_id: reference.to_owned(),
            clause_ref: format!("contract.orders#{reference}"),
            text: "test clause".to_owned(),
            applies_to: vec![],
            authority_ref: None,
            status: status.to_owned(),
            evidence_refs: vec![],
            verification_result_ids: vec![],
            stale_refs: vec![],
        }
    }

    fn health() -> ContractHealthReport {
        ContractHealthReport {
            schema_version: "1".to_owned(),
            repository_revision: Some("abc123".to_owned()),
            summary: ContractHealthSummary {
                total: 4,
                verified: 1,
                stale: 1,
                unverified: 1,
                failed: 1,
            },
            clauses: vec![
                clause("failed", "failed"),
                clause("stale", "stale"),
                clause("unverified", "unverified"),
                clause("verified", "verified"),
            ],
        }
    }

    #[test]
    fn only_explicit_failure_states_block_the_gate() {
        for (states, expected) in [
            (vec!["failed"], vec!["contract.orders#failed"]),
            (vec!["stale"], vec!["contract.orders#stale"]),
            (vec!["unverified"], vec!["contract.orders#unverified"]),
        ] {
            let policy = ContractHealthPolicy {
                schema_version: "1".to_owned(),
                fail_on: states.into_iter().map(str::to_owned).collect(),
            };
            let report = ContractHealthGateReport::build(".adf/health.yaml", &policy, health());
            assert!(!report.passed());
            assert_eq!(report.blocking_clause_refs, expected);
        }
        let policy = ContractHealthPolicy {
            schema_version: "1".to_owned(),
            fail_on: vec!["stale".to_owned()],
        };
        let verified_only = ContractHealthReport {
            clauses: vec![clause("verified", "verified")],
            summary: ContractHealthSummary {
                total: 1,
                verified: 1,
                stale: 0,
                unverified: 0,
                failed: 0,
            },
            ..health()
        };
        assert!(
            ContractHealthGateReport::build(".adf/health.yaml", &policy, verified_only).passed()
        );
    }

    #[test]
    fn policy_is_strict_and_deterministically_normalized() {
        let policy = ContractHealthPolicy::from_value(&json!({
            "schema_version": "1",
            "fail_on": ["unverified", "failed", "stale"]
        }))
        .unwrap();
        assert_eq!(policy.fail_on, vec!["failed", "stale", "unverified"]);

        for invalid in [
            json!({"schema_version": "2", "fail_on": ["failed"]}),
            json!({"schema_version": "1", "fail_on": []}),
            json!({"schema_version": "1", "fail_on": ["verified"]}),
            json!({"schema_version": "1", "fail_on": ["failed", "failed"]}),
            json!({"schema_version": "1", "fail_on": ["failed"], "extra": true}),
        ] {
            assert!(ContractHealthPolicy::from_value(&invalid).is_err());
        }
    }
}
