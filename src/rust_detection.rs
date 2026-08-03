//! Mechanical observations extracted from Rust syntax.

use crate::source_detection::{SourceObservation, observation_from_nodes, observe_tree};
use tree_sitter::Node;

pub fn observe_rust(source: &str) -> Result<Vec<SourceObservation>, String> {
    observe_tree(
        source,
        &tree_sitter_rust::LANGUAGE.into(),
        "Rust",
        collect_root,
    )
}

fn collect_root(node: Node<'_>, source: &[u8], observations: &mut Vec<SourceObservation>) {
    collect_observations(node, source, None, None, observations);
}

fn collect_observations(
    node: Node<'_>,
    source: &[u8],
    enclosing_impl: Option<&str>,
    enclosing_symbol: Option<&str>,
    observations: &mut Vec<SourceObservation>,
) {
    let declared_impl = if node.kind() == "impl_item" {
        node.child_by_field_name("type")
            .and_then(|type_node| crate::source_detection::canonical_node_text(type_node, source))
    } else {
        None
    };
    let impl_name = declared_impl.as_deref().or(enclosing_impl);
    let declared_symbol = if node.kind() == "function_item" {
        node.child_by_field_name("name")
            .and_then(|name| name.utf8_text(source).ok())
            .map(|name| match impl_name {
                Some(impl_name) => format!("{impl_name}::{name}"),
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
        collect_observations(child, source, impl_name, symbol, observations);
    }
}

fn observation_for_call(
    call: Node<'_>,
    source: &[u8],
    symbol: Option<&str>,
) -> Option<SourceObservation> {
    let callable = call.child_by_field_name("function")?;
    let callable = if callable.kind() == "generic_function" {
        callable.child_by_field_name("function")?
    } else {
        callable
    };
    if callable.kind() != "field_expression" {
        return None;
    }
    let receiver = callable.child_by_field_name("value")?;
    let field = callable.child_by_field_name("field")?;
    observation_from_nodes("rust", receiver, field, call, source, symbol)
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
                    symbol: "OrderService::cancel_order".to_owned(),
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
    fn observes_turbofish_calls_and_normalizes_multiline_receivers() {
        let observations = observe_rust(
            r#"
fn place_order(order: Order) {
    client
        .table("orders")
        .insert::<Order>(order);
}
"#,
        )
        .unwrap();
        let insert = observations
            .iter()
            .find(|observation| observation.method == "insert")
            .unwrap();
        assert_eq!(insert.resource, "client.table(\"orders\")");
    }

    #[test]
    fn qualifies_same_named_methods_by_impl_type() {
        let observations = observe_rust(
            "impl A { fn save(&self) { orders.insert(1); } }\n\
             impl B { fn save(&self) { orders.insert(1); } }\n",
        )
        .unwrap();
        assert_eq!(
            observations
                .iter()
                .map(|observation| observation.symbol.as_str())
                .collect::<Vec<_>>(),
            vec!["A::save", "B::save"]
        );
    }

    #[test]
    fn rejects_invalid_rust() {
        let error = observe_rust("fn place_order( {").unwrap_err();
        assert_eq!(error, "Rust source contains a syntax error");
    }
}
