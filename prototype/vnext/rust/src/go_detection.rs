//! Mechanical observations extracted from Go syntax.

use crate::source_detection::{SourceObservation, classify_method};
use tree_sitter::{Node, Parser};

pub fn observe_go(source: &str) -> Result<Vec<SourceObservation>, String> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_go::LANGUAGE.into())
        .map_err(|error| format!("cannot initialize Go parser: {error}"))?;
    let tree = parser
        .parse(source, None)
        .ok_or_else(|| "Go parser returned no syntax tree".to_owned())?;
    let root = tree.root_node();
    if root.has_error() {
        return Err("Go source contains a syntax error".to_owned());
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
    let declared_symbol = if matches!(node.kind(), "function_declaration" | "method_declaration") {
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
    if callable.kind() != "selector_expression" {
        return None;
    }
    let receiver = callable.child_by_field_name("operand")?;
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
    fn observes_go_function_and_method_calls() {
        let observations = observe_go(
            r#"
package ordering

func PlaceOrder(order Order) {
    orders.Insert(order)
    orderEvents.Publish(order)
}

func (service *OrderService) CancelOrder(order Order) {
    orders.Delete(order)
}
"#,
        )
        .unwrap();

        assert_eq!(
            observations,
            vec![
                SourceObservation {
                    kind: SourceObservationKind::DbWrite,
                    symbol: "CancelOrder".to_owned(),
                    resource: "orders".to_owned(),
                    method: "Delete".to_owned(),
                    line: 10,
                },
                SourceObservation {
                    kind: SourceObservationKind::DbWrite,
                    symbol: "PlaceOrder".to_owned(),
                    resource: "orders".to_owned(),
                    method: "Insert".to_owned(),
                    line: 5,
                },
                SourceObservation {
                    kind: SourceObservationKind::MessagePublish,
                    symbol: "PlaceOrder".to_owned(),
                    resource: "orderEvents".to_owned(),
                    method: "Publish".to_owned(),
                    line: 6,
                },
            ]
        );
    }

    #[test]
    fn retains_framework_specific_calls_for_reviewed_bindings() {
        let observations = observe_go(
            r#"
package ordering

func PlaceOrder(order Order) {
    repository.Save(order)
}
"#,
        )
        .unwrap();
        assert_eq!(
            observations,
            vec![SourceObservation {
                kind: SourceObservationKind::OtherMethodCall,
                symbol: "PlaceOrder".to_owned(),
                resource: "repository".to_owned(),
                method: "Save".to_owned(),
                line: 5,
            }]
        );
    }

    #[test]
    fn rejects_invalid_go() {
        let error = observe_go("package ordering\nfunc PlaceOrder( {").unwrap_err();
        assert_eq!(error, "Go source contains a syntax error");
    }
}
