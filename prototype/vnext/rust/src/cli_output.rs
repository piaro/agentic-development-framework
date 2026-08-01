//! Stable public projections for CLI output.
//!
//! Internal Kernel structs may grow independently. The CLI exposes only the
//! fields needed to understand or execute the current action.

use crate::application::ApplicationResponse;
use crate::binding_validation::mentions_binding_gap_kind;
use serde_json::{Value, json};

pub const NEXT_RESPONSE_SCHEMA_VERSION: &str = "1";

pub fn next_response_value(change_id: &str, response: &ApplicationResponse) -> Value {
    let next_action = response.decision.action.as_ref().map(|action| {
        json!({
            "id": action.id,
            "role": action.role,
            "action": action.action,
            "reason": action.reason,
            "result_schema": action.expected_result_schema,
            "requirement_instances": action
                .requirement_instances
                .iter()
                .map(|instance| instance.instance_key.as_str())
                .collect::<Vec<_>>(),
            "candidate_fingerprints": action.candidate_fingerprints,
        })
    });
    json!({
        "schema_version": NEXT_RESPONSE_SCHEMA_VERSION,
        "change_id": change_id,
        "state": response.decision.state,
        "next_action": next_action,
        "context": response.context,
        "diagnostics": response.decision.diagnostics,
    })
}

pub fn render_next_text(change_id: &str, response: &ApplicationResponse) -> String {
    let mut lines = vec![
        format!("change: {change_id}"),
        format!("state: {}", response.decision.state),
    ];
    match &response.decision.action {
        Some(action) => {
            lines.push(format!(
                "next: {}/{} ({})",
                action.role, action.action, action.reason
            ));
            lines.push(format!("action_id: {}", action.id));
            lines.push(format!("result_schema: {}", action.expected_result_schema));
        }
        None => lines.push("next: none".to_owned()),
    }
    if let Some(context) = &response.context {
        lines.push(format!("context_digest: {}", context.digest));
        lines.push(format!(
            "sources: {}",
            if context.source_refs.is_empty() {
                "-".to_owned()
            } else {
                context.source_refs.join(",")
            }
        ));
    }
    if response.decision.state == "blocked-detection"
        && response
            .decision
            .diagnostics
            .iter()
            .any(|diagnostic| mentions_binding_gap_kind(diagnostic))
    {
        lines.push(String::new());
        lines.push(
            "Next: run agentic project observe to generate a non-authoritative Binding draft."
                .to_owned(),
        );
        lines.push(
            "Then: review the draft, complete its logical refs, owners, fact kinds, and accepted Decision authorities."
                .to_owned(),
        );
        lines.push("Then: run agentic project validate-bindings.".to_owned());
        lines.push("Binding candidates are never applied automatically.".to_owned());
    }
    lines.join("\n") + "\n"
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::KernelDecision;

    #[test]
    fn binding_detection_block_points_to_the_review_workflow() {
        let response = ApplicationResponse {
            decision: KernelDecision {
                state: "blocked-detection".to_owned(),
                action: None,
                requirement_instances: Vec::new(),
                diagnostics: vec![
                    "detection coverage incomplete: unsupported-observation: policy.apply"
                        .to_owned(),
                ],
            },
            context: None,
        };

        let output = render_next_text("change.test", &response);

        assert!(output.contains("Next: run agentic project observe"));
        assert!(output.contains("Then: run agentic project validate-bindings"));
        assert!(output.contains("never applied automatically"));
    }

    #[test]
    fn non_binding_detection_block_does_not_offer_a_binding_draft() {
        let response = ApplicationResponse {
            decision: KernelDecision {
                state: "blocked-detection".to_owned(),
                action: None,
                requirement_instances: Vec::new(),
                diagnostics: vec!["detection coverage incomplete: parse-error".to_owned()],
            },
            context: None,
        };

        let output = render_next_text("change.test", &response);

        assert!(!output.contains("project observe"));
    }
}
