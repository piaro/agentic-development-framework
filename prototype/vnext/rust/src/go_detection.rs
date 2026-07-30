//! Mechanical observations extracted from Go syntax.

use crate::source_detection::{SourceObservation, observation_from_nodes, observe_tree};
use tree_sitter::Node;

pub fn observe_go(source: &str) -> Result<Vec<SourceObservation>, String> {
    observe_tree(source, &tree_sitter_go::LANGUAGE.into(), "Go", collect_root)
}

fn collect_root(node: Node<'_>, source: &[u8], observations: &mut Vec<SourceObservation>) {
    collect_observations(node, source, None, observations);
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
            .map(|name| {
                if node.kind() == "method_declaration" {
                    receiver_type(node, source)
                        .map(|receiver| format!("{receiver}.{name}"))
                        .unwrap_or_else(|| name.to_owned())
                } else {
                    name.to_owned()
                }
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
        collect_observations(child, source, symbol, observations);
    }
}

fn receiver_type(method: Node<'_>, source: &[u8]) -> Option<String> {
    let receiver = method.child_by_field_name("receiver")?;
    descendant_of_kind(receiver, "type_identifier")
        .and_then(|node| node.utf8_text(source).ok())
        .map(str::to_owned)
}

fn descendant_of_kind<'tree>(node: Node<'tree>, kind: &str) -> Option<Node<'tree>> {
    if node.kind() == kind {
        return Some(node);
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if let Some(found) = descendant_of_kind(child, kind) {
            return Some(found);
        }
    }
    None
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
    observation_from_nodes("go", receiver, field, call, source, symbol)
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
                    symbol: "OrderService.CancelOrder".to_owned(),
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
    fn qualifies_same_named_methods_by_receiver_type() {
        let observations = observe_go(
            "package ordering\n\
             func (a *A) Save() { orders.Insert(1) }\n\
             func (b *B) Save() { orders.Insert(1) }\n",
        )
        .unwrap();
        assert_eq!(
            observations
                .iter()
                .map(|observation| observation.symbol.as_str())
                .collect::<Vec<_>>(),
            vec!["A.Save", "B.Save"]
        );
    }

    #[test]
    fn rejects_invalid_go() {
        let error = observe_go("package ordering\nfunc PlaceOrder( {").unwrap_err();
        assert_eq!(error, "Go source contains a syntax error");
    }
}
