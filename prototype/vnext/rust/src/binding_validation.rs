//! Repository-wide validation report for reviewed source Binding Records.

use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeSet;

pub const BINDING_VALIDATION_REPORT_VERSION: &str = "1";

const BINDING_GAP_KINDS: [&str; 5] = [
    "ambiguous-symbol-binding",
    "invalid-binding",
    "unbound-source-artifact",
    "unmapped-observation",
    "unsupported-observation",
];

pub(crate) fn is_binding_gap_kind(kind: &str) -> bool {
    BINDING_GAP_KINDS.contains(&kind)
}

pub(crate) fn mentions_binding_gap_kind(message: &str) -> bool {
    BINDING_GAP_KINDS.iter().any(|kind| message.contains(kind))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BindingValidationIssue {
    pub category: String,
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artifact_ref: Option<String>,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BindingValidationSummary {
    pub artifacts: usize,
    pub observations: usize,
    pub facts: usize,
    pub binding_issues: usize,
    pub coverage_issues: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BindingValidationReport {
    pub schema_version: String,
    pub status: String,
    pub summary: BindingValidationSummary,
    pub issues: Vec<BindingValidationIssue>,
}

impl BindingValidationReport {
    pub fn build(
        repository: &Value,
        decisions: &[Value],
        declared_authority_refs: &[String],
    ) -> Self {
        let artifacts = repository["artifacts"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        let facts = repository["facts"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        let accepted_decisions = decisions
            .iter()
            .filter(|decision| decision["status"].as_str() == Some("accepted"))
            .filter_map(|decision| decision["id"].as_str())
            .collect::<BTreeSet<_>>();
        let mut issues = repository["coverage"]["gaps"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|gap| {
                let kind = gap["kind"].as_str().unwrap_or("invalid-coverage-gap");
                BindingValidationIssue {
                    category: if is_binding_gap_kind(kind) {
                        "binding"
                    } else {
                        "coverage"
                    }
                    .to_owned(),
                    kind: kind.to_owned(),
                    artifact_ref: gap["ref"].as_str().map(str::to_owned),
                    reason: gap["reason"]
                        .as_str()
                        .unwrap_or("Repository coverage gap has no reason")
                        .to_owned(),
                }
            })
            .collect::<Vec<_>>();

        let mut used_authority_refs = declared_authority_refs
            .iter()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        for fact in facts {
            used_authority_refs.extend(
                fact["binding_authority_refs"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str),
            );
        }
        issues.extend(
            used_authority_refs
                .into_iter()
                .filter(|authority_ref| !accepted_decisions.contains(authority_ref))
                .map(|authority_ref| BindingValidationIssue {
                    category: "binding".to_owned(),
                    kind: "unaccepted-binding-authority".to_owned(),
                    artifact_ref: None,
                    reason: format!(
                        "binding authority is not an accepted Decision: {authority_ref}"
                    ),
                }),
        );
        issues.sort_by(|left, right| {
            (
                left.category.as_str(),
                left.kind.as_str(),
                left.artifact_ref.as_deref(),
                left.reason.as_str(),
            )
                .cmp(&(
                    right.category.as_str(),
                    right.kind.as_str(),
                    right.artifact_ref.as_deref(),
                    right.reason.as_str(),
                ))
        });
        issues.dedup();

        let binding_issues = issues
            .iter()
            .filter(|issue| issue.category == "binding")
            .count();
        let coverage_issues = issues
            .iter()
            .filter(|issue| issue.category == "coverage")
            .count();
        let status = if binding_issues > 0 {
            "invalid"
        } else if coverage_issues > 0 {
            "blocked"
        } else {
            "valid"
        };
        Self {
            schema_version: BINDING_VALIDATION_REPORT_VERSION.to_owned(),
            status: status.to_owned(),
            summary: BindingValidationSummary {
                artifacts: artifacts.len(),
                observations: artifacts
                    .iter()
                    .filter_map(|artifact| artifact["observations"].as_array())
                    .map(Vec::len)
                    .sum(),
                facts: facts.len(),
                binding_issues,
                coverage_issues,
            },
            issues,
        }
    }

    pub fn is_valid(&self) -> bool {
        self.status == "valid"
    }

    pub fn render_text(&self) -> String {
        let mut lines = vec![
            format!("Binding validation: {}", self.status),
            format!(
                "Artifacts: {}, observations: {}, facts: {}",
                self.summary.artifacts, self.summary.observations, self.summary.facts
            ),
            format!(
                "Binding issues: {}, coverage issues: {}",
                self.summary.binding_issues, self.summary.coverage_issues
            ),
        ];
        for issue in &self.issues {
            let reference = issue
                .artifact_ref
                .as_deref()
                .map(|reference| format!(" [{reference}]"))
                .unwrap_or_default();
            lines.push(format!(
                "- {}:{}{}: {}",
                issue.category, issue.kind, reference, issue.reason
            ));
        }
        match self.status.as_str() {
            "valid" => {
                lines.push("Next: Binding Records are ready for normal project evaluation.".to_owned())
            }
            "invalid" => lines.extend([
                "Next: run agentic project observe to generate a non-authoritative Binding draft."
                    .to_owned(),
                "Then: review the listed physical names, logical refs, owners, fact kinds, and accepted Decision authorities."
                    .to_owned(),
                "Then: run agentic project validate-bindings again.".to_owned(),
                "Binding candidates are never applied automatically.".to_owned(),
            ]),
            _ => lines.push(
                "Next: resolve coverage issues before treating Binding validation as complete."
                    .to_owned(),
            ),
        }
        lines.join("\n") + "\n"
    }

    pub fn as_value(&self) -> Value {
        serde_json::to_value(self).expect("Binding Validation Report is serializable")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn distinguishes_binding_violations_from_coverage_blocks() {
        let repository = json!({
            "artifacts": [{
                "ref": "code.orders",
                "observations": [{"kind": "db_write"}]
            }],
            "facts": [{
                "binding_authority_refs": ["decision.bindings"]
            }],
            "coverage": {
                "gaps": [
                    {
                        "kind": "unmapped-observation",
                        "ref": "code.orders",
                        "reason": "resource orders has no resource binding"
                    },
                    {
                        "kind": "parse-error",
                        "ref": "code.other",
                        "reason": "invalid syntax"
                    }
                ]
            }
        });
        let report = BindingValidationReport::build(
            &repository,
            &[json!({
                "id": "decision.bindings",
                "status": "proposed"
            })],
            &["decision.bindings".to_owned()],
        );

        assert_eq!(report.status, "invalid");
        assert_eq!(report.summary.binding_issues, 2);
        assert_eq!(report.summary.coverage_issues, 1);
        let text = report.render_text();
        assert!(text.contains("Next: run agentic project observe"));
        assert!(text.contains("Then: run agentic project validate-bindings again"));
        assert!(text.contains("never applied automatically"));
    }
}
