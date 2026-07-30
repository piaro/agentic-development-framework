//! Mechanical observations extracted from JavaScript and TypeScript syntax.

use crate::source_detection::{SourceObservation, classify_method};
use tree_sitter::{Language, Node, Parser};

pub fn observe_javascript(source: &str) -> Result<Vec<SourceObservation>, String> {
    observe_script(
        source,
        tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        "JavaScript",
    )
}

pub fn observe_jsx(source: &str) -> Result<Vec<SourceObservation>, String> {
    observe_script(source, tree_sitter_typescript::LANGUAGE_TSX.into(), "JSX")
}

pub fn observe_typescript(source: &str) -> Result<Vec<SourceObservation>, String> {
    observe_script(
        source,
        tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        "TypeScript",
    )
}

pub fn observe_tsx(source: &str) -> Result<Vec<SourceObservation>, String> {
    observe_script(source, tree_sitter_typescript::LANGUAGE_TSX.into(), "TSX")
}

fn observe_script(
    source: &str,
    language: Language,
    label: &str,
) -> Result<Vec<SourceObservation>, String> {
    let mut parser = Parser::new();
    parser
        .set_language(&language)
        .map_err(|error| format!("cannot initialize {label} parser: {error}"))?;
    let tree = parser
        .parse(source, None)
        .ok_or_else(|| format!("{label} parser returned no syntax tree"))?;
    let root = tree.root_node();
    if root.has_error() {
        return Err(format!("{label} source contains a syntax error"));
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
    let declared_symbol = symbol_for_node(node, source);
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

fn symbol_for_node(node: Node<'_>, source: &[u8]) -> Option<String> {
    if matches!(
        node.kind(),
        "function_declaration" | "generator_function_declaration" | "method_definition"
    ) {
        return node
            .child_by_field_name("name")
            .and_then(|name| name.utf8_text(source).ok())
            .map(str::to_owned);
    }
    if matches!(node.kind(), "arrow_function" | "function_expression")
        && let Some(parent) = node.parent()
        && parent.kind() == "variable_declarator"
    {
        return parent
            .child_by_field_name("name")
            .and_then(|name| name.utf8_text(source).ok())
            .map(str::to_owned);
    }
    None
}

fn observation_for_call(
    call: Node<'_>,
    source: &[u8],
    symbol: Option<&str>,
) -> Option<SourceObservation> {
    let callable = call.child_by_field_name("function")?;
    if callable.kind() != "member_expression" {
        return None;
    }
    let receiver = callable.child_by_field_name("object")?;
    let property = callable.child_by_field_name("property")?;
    let method = property.utf8_text(source).ok()?;
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
    fn observes_typescript_function_and_arrow_calls() {
        let observations = observe_typescript(
            r#"
export async function placeOrder(order: Order) {
  await orders.insert(order);
  events.publish(order);
}
const cancelOrder = (order: Order) => orders.delete(order);
"#,
        )
        .unwrap();

        assert_eq!(
            observations,
            vec![
                SourceObservation {
                    kind: SourceObservationKind::DbWrite,
                    symbol: "cancelOrder".to_owned(),
                    resource: "orders".to_owned(),
                    method: "delete".to_owned(),
                    line: 6,
                },
                SourceObservation {
                    kind: SourceObservationKind::DbWrite,
                    symbol: "placeOrder".to_owned(),
                    resource: "orders".to_owned(),
                    method: "insert".to_owned(),
                    line: 3,
                },
                SourceObservation {
                    kind: SourceObservationKind::MessagePublish,
                    symbol: "placeOrder".to_owned(),
                    resource: "events".to_owned(),
                    method: "publish".to_owned(),
                    line: 4,
                },
            ]
        );
    }

    #[test]
    fn parses_jsx_without_treating_text_as_calls() {
        let observations = observe_jsx(
            r#"
export function Button() {
  return <button title="events.publish(order)">Save</button>;
}
"#,
        )
        .unwrap();
        assert!(observations.is_empty());
    }

    #[test]
    fn rejects_invalid_typescript() {
        let error = observe_typescript("function placeOrder(: void {}").unwrap_err();
        assert_eq!(error, "TypeScript source contains a syntax error");
    }
}
