//! Mechanical observations extracted from Rust syntax.

use crate::source_detection::{SourceObservation, classify_method};
use tree_sitter::{Node, Parser};

pub fn observe_rust(source: &str) -> Result<Vec<SourceObservation>, String> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_rust::LANGUAGE.into())
        .map_err(|error| format!("cannot initialize Rust parser: {error}"))?;
    let tree = parser
        .parse(source, None)
        .ok_or_else(|| "Rust parser returned no syntax tree".to_owned())?;
    let root = tree.root_node();
    if root.has_error() {
        return Err("Rust source contains a syntax error".to_owned());
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
    let declared_symbol = if node.kind() == "function_item" {
        node.child_by_field_name("name")
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
    let callable = call.child_by_field_name("function")?;
    if callable.kind() != "field_expression" {
        return None;
    }
    let receiver = callable.child_by_field_name("value")?;
    let field = callable.child_by_field_name("field")?;
    let method = field.utf8_text(source).ok()?;
    Some(SourceObservation {
        kind: classify_method(method),
        symbol: symbol.unwrap_or("<module>").to_owned(),
        resource: receiver.utf8_text(source).ok()?.to_owned(),
        method: method.to_owned(),
        line: call.start_position().row + 1,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source_detection::SourceObservationKind;

    #[test]
    fn observes_rust_function_and_impl_method_calls() {
        let observations = observe_rust(
            r#"
fn place_order(order: Order) {
    orders.insert(order);
    order_events.publish(order);
}

impl OrderService {
    fn cancel_order(&self, order: Order) {
        self.orders.delete(order);
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
                    symbol: "cancel_order".to_owned(),
                    resource: "self.orders".to_owned(),
                    method: "delete".to_owned(),
                    line: 9,
                },
                SourceObservation {
                    kind: SourceObservationKind::DbWrite,
                    symbol: "place_order".to_owned(),
                    resource: "orders".to_owned(),
                    method: "insert".to_owned(),
                    line: 3,
                },
                SourceObservation {
                    kind: SourceObservationKind::MessagePublish,
                    symbol: "place_order".to_owned(),
                    resource: "order_events".to_owned(),
                    method: "publish".to_owned(),
                    line: 4,
                },
            ]
        );
    }

    #[test]
    fn retains_framework_specific_calls_for_reviewed_bindings() {
        let observations = observe_rust(
            r#"
fn place_order(order: Order) {
    repository.save(order);
}
"#,
        )
        .unwrap();
        assert_eq!(
            observations,
            vec![SourceObservation {
                kind: SourceObservationKind::OtherMethodCall,
                symbol: "place_order".to_owned(),
                resource: "repository".to_owned(),
                method: "save".to_owned(),
                line: 3,
            }]
        );
    }

    #[test]
    fn rejects_invalid_rust() {
        let error = observe_rust("fn place_order( {").unwrap_err();
        assert_eq!(error, "Rust source contains a syntax error");
    }
}
