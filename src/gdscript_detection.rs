//! Mechanical observations extracted from Godot GDScript syntax.

use crate::source_detection::{
    SourceObservation, canonical_node_text, classify_method, observe_tree_with_parser_source,
};
use std::borrow::Cow;
use tree_sitter::Node;

pub fn observe_gdscript(source: &str) -> Result<Vec<SourceObservation>, String> {
    let parser_source = gdscript_parser_source(source);
    observe_tree_with_parser_source(
        source,
        &parser_source,
        &tree_sitter_gdscript::LANGUAGE.into(),
        "GDScript",
        collect_root,
    )
}

fn gdscript_parser_source(source: &str) -> Cow<'_, str> {
    // Godot accepts `$%UniqueNode` as get_node("%UniqueNode"), while the
    // bundled grammar currently accepts `$NodePath` and `%UniqueNode` only.
    // Replace the `%` for parsing with an identifier byte. Keeping the byte
    // length unchanged lets observations retain exact source locations and
    // the original `$%UniqueNode` receiver text.
    if !source.contains("$%") {
        return Cow::Borrowed(source);
    }

    let mut parser_source = source.as_bytes().to_vec();
    for index in 1..parser_source.len() {
        if parser_source[index - 1] == b'$' && parser_source[index] == b'%' {
            parser_source[index] = b'_';
        }
    }
    Cow::Owned(String::from_utf8(parser_source).expect("ASCII substitution preserves UTF-8"))
}

fn collect_root(node: Node<'_>, source: &[u8], observations: &mut Vec<SourceObservation>) {
    let script_class = find_script_class(node, source);
    let script_body_symbol = script_class
        .as_deref()
        .map(|class_name| format!("{class_name}.<body>"));
    collect_observations(
        node,
        source,
        script_class.as_deref(),
        script_body_symbol.as_deref(),
        observations,
    );
}

fn find_script_class(node: Node<'_>, source: &[u8]) -> Option<String> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor)
        .find(|child| child.kind() == "class_name_statement")
        .and_then(|statement| statement.child_by_field_name("name"))
        .and_then(|name| name.utf8_text(source).ok())
        .map(str::to_owned)
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
    let declared_symbol = match node.kind() {
        "function_definition" => node
            .child_by_field_name("name")
            .and_then(|name| name.utf8_text(source).ok())
            .map(|name| qualify_symbol(class_name, name)),
        "constructor_definition" => Some(qualify_symbol(class_name, "_init")),
        "lambda" => lambda_name(node, source).map(|name| qualify_symbol(class_name, &name)),
        "set_body" | "get_body" => {
            property_accessor_name(node, source).map(|name| qualify_symbol(class_name, &name))
        }
        _ => None,
    };
    let class_body_symbol = declared_class
        .as_deref()
        .map(|name| format!("{name}.<body>"));
    let symbol = declared_symbol
        .as_deref()
        .or(class_body_symbol.as_deref())
        .or(enclosing_symbol);

    if node.kind() == "attribute_call"
        && let Some(observation) = observation_for_attribute_call(node, source, symbol)
    {
        observations.push(observation);
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_observations(child, source, class_name, symbol, observations);
    }
}

fn qualify_symbol(class_name: Option<&str>, name: &str) -> String {
    match class_name {
        Some(class_name) => format!("{class_name}.{name}"),
        None => name.to_owned(),
    }
}

fn lambda_name(node: Node<'_>, source: &[u8]) -> Option<String> {
    if let Some(name) = node
        .child_by_field_name("name")
        .and_then(|name| name.utf8_text(source).ok())
    {
        return Some(name.to_owned());
    }
    let parent = node.parent()?;
    match parent.kind() {
        "variable_statement" | "export_variable_statement" | "onready_variable_statement" => parent
            .child_by_field_name("name")
            .and_then(|name| name.utf8_text(source).ok())
            .map(str::to_owned),
        "assignment" => parent
            .child_by_field_name("left")
            .and_then(|left| canonical_node_text(left, source)),
        _ => None,
    }
}

fn property_accessor_name(node: Node<'_>, source: &[u8]) -> Option<String> {
    let variable = node.parent()?.parent()?;
    if !matches!(
        variable.kind(),
        "variable_statement" | "export_variable_statement" | "onready_variable_statement"
    ) {
        return None;
    }
    let property = variable
        .child_by_field_name("name")?
        .utf8_text(source)
        .ok()?;
    let accessor = match node.kind() {
        "set_body" => "set",
        "get_body" => "get",
        _ => return None,
    };
    Some(format!("{property}.{accessor}"))
}

fn observation_for_attribute_call(
    call: Node<'_>,
    source: &[u8],
    symbol: Option<&str>,
) -> Option<SourceObservation> {
    let attribute = call.parent()?;
    if attribute.kind() != "attribute" {
        return None;
    }

    let mut receiver_parts = Vec::new();
    let mut cursor = attribute.walk();
    for child in attribute.named_children(&mut cursor) {
        if child.id() == call.id() {
            break;
        }
        if child.kind() != "line_continuation" {
            receiver_parts.push(canonical_gdscript_node(child, source)?);
        }
    }
    if receiver_parts.is_empty() {
        return None;
    }

    let method = call.named_child(0)?.utf8_text(source).ok()?.to_owned();
    Some(SourceObservation {
        kind: classify_method("gdscript", &method),
        symbol: symbol.unwrap_or("<module>").to_owned(),
        resource: receiver_parts.join("."),
        method,
        line: call.start_position().row + 1,
    })
}

