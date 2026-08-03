//! Mechanical observations extracted from Swift syntax.

use crate::source_detection::{
    SourceObservation, canonical_node_text, observation_from_nodes, observe_tree,
};
use tree_sitter::Node;

pub fn observe_swift(source: &str) -> Result<Vec<SourceObservation>, String> {
    observe_tree(
        source,
        &tree_sitter_swift::LANGUAGE.into(),
        "Swift",
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
    let declared_type = if matches!(node.kind(), "class_declaration" | "protocol_declaration") {
        node.child_by_field_name("name")
            .and_then(|name| canonical_node_text(name, source))
            .map(|name| match enclosing_type {
                Some(parent) => format!("{parent}.{name}"),
                None => name,
            })
    } else {
        None
    };
    let type_name = declared_type.as_deref().or(enclosing_type);
    let declared_symbol = declared_symbol(node, source, type_name);
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

fn declared_symbol(node: Node<'_>, source: &[u8], type_name: Option<&str>) -> Option<String> {
    let name = match node.kind() {
        "function_declaration" => node
            .child_by_field_name("name")
            .and_then(|name| canonical_node_text(name, source)),
        "init_declaration" => Some("init".to_owned()),
        "deinit_declaration" => Some("deinit".to_owned()),
        "subscript_declaration" => Some("subscript".to_owned()),
        "property_declaration" if is_type_or_global_property(node) => node
            .child_by_field_name("name")
            .and_then(|name| canonical_node_text(name, source)),
        _ => None,
    }?;
    match type_name {
        Some(type_name) => Some(format!("{type_name}.{name}")),
        None => Some(name),
    }
}

fn is_type_or_global_property(node: Node<'_>) -> bool {
    node.parent().is_some_and(|parent| {
        matches!(
            parent.kind(),
            "source_file" | "class_body" | "enum_class_body" | "protocol_body"
        )
    })
}

fn observation_for_call(
    call: Node<'_>,
    source: &[u8],
    symbol: Option<&str>,
) -> Option<SourceObservation> {
    let callable = call.named_child(0)?;
    if callable.kind() != "navigation_expression" {
        return None;
    }
    let receiver = callable.child_by_field_name("target")?;
    let method = callable
        .child_by_field_name("suffix")?
        .child_by_field_name("suffix")?;
    if method.kind() != "simple_identifier" {
        return None;
    }
    observation_from_nodes("swift", receiver, method, call, source, symbol)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source_detection::SourceObservationKind;

    #[test]
    fn observes_swift_methods_initializers_and_optional_calls() {
        let observations = observe_swift(
            r#"
final class OrderService {
    init() {
        audit.save(self)
    }

    func placeOrder(_ order: Order) {
        orders.insert(order)
        events?.publish(order)
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
                    symbol: "OrderService.placeOrder".to_owned(),
                    resource: "orders".to_owned(),
                    method: "insert".to_owned(),
                    line: 8,
                },
                SourceObservation {
                    kind: SourceObservationKind::MessagePublish,
                    symbol: "OrderService.placeOrder".to_owned(),
                    resource: "events".to_owned(),
                    method: "publish".to_owned(),
                    line: 9,
                },
                SourceObservation {
                    kind: SourceObservationKind::OtherMethodCall,
                    symbol: "OrderService.init".to_owned(),
                    resource: "audit".to_owned(),
                    method: "save".to_owned(),
                    line: 4,
                },
            ]
        );
    }

    #[test]
    fn names_type_property_closures_and_framework_specific_calls() {
        let observations = observe_swift(
            r#"
struct OrderHandlers {
    let placeOrder: (Order) -> Void = { order in
        repository.save(order)
    }
}
"#,
        )
        .unwrap();
        assert_eq!(
            observations,
            vec![SourceObservation {
                kind: SourceObservationKind::OtherMethodCall,
                symbol: "OrderHandlers.placeOrder".to_owned(),
                resource: "repository".to_owned(),
                method: "save".to_owned(),
                line: 4,
            }]
        );
    }

    #[test]
    fn rejects_invalid_swift() {
        let error = observe_swift("func placeOrder( {").unwrap_err();
        assert_eq!(error, "Swift source contains a syntax error");
    }
}
