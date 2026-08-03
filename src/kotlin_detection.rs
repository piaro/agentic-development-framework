//! Mechanical observations extracted from Kotlin syntax.

use crate::source_detection::{SourceObservation, observation_from_nodes, observe_tree};
use tree_sitter::Node;

pub fn observe_kotlin(source: &str) -> Result<Vec<SourceObservation>, String> {
    observe_tree(
        source,
        &tree_sitter_kotlin_ng::LANGUAGE.into(),
        "Kotlin",
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
        "class_declaration" | "object_declaration" | "companion_object"
    ) {
        node.child_by_field_name("name")
            .or_else(|| first_named_child_of_kind(node, "type_identifier"))
            .or_else(|| first_named_child_of_kind(node, "identifier"))
            .and_then(|name| name.utf8_text(source).ok())
            .map(|name| match enclosing_type {
                Some(parent) => format!("{parent}.{name}"),
                None => name.to_owned(),
            })
    } else {
        None
    };
    let type_name = declared_type.as_deref().or(enclosing_type);
    let declared_symbol = if node.kind() == "function_declaration" {
        node.child_by_field_name("name")
            .or_else(|| first_named_child_of_kind(node, "identifier"))
            .and_then(|name| name.utf8_text(source).ok())
            .map(|name| match type_name {
                Some(type_name) => format!("{type_name}.{name}"),
                None => name.to_owned(),
            })
    } else {
        None
    };
    let symbol = declared_symbol.as_deref().or(enclosing_symbol);

    if node.kind() == "call_expression"
        && let Some(observation) = observation_for_call(node, source, symbol)
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
    let callable = call
        .child_by_field_name("function")
        .or_else(|| call.named_child(0))?;
    if callable.kind() != "navigation_expression" {
        return None;
    }
    let receiver = callable.named_child(0)?;
    let method_node = callable.named_child(callable.named_child_count().checked_sub(1)?)?;
    if receiver.id() == method_node.id() {
        return None;
    }
    observation_from_nodes("kotlin", receiver, method_node, call, source, symbol)
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
    fn observes_kotlin_top_level_and_class_method_calls() {
        let observations = observe_kotlin(
            r#"
fun placeOrder(order: Order) {
    orders.insert(order)
    orderEvents.publish(order)
}

class OrderService {
    fun cancelOrder(order: Order) {
        orders.delete(order)
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
                    symbol: "OrderService.cancelOrder".to_owned(),
                    resource: "orders".to_owned(),
                    method: "delete".to_owned(),
                    line: 9,
                },
                SourceObservation {
                    kind: SourceObservationKind::DbWrite,
                    symbol: "placeOrder".to_owned(),
                    resource: "orders".to_owned(),
                    method: "insert".to_owned(),
                    line: 3,
                },
                SourceObservation {
                    kind: SourceObservationKind::MessagePublish,
                    symbol: "placeOrder".to_owned(),
                    resource: "orderEvents".to_owned(),
                    method: "publish".to_owned(),
                    line: 4,
                },
            ]
        );
    }

    #[test]
    fn retains_framework_specific_calls_for_reviewed_bindings() {
        let observations = observe_kotlin(
            r#"
fun placeOrder(order: Order) {
    repository.save(order)
}
"#,
        )
        .unwrap();
        assert_eq!(
            observations,
            vec![SourceObservation {
                kind: SourceObservationKind::OtherMethodCall,
                symbol: "placeOrder".to_owned(),
                resource: "repository".to_owned(),
                method: "save".to_owned(),
                line: 3,
            }]
        );
    }

    #[test]
    fn rejects_invalid_kotlin() {
        let error = observe_kotlin("fun placeOrder( {").unwrap_err();
        assert_eq!(error, "Kotlin source contains a syntax error");
    }
}
