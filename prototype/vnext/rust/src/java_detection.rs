//! Mechanical observations extracted from Java syntax.

use crate::source_detection::{SourceObservation, observation_from_nodes, observe_tree};
use tree_sitter::Node;

pub fn observe_java(source: &str) -> Result<Vec<SourceObservation>, String> {
    observe_tree(
        source,
        &tree_sitter_java::LANGUAGE.into(),
        "Java",
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
        "class_declaration"
            | "record_declaration"
            | "enum_declaration"
            | "interface_declaration"
            | "annotation_type_declaration"
    ) {
        node.child_by_field_name("name")
            .and_then(|name| name.utf8_text(source).ok())
            .map(|name| match enclosing_type {
                Some(parent) => format!("{parent}.{name}"),
                None => name.to_owned(),
            })
    } else {
        None
    };
    let type_name = declared_type.as_deref().or(enclosing_type);
    let declared_symbol = if matches!(
        node.kind(),
        "method_declaration" | "constructor_declaration"
    ) {
        node.child_by_field_name("name")
            .and_then(|name| name.utf8_text(source).ok())
            .map(|name| match type_name {
                Some(type_name) => format!("{type_name}.{name}"),
                None => name.to_owned(),
            })
    } else if node.kind() == "static_initializer" {
        type_name.map(|type_name| format!("{type_name}.<static>"))
    } else {
        None
    };
    let symbol = declared_symbol.as_deref().or(enclosing_symbol);

    if node.kind() == "method_invocation"
        && let Some(observation) = observation_for_invocation(node, source, symbol)
    {
        observations.push(observation);
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_observations(child, source, type_name, symbol, observations);
    }
}

fn observation_for_invocation(
    invocation: Node<'_>,
    source: &[u8],
    symbol: Option<&str>,
) -> Option<SourceObservation> {
    let receiver = invocation.child_by_field_name("object")?;
    let name = invocation.child_by_field_name("name")?;
    observation_from_nodes("java", receiver, name, invocation, source, symbol)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source_detection::SourceObservationKind;

    #[test]
    fn observes_java_method_and_constructor_calls() {
        let observations = observe_java(
            r#"
final class OrderService {
    OrderService() {
        audit.save(this);
    }

    void placeOrder(Order order) {
        orders.insert(order);
        orderEvents.publish(order);
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
                    resource: "orderEvents".to_owned(),
                    method: "publish".to_owned(),
                    line: 9,
                },
                SourceObservation {
                    kind: SourceObservationKind::OtherMethodCall,
                    symbol: "OrderService.OrderService".to_owned(),
                    resource: "audit".to_owned(),
                    method: "save".to_owned(),
                    line: 4,
                },
            ]
        );
    }

    #[test]
    fn ignores_names_inside_comments_and_strings() {
        let observations = observe_java(
            r#"
class OrderService {
    String describe() {
        // orders.insert(order);
        return "orderEvents.publish(order)";
    }
}
"#,
        )
        .unwrap();
        assert!(observations.is_empty());
    }

    #[test]
    fn qualifies_same_named_methods_by_declaring_type() {
        let observations = observe_java(
            "class A { void save() { orders.insert(null); } }\n\
             class B { void save() { orders.insert(null); } }\n",
        )
        .unwrap();
        assert_eq!(
            observations
                .iter()
                .map(|observation| observation.symbol.as_str())
                .collect::<Vec<_>>(),
            vec!["A.save", "B.save"]
        );
    }

    #[test]
    fn names_static_initializer_blocks() {
        let observations =
            observe_java("class Service { static { orders.insert(null); } }").unwrap();
        assert_eq!(observations[0].symbol, "Service.<static>");
    }

    #[test]
    fn rejects_invalid_java() {
        let error = observe_java("class OrderService { void save( }").unwrap_err();
        assert_eq!(error, "Java source contains a syntax error");
    }
}
