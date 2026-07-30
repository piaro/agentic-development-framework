//! Mechanical observations extracted from C syntax.

use crate::source_detection::{SourceObservation, observation_from_nodes, observe_tree};
use tree_sitter::Node;

pub fn observe_c(source: &str) -> Result<Vec<SourceObservation>, String> {
    observe_tree(source, &tree_sitter_c::LANGUAGE.into(), "C", collect_root)
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
    let declared_symbol = if node.kind() == "function_definition" {
        function_name(node, source).map(|name| match enclosing_symbol {
            Some(parent) => format!("{parent}.{name}"),
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
        collect_observations(child, source, symbol, observations);
    }
}

fn function_name<'source>(function: Node<'_>, source: &'source [u8]) -> Option<&'source str> {
    declarator_identifier(function.child_by_field_name("declarator")?)?
        .utf8_text(source)
        .ok()
}

fn declarator_identifier(node: Node<'_>) -> Option<Node<'_>> {
    if node.kind() == "identifier" {
        return Some(node);
    }
    node.child_by_field_name("declarator")
        .and_then(declarator_identifier)
}

fn observation_for_call(
    call: Node<'_>,
    source: &[u8],
    symbol: Option<&str>,
) -> Option<SourceObservation> {
    let callable = unwrap_callable(call.child_by_field_name("function")?, source);
    if callable.kind() == "field_expression" {
        let receiver = callable.child_by_field_name("argument")?;
        let method = callable.child_by_field_name("field")?;
        return observation_from_nodes("c", receiver, method, call, source, symbol);
    }
    if callable.kind() != "identifier" {
        return None;
    }
    let arguments = call.child_by_field_name("arguments")?;
    let receiver = normalize_resource_argument(arguments.named_child(0)?, source);
    observation_from_nodes("c", receiver, callable, call, source, symbol)
}

fn unwrap_callable<'tree>(mut node: Node<'tree>, source: &[u8]) -> Node<'tree> {
    loop {
        match node.kind() {
            "parenthesized_expression" => {
                let Some(child) = node.named_child(0) else {
                    return node;
                };
                node = child;
            }
            "pointer_expression"
                if node
                    .child_by_field_name("operator")
                    .and_then(|operator| operator.utf8_text(source).ok())
                    == Some("*") =>
            {
                let Some(argument) = node.child_by_field_name("argument") else {
                    return node;
                };
                node = argument;
            }
            _ => return node,
        }
    }
}

fn normalize_resource_argument<'tree>(node: Node<'tree>, source: &[u8]) -> Node<'tree> {
    if node.kind() == "pointer_expression"
        && node
            .child_by_field_name("operator")
            .and_then(|operator| operator.utf8_text(source).ok())
            == Some("&")
    {
        node.child_by_field_name("argument").unwrap_or(node)
    } else {
        node
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source_detection::SourceObservationKind;

    #[test]
    fn observes_free_and_struct_function_calls() {
        let observations = observe_c(
            r#"
void place_order(Order *order) {
    insert(&orders, order);
    events->publish(events, order);
    sqlite3_exec(db, "INSERT INTO orders VALUES (?)", 0, 0, 0);
}
"#,
        )
        .unwrap();

        assert_eq!(
            observations,
            vec![
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
                    resource: "events".to_owned(),
                    method: "publish".to_owned(),
                    line: 4,
                },
                SourceObservation {
                    kind: SourceObservationKind::OtherMethodCall,
                    symbol: "place_order".to_owned(),
                    resource: "db".to_owned(),
                    method: "sqlite3_exec".to_owned(),
                    line: 5,
                },
            ]
        );
    }

    #[test]
    fn observes_parenthesized_function_pointer_members() {
        let observations =
            observe_c("void publish_order(Order *order) { (*events->publish)(events, order); }\n")
                .unwrap();
        assert_eq!(
            observations,
            vec![SourceObservation {
                kind: SourceObservationKind::MessagePublish,
                symbol: "publish_order".to_owned(),
                resource: "events".to_owned(),
                method: "publish".to_owned(),
                line: 1,
            }]
        );
    }

    #[test]
    fn ignores_calls_without_a_resource_argument() {
        let observations = observe_c("void place_order(void) { insert(); ready(); }\n").unwrap();
        assert!(observations.is_empty());
    }

    #[test]
    fn rejects_invalid_c() {
        let error = observe_c("void place_order( {").unwrap_err();
        assert_eq!(error, "C source contains a syntax error");
    }
}
