//! Mechanical observations extracted from Kotlin syntax.

use crate::source_detection::{SourceObservation, classify_method};
use tree_sitter::{Node, Parser};

pub fn observe_kotlin(source: &str) -> Result<Vec<SourceObservation>, String> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_kotlin_ng::LANGUAGE.into())
        .map_err(|error| format!("cannot initialize Kotlin parser: {error}"))?;
    let tree = parser
        .parse(source, None)
        .ok_or_else(|| "Kotlin parser returned no syntax tree".to_owned())?;
    let root = tree.root_node();
    if root.has_error() {
        return Err("Kotlin source contains a syntax error".to_owned());
    }

    let mut observations = Vec::new();
    collect_observations(root, source.as_bytes(), None, &mut observations);
    observations.sort();
    observations.dedup();
    Ok(observations)
}

fn collect_observations(
    node: Node<'_>,
    source: &[u8],
    enclosing_symbol: Option<&str>,
    observations: &mut Vec<SourceObservation>,
) {
    let declared_symbol = if node.kind() == "function_declaration" {
        node.child_by_field_name("name")
            .or_else(|| first_named_child_of_kind(node, "identifier"))
            .and_then(|name| name.utf8_text(source).ok())
    } else {
        None
    };
    let symbol = declared_symbol.or(enclosing_symbol);

    if node.kind() == "call_expression"
        && let Some(observation) = observation_for_call(node, source, symbol)
    {
        observations.push(observation);
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_observations(child, source, symbol, observations);
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
    let method = method_node.utf8_text(source).ok()?;
    Some(SourceObservation {
        kind: classify_method(method),
        symbol: symbol.unwrap_or("<module>").to_owned(),
        resource: receiver.utf8_text(source).ok()?.to_owned(),
        method: method.to_owned(),
        line: call.start_position().row + 1,
    })
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
                    symbol: "cancelOrder".to_owned(),
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
