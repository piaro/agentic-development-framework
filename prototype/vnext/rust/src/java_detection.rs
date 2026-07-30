//! Mechanical observations extracted from Java syntax.

use crate::source_detection::{SourceObservation, classify_method};
use tree_sitter::{Node, Parser};

pub fn observe_java(source: &str) -> Result<Vec<SourceObservation>, String> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_java::LANGUAGE.into())
        .map_err(|error| format!("cannot initialize Java parser: {error}"))?;
    let tree = parser
        .parse(source, None)
        .ok_or_else(|| "Java parser returned no syntax tree".to_owned())?;
    let root = tree.root_node();
    if root.has_error() {
        return Err("Java source contains a syntax error".to_owned());
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
    let declared_symbol = if matches!(
        node.kind(),
        "method_declaration" | "constructor_declaration"
    ) {
        node.child_by_field_name("name")
            .and_then(|name| name.utf8_text(source).ok())
    } else {
        None
    };
    let symbol = declared_symbol.or(enclosing_symbol);

    if node.kind() == "method_invocation"
        && let Some(observation) = observation_for_invocation(node, source, symbol)
    {
        observations.push(observation);
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_observations(child, source, symbol, observations);
    }
}

fn observation_for_invocation(
    invocation: Node<'_>,
    source: &[u8],
    symbol: Option<&str>,
) -> Option<SourceObservation> {
    let receiver = invocation.child_by_field_name("object")?;
    let name = invocation.child_by_field_name("name")?;
    let method = name.utf8_text(source).ok()?;
    Some(SourceObservation {
        kind: classify_method(method),
        symbol: symbol.unwrap_or("<module>").to_owned(),
        resource: receiver.utf8_text(source).ok()?.to_owned(),
        method: method.to_owned(),
        line: invocation.start_position().row + 1,
    })
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
                    symbol: "placeOrder".to_owned(),
                    resource: "orders".to_owned(),
                    method: "insert".to_owned(),
                    line: 8,
                },
                SourceObservation {
                    kind: SourceObservationKind::MessagePublish,
                    symbol: "placeOrder".to_owned(),
                    resource: "orderEvents".to_owned(),
                    method: "publish".to_owned(),
                    line: 9,
                },
                SourceObservation {
                    kind: SourceObservationKind::OtherMethodCall,
                    symbol: "OrderService".to_owned(),
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
    fn rejects_invalid_java() {
        let error = observe_java("class OrderService { void save( }").unwrap_err();
        assert_eq!(error, "Java source contains a syntax error");
    }
}
