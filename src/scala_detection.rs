//! Mechanical observations extracted from Scala syntax.

use crate::source_detection::{
    SourceObservation, canonical_node_text, observation_from_nodes, observe_tree,
};
use tree_sitter::Node;

pub fn observe_scala(source: &str) -> Result<Vec<SourceObservation>, String> {
    observe_tree(
        source,
        &tree_sitter_scala::LANGUAGE.into(),
        "Scala",
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
    let declared_type = declared_type(node, source, enclosing_type);
    let type_name = declared_type.as_deref().or(enclosing_type);
    let declared_symbol = declared_symbol(node, source, type_name, enclosing_symbol);
    let symbol = declared_symbol.as_deref().or(enclosing_symbol);

    let observation = match node.kind() {
        "call_expression" => observation_for_call(node, source, symbol),
        "infix_expression" => observation_for_infix(node, source, symbol),
        "postfix_expression" => observation_for_postfix(node, source, symbol),
        _ => None,
    };
    if let Some(observation) = observation {
        observations.push(observation);
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_observations(child, source, type_name, symbol, observations);
    }
}

fn declared_type(node: Node<'_>, source: &[u8], enclosing_type: Option<&str>) -> Option<String> {
    let name = match node.kind() {
        "class_definition" | "object_definition" | "trait_definition" | "enum_definition" => node
            .child_by_field_name("name")
            .and_then(|name| canonical_node_text(name, source)),
        "extension_definition" => {
            extension_receiver_type(node, source).map(|receiver| format!("extension[{receiver}]"))
        }
        _ => None,
    }?;
    Some(match enclosing_type {
        Some(parent) => format!("{parent}.{name}"),
        None => name,
    })
}

fn declared_symbol(
    node: Node<'_>,
    source: &[u8],
    type_name: Option<&str>,
    enclosing_symbol: Option<&str>,
) -> Option<String> {
    let name = match node.kind() {
        "function_definition" => node
            .child_by_field_name("name")
            .and_then(|name| canonical_node_text(name, source)),
        "given_definition" => given_name(node, source),
        "val_definition" | "var_definition" if is_type_or_global_value(node) => node
            .child_by_field_name("pattern")
            .filter(|pattern| pattern.kind() == "identifier")
            .and_then(|name| canonical_node_text(name, source)),
        "class_definition" | "object_definition" | "trait_definition" | "enum_definition" => {
            Some("<init>".to_owned())
        }
        _ => None,
    }?;
    match (type_name, enclosing_symbol, node.kind()) {
        (_, Some(parent), "function_definition") if !parent.ends_with(".<init>") => {
            Some(format!("{parent}.{name}"))
        }
        (Some(type_name), _, _) => Some(format!("{type_name}.{name}")),
        (None, _, _) => Some(name),
    }
}

fn extension_receiver_type(node: Node<'_>, source: &[u8]) -> Option<String> {
    let parameters = node.child_by_field_name("parameters")?;
    let parameter = first_named_descendant(parameters, "parameter")?;
    parameter
        .child_by_field_name("type")
        .and_then(|kind| canonical_node_text(kind, source))
}

fn given_name(node: Node<'_>, source: &[u8]) -> Option<String> {
    if let Some(name) = node.child_by_field_name("name") {
        return canonical_node_text(name, source);
    }
    node.child_by_field_name("return_type")
        .and_then(|kind| canonical_node_text(kind, source))
        .map(|kind| format!("given[{kind}]"))
}

fn is_type_or_global_value(node: Node<'_>) -> bool {
    node.parent().is_some_and(|parent| {
        matches!(
            parent.kind(),
            "compilation_unit" | "template_body" | "enum_body"
        )
    })
}

fn observation_for_call(
    call: Node<'_>,
    source: &[u8],
    symbol: Option<&str>,
) -> Option<SourceObservation> {
    let mut callable = call.child_by_field_name("function")?;
    if callable.kind() == "generic_function" {
        callable = callable.child_by_field_name("function")?;
    }
    if callable.kind() != "field_expression" {
        return None;
    }
    let receiver = callable.child_by_field_name("value")?;
    let method = callable.child_by_field_name("field")?;
    observation_from_nodes("scala", receiver, method, call, source, symbol)
}

fn observation_for_infix(
    expression: Node<'_>,
    source: &[u8],
    symbol: Option<&str>,
) -> Option<SourceObservation> {
    let method = expression.child_by_field_name("operator")?;
    if method.kind() != "identifier" {
        return None;
    }
    let receiver = expression.child_by_field_name("left")?;
    observation_from_nodes("scala", receiver, method, expression, source, symbol)
}

fn observation_for_postfix(
    expression: Node<'_>,
    source: &[u8],
    symbol: Option<&str>,
) -> Option<SourceObservation> {
    let receiver = expression.named_child(0)?;
    let method = expression.named_child(1)?;
    if method.kind() != "identifier" {
        return None;
    }
    observation_from_nodes("scala", receiver, method, expression, source, symbol)
}

fn first_named_descendant<'tree>(node: Node<'tree>, kind: &str) -> Option<Node<'tree>> {
    if node.kind() == kind {
        return Some(node);
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if let Some(found) = first_named_descendant(child, kind) {
            return Some(found);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source_detection::SourceObservationKind;

    #[test]
    fn observes_scala_two_methods_and_type_initialization() {
        let observations = observe_scala(
            r#"
final class OrderService {
  audit.save(this)

  def placeOrder(order: Order): Unit = {
    orders.insert(order)
    events.publish(order)
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
                    line: 6,
                },
                SourceObservation {
                    kind: SourceObservationKind::MessagePublish,
                    symbol: "OrderService.placeOrder".to_owned(),
                    resource: "events".to_owned(),
                    method: "publish".to_owned(),
                    line: 7,
                },
                SourceObservation {
                    kind: SourceObservationKind::OtherMethodCall,
                    symbol: "OrderService.<init>".to_owned(),
                    resource: "audit".to_owned(),
                    method: "save".to_owned(),
                    line: 3,
                },
            ]
        );
    }

    #[test]
    fn observes_scala_three_extensions_generic_and_infix_calls() {
        let observations = observe_scala(
            r#"
object OrderHandlers:
  val persist: Order => Unit = order =>
    repository.save(order)

  extension (orders: Orders)
    def add(order: Order): Unit =
      orders.insert[Order](order)

  def publish(order: Order): Unit =
    events publish order
"#,
        )
        .unwrap();

        assert_eq!(observations.len(), 3);
        assert!(observations.iter().any(|observation| {
            observation.kind == SourceObservationKind::DbWrite
                && observation.symbol == "OrderHandlers.extension[Orders].add"
                && observation.resource == "orders"
                && observation.method == "insert"
        }));
        assert!(observations.iter().any(|observation| {
            observation.kind == SourceObservationKind::MessagePublish
                && observation.symbol == "OrderHandlers.publish"
                && observation.resource == "events"
                && observation.method == "publish"
        }));
        assert!(observations.iter().any(|observation| {
            observation.kind == SourceObservationKind::OtherMethodCall
                && observation.symbol == "OrderHandlers.persist"
                && observation.resource == "repository"
                && observation.method == "save"
        }));
    }

    #[test]
    fn names_anonymous_given_definitions_by_their_return_type() {
        let observations = observe_scala(
            r#"
object Bindings:
  given OrderHandler = repository.save(order)
"#,
        )
        .unwrap();
        assert_eq!(
            observations,
            vec![SourceObservation {
                kind: SourceObservationKind::OtherMethodCall,
                symbol: "Bindings.given[OrderHandler]".to_owned(),
                resource: "repository".to_owned(),
                method: "save".to_owned(),
                line: 3,
            }]
        );
    }

    #[test]
    fn observes_named_postfix_method_calls() {
        let observations = observe_scala("def flush(): Unit = repository save\n").unwrap();
        assert_eq!(
            observations,
            vec![SourceObservation {
                kind: SourceObservationKind::OtherMethodCall,
                symbol: "flush".to_owned(),
                resource: "repository".to_owned(),
                method: "save".to_owned(),
                line: 1,
            }]
        );
    }

    #[test]
    fn rejects_invalid_scala() {
        let error = observe_scala("def placeOrder( = {").unwrap_err();
        assert_eq!(error, "Scala source contains a syntax error");
    }
}
