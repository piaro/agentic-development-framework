//! Stable public projections for CLI output.
//!
//! Internal Kernel structs may grow independently. The CLI exposes only the
//! fields needed to understand or execute the current action.

use crate::application::ApplicationResponse;
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
    lines.join("\n") + "\n"
}
