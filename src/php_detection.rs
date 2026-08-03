//! Mechanical observations extracted from PHP syntax.

use crate::source_detection::{SourceObservation, observation_from_nodes, observe_tree};
use tree_sitter::Node;

pub fn observe_php(source: &str) -> Result<Vec<SourceObservation>, String> {
    observe_tree(
        source,
        &tree_sitter_php::LANGUAGE_PHP.into(),
        "PHP",
        collect_root,
    )
}

fn collect_root(node: Node<'_>, source: &[u8], observations: &mut Vec<SourceObservation>) {
    collect_observations(node, source, None, None, observations);
}

fn collect_observations(
    node: Node<'_>,
    source: &[u8],
    enclosing_type: Option<&str>,
    enclosing_symbol: Option<&str>,
    observations: &mut Vec<SourceObservation>,
) {
    let declared_type = if matches!(
        node.kind(),
        "class_declaration" | "interface_declaration" | "trait_declaration" | "enum_declaration"
    ) {
        node.child_by_field_name("name")
            .and_then(|name| name.utf8_text(source).ok())
            .map(|name| match enclosing_type {
                Some(parent) => format!("{parent}::{name}"),
                None => name.to_owned(),
            })
    } else {
        None
    };
    let type_name = declared_type.as_deref().or(enclosing_type);
    let declared_symbol = if matches!(node.kind(), "function_definition" | "method_declaration") {
        node.child_by_field_name("name")
            .or_else(|| first_named_child_of_kind(node, "name"))
            .and_then(|name| name.utf8_text(source).ok())
            .map(|name| match (type_name, node.kind()) {
                (Some(type_name), "method_declaration") => format!("{type_name}::{name}"),
                _ => name.to_owned(),
            })
    } else {
        None
    };
    let symbol = declared_symbol.as_deref().or(enclosing_symbol);

    if matches!(
        node.kind(),
        "member_call_expression" | "nullsafe_member_call_expression" | "scoped_call_expression"
    ) && let Some(observation) = observation_for_call(node, source, symbol)
    {
        observations.push(observation);
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_observations(child, source, type_name, symbol, observations);
    }
}

fn observation_for_call(
    call: Node<'_>,
    source: &[u8],
    symbol: Option<&str>,
) -> Option<SourceObservation> {
    let receiver = call
        .child_by_field_name("object")
        .or_else(|| call.child_by_field_name("scope"))?;
    let name = call.child_by_field_name("name")?;
    observation_from_nodes("php", receiver, name, call, source, symbol)
}

fn first_named_child_of_kind<'tree>(node: Node<'tree>, kind: &str) -> Option<Node<'tree>> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor)
        .find(|child| child.kind() == kind)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source_detection::SourceObservationKind;

    #[test]
    fn observes_php_function_method_and_nullsafe_calls() {
        let observations = observe_php(
            r#"<?php
function placeOrder($order) {
    $orders->insert($order);
    $events?->publish($order);
}

final class OrderService {
    public function cancelOrder($order) {
        $orders->delete($order);
    }
}
"#,
        )
        .unwrap();

        assert_eq!(
            observations,
            vec![
                SourceObservation {
                    kind: SourceObservationKind::DbWrite,
                    symbol: "OrderService::cancelOrder".to_owned(),
                    resource: "$orders".to_owned(),
                    method: "delete".to_owned(),
                    line: 9,
                },
                SourceObservation {
                    kind: SourceObservationKind::DbWrite,
                    symbol: "placeOrder".to_owned(),
                    resource: "$orders".to_owned(),
                    method: "insert".to_owned(),
                    line: 3,
                },
                SourceObservation {
                    kind: SourceObservationKind::MessagePublish,
                    symbol: "placeOrder".to_owned(),
                    resource: "$events".to_owned(),
                    method: "publish".to_owned(),
                    line: 4,
                },
            ]
        );
    }

    #[test]
    fn retains_framework_specific_calls_for_reviewed_bindings() {
        let observations = observe_php(
            "<?php\nfunction placeOrder($order) {\n    $repository->save($order);\n}\n",
        )
        .unwrap();
        assert_eq!(
            observations,
            vec![SourceObservation {
                kind: SourceObservationKind::OtherMethodCall,
                symbol: "placeOrder".to_owned(),
                resource: "$repository".to_owned(),
                method: "save".to_owned(),
                line: 3,
            }]
        );
    }

    #[test]
    fn rejects_invalid_php() {
        let error = observe_php("<?php\nfunction placeOrder( {\n").unwrap_err();
        assert_eq!(error, "PHP source contains a syntax error");
    }
}
