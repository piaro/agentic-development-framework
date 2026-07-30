//! Mechanical observations extracted from Python syntax.
//!
//! This module deliberately stops at physical source identities. Mapping a
//! function or receiver to a project-owned logical ID is handled by the Git
//! repository Adapter's reviewed Binding Record.

use tree_sitter::{Node, Parser};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum PythonObservationKind {
    DbWrite,
    MessagePublish,
    OtherMethodCall,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct PythonObservation {
    pub kind: PythonObservationKind,
    pub symbol: String,
    pub resource: String,
    pub method: String,
    pub line: usize,
}

pub fn observe_python(source: &str) -> Result<Vec<PythonObservation>, String> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_python::LANGUAGE.into())
        .map_err(|error| format!("cannot initialize Python parser: {error}"))?;
    let tree = parser
        .parse(source, None)
        .ok_or_else(|| "Python parser returned no syntax tree".to_owned())?;
    let root = tree.root_node();
    if root.has_error() {
        return Err("Python source contains a syntax error".to_owned());
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
    observations: &mut Vec<PythonObservation>,
) {
    let symbol = if node.kind() == "function_definition" {
        node.child_by_field_name("name")
            .and_then(|name| name.utf8_text(source).ok())
            .or(enclosing_symbol)
    } else {
        enclosing_symbol
    };

    if node.kind() == "call"
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
) -> Option<PythonObservation> {
    let callable = call.child_by_field_name("function")?;
    if callable.kind() != "attribute" {
        return None;
    }
    let receiver = callable.child_by_field_name("object")?;
    let attribute = callable.child_by_field_name("attribute")?;
    let method = attribute.utf8_text(source).ok()?;
    let kind = match method {
        "insert" | "update" | "delete" => PythonObservationKind::DbWrite,
        "publish" | "send_message" => PythonObservationKind::MessagePublish,
        _ => PythonObservationKind::OtherMethodCall,
    };
    Some(PythonObservation {
        kind,
        symbol: symbol.unwrap_or("<module>").to_owned(),
        resource: receiver.utf8_text(source).ok()?.to_owned(),
        method: method.to_owned(),
        line: call.start_position().row + 1,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

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
                PythonObservation {
                    kind: PythonObservationKind::DbWrite,
                    symbol: "place_order".to_owned(),
                    resource: "orders".to_owned(),
                    method: "insert".to_owned(),
                    line: 3,
                },
                PythonObservation {
                    kind: PythonObservationKind::MessagePublish,
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
            vec![PythonObservation {
                kind: PythonObservationKind::OtherMethodCall,
                symbol: "place_order".to_owned(),
                resource: "orders".to_owned(),
                method: "save".to_owned(),
                line: 3,
            }]
        );
    }

    #[test]
    fn rejects_invalid_python_instead_of_reporting_complete_coverage() {
        let error = observe_python("def place_order(:\n").unwrap_err();
        assert_eq!(error, "Python source contains a syntax error");
    }
}
