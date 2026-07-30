//! Mechanical observations extracted from Python syntax.
//!
//! This module deliberately stops at physical source identities. Mapping a
//! function or receiver to a project-owned logical ID is handled by the Git
//! repository Adapter's reviewed Binding Record.

use crate::source_detection::{SourceObservation, observation_from_nodes, observe_tree};
use tree_sitter::Node;

pub fn observe_python(source: &str) -> Result<Vec<SourceObservation>, String> {
    observe_tree(
        source,
        &tree_sitter_python::LANGUAGE.into(),
        "Python",
        collect_root,
    )
}

fn collect_root(node: Node<'_>, source: &[u8], observations: &mut Vec<SourceObservation>) {
    collect_observations(node, source, None, None, observations);
}

fn collect_observations(
    node: Node<'_>,
    source: &[u8],
    enclosing_class: Option<&str>,
    enclosing_symbol: Option<&str>,
    observations: &mut Vec<SourceObservation>,
) {
    let declared_class = if node.kind() == "class_definition" {
        node.child_by_field_name("name")
            .and_then(|name| name.utf8_text(source).ok())
            .map(|name| match enclosing_class {
                Some(parent) => format!("{parent}.{name}"),
                None => name.to_owned(),
            })
    } else {
        None
    };
    let class_name = declared_class.as_deref().or(enclosing_class);
    let declared_symbol = if node.kind() == "function_definition" {
        node.child_by_field_name("name")
            .and_then(|name| name.utf8_text(source).ok())
            .map(|name| match class_name {
                Some(class_name) => format!("{class_name}.{name}"),
                None => name.to_owned(),
            })
    } else if node.kind() == "lambda" {
        assigned_name(node, source).map(|name| match class_name {
            Some(class_name) => format!("{class_name}.{name}"),
            None => name,
        })
    } else {
        None
    };
    let class_body_symbol = declared_class
        .as_deref()
        .map(|name| format!("{name}.<body>"));
    let symbol = declared_symbol
        .as_deref()
        .or(class_body_symbol.as_deref())
        .or(enclosing_symbol);

    if node.kind() == "call"
        && let Some(observation) = observation_for_call(node, source, symbol)
    {
        observations.push(observation);
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_observations(child, source, class_name, symbol, observations);
    }
}

fn assigned_name(node: Node<'_>, source: &[u8]) -> Option<String> {
    let parent = node.parent()?;
    if parent.kind() != "assignment" {
        return None;
    }
    parent
        .child_by_field_name("left")
        .and_then(|left| crate::source_detection::canonical_node_text(left, source))
}

fn observation_for_call(
    call: Node<'_>,
    source: &[u8],
    symbol: Option<&str>,
) -> Option<SourceObservation> {
    let callable = call.child_by_field_name("function")?;
    if callable.kind() != "attribute" {
        return None;
    }
    let receiver = callable.child_by_field_name("object")?;
    let attribute = callable.child_by_field_name("attribute")?;
    observation_from_nodes("python", receiver, attribute, call, source, symbol)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source_detection::SourceObservationKind;

    #[test]
    fn observes_calls_with_physical_symbol_and_resource_identities() {
        let observations = observe_python(
            r#"
def place_order(order):
    orders.insert(order)
    order_events.publish(order)
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
                    resource: "order_events".to_owned(),
                    method: "publish".to_owned(),
                    line: 4,
                },
            ]
        );
    }

    #[test]
    fn ignores_method_names_inside_comments_and_strings() {
        let observations = observe_python(
            r#"
def place_order():
    text = "orders.insert(order)"
    # order_events.publish(order)
    return text
"#,
        )
        .unwrap();

        assert!(observations.is_empty());
    }

    #[test]
    fn retains_other_method_calls_for_bound_resource_coverage_checks() {
        let observations = observe_python(
            r#"
def place_order(order):
    orders.save(order)
"#,
        )
        .unwrap();

        assert_eq!(
            observations,
            vec![SourceObservation {
                kind: SourceObservationKind::OtherMethodCall,
                symbol: "place_order".to_owned(),
                resource: "orders".to_owned(),
                method: "save".to_owned(),
                line: 3,
            }]
        );
    }

    #[test]
    fn names_assigned_lambdas_and_class_body_calls() {
        let lambda = observe_python("place = lambda order: orders.insert(order)\n").unwrap();
        assert_eq!(lambda[0].symbol, "place");

        let class_body = observe_python("class Service:\n    orders.insert(None)\n").unwrap();
        assert_eq!(class_body[0].symbol, "Service.<body>");
    }

    #[test]
    fn rejects_invalid_python_instead_of_reporting_complete_coverage() {
        let error = observe_python("def place_order(:\n").unwrap_err();
        assert_eq!(error, "Python source contains a syntax error");
    }
}
