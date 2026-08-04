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
    if response.decision.state == "blocked-detection" {
        lines.extend(detection_guidance(&response.decision.diagnostics));
    }
    lines.join("\n") + "\n"
}

const COVERAGE_DIAGNOSTIC_PREFIX: &str = "detection coverage incomplete: ";

/// The coverage gap kind a diagnostic reports, when it reports one.
fn coverage_gap_kind(diagnostic: &str) -> Option<&str> {
    let rest = diagnostic.strip_prefix(COVERAGE_DIAGNOSTIC_PREFIX)?;
    let end = rest.find([' ', ':']).unwrap_or(rest.len());
    Some(&rest[..end])
}

/// What to do about a stopped change, told apart by why it stopped.
///
/// A stop that means "review these bindings" and a stop that means "this
/// language has no detector" read identically once they are both a blocked
/// state with a diagnostic. The second is a limit of the kit rather than
/// anything wrong with the repository, and saying so is the difference between
/// a user acting on it and a user filing it as a defect.
fn detection_guidance(diagnostics: &[String]) -> Vec<String> {
    let kinds = diagnostics
        .iter()
        .filter_map(|diagnostic| coverage_gap_kind(diagnostic))
        .collect::<Vec<_>>();
    let mut lines = Vec::new();

    if kinds.contains(&"unsupported-language") {
        lines.push(String::new());
        lines.push(
            "The sources above are written in a language this build has no detector for, so their calls cannot be observed at all."
                .to_owned(),
        );
        lines.push(
            "This is a limit of the kit, not a defect in the repository, and no binding review can resolve it."
                .to_owned(),
        );
        lines.push(
            "Either narrow analysis.roots in .agentic/repository-observation.yaml so these sources are not analyzed, or leave them and accept that the change stops here until a detector exists."
                .to_owned(),
        );
    }

    if kinds.contains(&"parse-error") {
        lines.push(String::new());
        lines.push(
            "The sources above are in a supported language but did not parse, so nothing could be observed from them."
                .to_owned(),
        );
        lines.push(
            "Fix the source and run again. If it is valid and the parser is wrong, that is a defect worth reporting."
                .to_owned(),
        );
    }

    if diagnostics
        .iter()
        .any(|diagnostic| mentions_binding_gap_kind(diagnostic))
    {
        lines.push(String::new());
        lines.push(
            "Next: run agentic project observe --output .agentic/repository-observation.draft.yaml to generate a non-authoritative Binding draft."
                .to_owned(),
        );
        lines.push(
            "Then: review the draft, complete its logical refs, owners, fact kinds, and accepted Decision authorities."
                .to_owned(),
        );
        lines.push("Then: run agentic project validate-bindings.".to_owned());
        lines.push("Binding candidates are never applied automatically.".to_owned());
    }

    lines
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
        let output = render_next_text("change.test", &blocked(&["parse-error"]));

        assert!(!output.contains("project observe"));
    }

    fn blocked(kinds: &[&str]) -> ApplicationResponse {
        ApplicationResponse {
            decision: KernelDecision {
                state: "blocked-detection".to_owned(),
                action: None,
                requirement_instances: Vec::new(),
                diagnostics: kinds
                    .iter()
                    .map(|kind| {
                        format!("detection coverage incomplete: {kind} (code.example): reason")
                    })
                    .collect(),
            },
            context: None,
        }
    }

    #[test]
    fn an_unsupported_language_says_it_is_a_limit_rather_than_a_defect() {
        let output = render_next_text("change.test", &blocked(&["unsupported-language"]));

        assert!(output.contains("limit of the kit, not a defect"));
        assert!(
            output.contains("analysis.roots"),
            "the way out has to be named: {output}"
        );
        assert!(
            !output.contains("project observe"),
            "no binding review can resolve a missing detector"
        );
    }

    #[test]
    fn a_parse_error_points_at_the_source_rather_than_the_kit() {
        let output = render_next_text("change.test", &blocked(&["parse-error"]));

        assert!(output.contains("did not parse"));
        assert!(output.contains("Fix the source"));
        assert!(!output.contains("limit of the kit"));
    }

    #[test]
    fn a_change_blocked_for_several_reasons_gets_guidance_for_each() {
        let output = render_next_text(
            "change.test",
            &blocked(&["unsupported-language", "unmapped-observation"]),
        );

        assert!(output.contains("limit of the kit, not a defect"));
        assert!(output.contains("project observe"));
    }

    #[test]
    fn a_diagnostic_that_is_not_a_coverage_gap_is_left_alone() {
        assert_eq!(coverage_gap_kind("something else entirely"), None);
        assert_eq!(
            coverage_gap_kind("detection coverage incomplete: parse-error"),
            Some("parse-error")
        );
        assert_eq!(
            coverage_gap_kind("detection coverage incomplete: parse-error (code.x): reason"),
            Some("parse-error")
        );
    }
}