fn canonical_gdscript_node(node: Node<'_>, source: &[u8]) -> Option<String> {
    if matches!(
        node.kind(),
        "string" | "string_name" | "node_path" | "get_node"
    ) {
        return node
            .utf8_text(source)
            .ok()
            .map(|text| text.replace("\\\r\n", "").replace("\\\n", ""));
    }
    if node.child_count() == 0 {
        return node.utf8_text(source).ok().map(str::to_owned);
    }

    let mut text = String::new();
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if !child.is_extra() && child.kind() != "line_continuation" {
            text.push_str(&canonical_gdscript_node(child, source)?);
        }
    }
    Some(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source_detection::SourceObservationKind;

    #[test]
    fn observes_script_class_functions_and_godot_signals() {
        let observations = observe_gdscript(
            r#"
class_name OrderService
extends Node

signal order_placed(order)

func place_order(order: Order) -> void:
    orders.insert(order)
    order_placed.emit(order)
    repository.save(order)
"#,
        )
        .unwrap();

        assert_eq!(
            observations,
            vec![
                SourceObservation {
                    kind: SourceObservationKind::DbWrite,
                    symbol: "OrderService.place_order".to_owned(),
                    resource: "orders".to_owned(),
                    method: "insert".to_owned(),
                    line: 8,
                },
                SourceObservation {
                    kind: SourceObservationKind::MessagePublish,
                    symbol: "OrderService.place_order".to_owned(),
                    resource: "order_placed".to_owned(),
                    method: "emit".to_owned(),
                    line: 9,
                },
                SourceObservation {
                    kind: SourceObservationKind::OtherMethodCall,
                    symbol: "OrderService.place_order".to_owned(),
                    resource: "repository".to_owned(),
                    method: "save".to_owned(),
                    line: 10,
                },
            ]
        );
    }

    #[test]
    fn observes_bound_and_object_signal_emission_forms() {
        let observations = observe_gdscript(
            r#"
func notify(player, order):
    player.order_placed.emit(order)
    self.emit_signal(&"order_placed", order)
"#,
        )
        .unwrap();

        assert_eq!(
            observations
                .iter()
                .map(|observation| {
                    (
                        observation.kind,
                        observation.resource.as_str(),
                        observation.method.as_str(),
                    )
                })
                .collect::<Vec<_>>(),
            vec![
                (
                    SourceObservationKind::MessagePublish,
                    "player.order_placed",
                    "emit",
                ),
                (SourceObservationKind::MessagePublish, "self", "emit_signal",),
            ]
        );
    }

    #[test]
    fn observes_inner_classes_constructors_node_paths_and_super_calls() {
        let observations = observe_gdscript(
            r#"
class_name OrderService

class Worker:
    func _init(order):
        $Orders.insert(order)
        %OrderEvents.publish(order)
        super.save(order)
"#,
        )
        .unwrap();

        assert_eq!(
            observations
                .iter()
                .map(|observation| {
                    (
                        observation.symbol.as_str(),
                        observation.resource.as_str(),
                        observation.method.as_str(),
                    )
                })
                .collect::<Vec<_>>(),
            vec![
                ("OrderService.Worker._init", "$Orders", "insert"),
                ("OrderService.Worker._init", "%OrderEvents", "publish"),
                ("OrderService.Worker._init", "super", "save"),
            ]
        );
    }

    #[test]
    fn normalizes_multiline_and_chained_receivers() {
        let observations = observe_gdscript(
            r#"
func place_order(order):
    client \
        .table("orders") \
        .insert(order)
"#,
        )
        .unwrap();

        assert!(
            observations.iter().any(|observation| {
                observation.resource == "client.table(\"orders\")" && observation.method == "insert"
            }),
            "{observations:?}"
        );
    }

    #[test]
    fn accepts_explicit_unique_node_shorthand_and_backslash_continuation() {
        let observations = observe_gdscript(
            r#"
@onready var button_sdfgi: CheckBox = $%SDFGI

func supported() -> bool:
    var display_server_name = DisplayServer.get_name()
    return display_server_name == &"Windows" \
            or display_server_name == &"macOS"

func remove_button() -> void:
    $%SDFGI.queue_free()
"#,
        )
        .unwrap();

        assert!(observations.iter().any(|observation| {
            observation.symbol == "remove_button"
                && observation.resource == "$%SDFGI"
                && observation.method == "queue_free"
        }));
    }

    #[test]
    fn names_assigned_lambdas() {
        let observations = observe_gdscript(
            r#"
class_name OrderService
var place_order = func(order):
    orders.insert(order)
"#,
        )
        .unwrap();

        assert_eq!(observations[0].symbol, "OrderService.place_order");
    }

    #[test]
    fn names_property_accessor_bodies() {
        let observations = observe_gdscript(
            r#"
class_name OrderService
var current_order:
    set(order):
        orders.update(order)
    get:
        audit.publish(current_order)
"#,
        )
        .unwrap();

        assert_eq!(
            observations
                .iter()
                .map(|observation| observation.symbol.as_str())
                .collect::<Vec<_>>(),
            vec![
                "OrderService.current_order.set",
                "OrderService.current_order.get",
            ]
        );
    }

    #[test]
    fn ignores_bare_calls_comments_and_strings() {
        let observations = observe_gdscript(
            r#"
func place_order(order):
    insert(order)
    # orders.insert(order)
    var text = "order_events.emit(order)"
"#,
        )
        .unwrap();
        assert!(observations.is_empty());
    }

    #[test]
    fn rejects_invalid_gdscript() {
        let error = observe_gdscript("func place_order(:\n").unwrap_err();
        assert_eq!(error, "GDScript source contains a syntax error");
    }
}
