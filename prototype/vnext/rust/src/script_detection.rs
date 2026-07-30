//! Mechanical observations extracted from JavaScript and TypeScript syntax.

use crate::source_detection::{
    SourceObservation, canonical_node_text, observation_from_nodes, observe_tree,
};
use tree_sitter::Node;

pub fn observe_javascript(source: &str) -> Result<Vec<SourceObservation>, String> {
    observe_tree(
        source,
        &tree_sitter_typescript::LANGUAGE_TSX.into(),
        "JavaScript",
        collect_javascript_root,
    )
}

pub fn observe_jsx(source: &str) -> Result<Vec<SourceObservation>, String> {
    observe_tree(
        source,
        &tree_sitter_typescript::LANGUAGE_TSX.into(),
        "JSX",
        collect_jsx_root,
    )
}

pub fn observe_typescript(source: &str) -> Result<Vec<SourceObservation>, String> {
    observe_tree(
        source,
        &tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        "TypeScript",
        collect_typescript_root,
    )
}

pub fn observe_tsx(source: &str) -> Result<Vec<SourceObservation>, String> {
    observe_tree(
        source,
        &tree_sitter_typescript::LANGUAGE_TSX.into(),
        "TSX",
        collect_tsx_root,
    )
}

fn collect_javascript_root(
    node: Node<'_>,
    source: &[u8],
    observations: &mut Vec<SourceObservation>,
) {
    collect_observations(node, source, None, "javascript", observations);
}

fn collect_jsx_root(node: Node<'_>, source: &[u8], observations: &mut Vec<SourceObservation>) {
    collect_observations(node, source, None, "jsx", observations);
}

fn collect_typescript_root(
    node: Node<'_>,
    source: &[u8],
    observations: &mut Vec<SourceObservation>,
) {
    collect_observations(node, source, None, "typescript", observations);
}

fn collect_tsx_root(node: Node<'_>, source: &[u8], observations: &mut Vec<SourceObservation>) {
    collect_observations(node, source, None, "tsx", observations);
}

fn collect_observations(
    node: Node<'_>,
    source: &[u8],
    enclosing_symbol: Option<&str>,
    language: &str,
    observations: &mut Vec<SourceObservation>,
) {
    let declared_symbol = symbol_for_node(node, source);
    let symbol = declared_symbol.as_deref().or(enclosing_symbol);

    if node.kind() == "call_expression"
        && let Some(observation) = observation_for_call(node, source, symbol, language)
    {
        observations.push(observation);
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_observations(child, source, symbol, language, observations);
    }
}

fn symbol_for_node(node: Node<'_>, source: &[u8]) -> Option<String> {
    if matches!(
        node.kind(),
        "function_declaration" | "generator_function_declaration" | "method_definition"
    ) {
        let name = node
            .child_by_field_name("name")
            .and_then(|name| name.utf8_text(source).ok())
            .map(str::to_owned);
        if let Some(name) = name {
            return Some(qualify_class_member(node, &name, source));
        }
        if node.kind() == "function_declaration"
            && node
                .parent()
                .is_some_and(|parent| parent.kind() == "export_statement")
        {
            return Some("default".to_owned());
        }
    }
    if matches!(node.kind(), "arrow_function" | "function_expression")
        && let Some(parent) = node.parent()
    {
        if parent.kind() == "export_statement" {
            return Some("default".to_owned());
        }
        if parent.kind() == "variable_declarator" {
            return parent
                .child_by_field_name("name")
                .and_then(|name| name.utf8_text(source).ok())
                .map(str::to_owned);
        }
        if matches!(
            parent.kind(),
            "public_field_definition" | "field_definition"
        ) {
            let name = parent
                .child_by_field_name("name")
                .or_else(|| parent.child_by_field_name("property"))
                .and_then(|name| name.utf8_text(source).ok())?;
            return Some(qualify_class_member(parent, name, source));
        }
        if parent.kind() == "assignment_expression"
            && parent
                .child_by_field_name("right")
                .is_some_and(|right| right.id() == node.id())
        {
            return parent
                .child_by_field_name("left")
                .and_then(|left| canonical_node_text(left, source));
        }
    }
    None
}

fn qualify_class_member(node: Node<'_>, name: &str, source: &[u8]) -> String {
    let mut ancestor = node.parent();
    while let Some(candidate) = ancestor {
        if matches!(candidate.kind(), "class_declaration" | "class")
            && let Some(class_name) = candidate
                .child_by_field_name("name")
                .and_then(|name| name.utf8_text(source).ok())
        {
            return format!("{class_name}.{name}");
        }
        ancestor = candidate.parent();
    }
    name.to_owned()
}

fn observation_for_call(
    call: Node<'_>,
    source: &[u8],
    symbol: Option<&str>,
    language: &str,
) -> Option<SourceObservation> {
    let callable = call.child_by_field_name("function")?;
    if callable.kind() == "member_expression" {
        let receiver = callable.child_by_field_name("object")?;
        let property = callable.child_by_field_name("property")?;
        return observation_from_nodes(language, receiver, property, call, source, symbol);
    }
    if callable.kind() == "subscript_expression" {
        let receiver = callable.child_by_field_name("object")?;
        let index = callable.child_by_field_name("index")?;
        let index_text = canonical_node_text(index, source)?;
        let method = quoted_property_name(&index_text).unwrap_or_else(|| format!("[{index_text}]"));
        return Some(SourceObservation {
            kind: crate::source_detection::classify_method(language, &method),
            symbol: symbol.unwrap_or("<module>").to_owned(),
            resource: canonical_node_text(receiver, source)?,
            method,
            line: call.start_position().row + 1,
        });
    }
    None
}

fn quoted_property_name(text: &str) -> Option<String> {
    let quote = text.as_bytes().first().copied()?;
    if !matches!(quote, b'\'' | b'"') || text.as_bytes().last().copied()? != quote {
        return None;
    }
    let name = &text[1..text.len().checked_sub(1)?];
    if name.is_empty() || name.contains('\\') {
        return None;
    }
    Some(name.to_owned())
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
    fn javascript_accepts_jsx_and_computed_string_calls() {
        let observations = observe_javascript(
            r#"
export function Button(order) {
  orders["insert"](order);
  return <button>Save</button>;
}
"#,
        )
        .unwrap();
        assert_eq!(observations.len(), 1);
        assert_eq!(observations[0].method, "insert");
        assert_eq!(observations[0].resource, "orders");
    }

    #[test]
    fn retains_dynamic_computed_calls_for_coverage_checks() {
        let observations =
            observe_typescript("function place(key: string) { orders[key](1); }").unwrap();
        assert_eq!(observations.len(), 1);
        assert_eq!(observations[0].resource, "orders");
        assert_eq!(observations[0].method, "[key]");
        assert_eq!(observations[0].kind, SourceObservationKind::OtherMethodCall);
    }

    #[test]
    fn names_common_assigned_and_default_function_forms() {
        let class_field =
            observe_typescript("class Service { place = (order: Order) => orders.insert(order); }")
                .unwrap();
        assert_eq!(class_field[0].symbol, "Service.place");

        let default_export =
            observe_javascript("export default function (order) { orders.insert(order); }")
                .unwrap();
        assert_eq!(default_export[0].symbol, "default");

        let common_js =
            observe_javascript("exports.place = function (order) { orders.insert(order); };")
                .unwrap();
        assert_eq!(common_js[0].symbol, "exports.place");
    }

    #[test]
    fn rejects_invalid_typescript() {
        let error = observe_typescript("function placeOrder(: void {}").unwrap_err();
        assert_eq!(error, "TypeScript source contains a syntax error");
    }
}
