//! Mechanical observations extracted from C# syntax.

use crate::source_detection::{SourceObservation, observation_from_nodes, observe_tree};
use tree_sitter::Node;

pub fn observe_csharp(source: &str) -> Result<Vec<SourceObservation>, String> {
    observe_tree(
        source,
        &tree_sitter_c_sharp::LANGUAGE.into(),
        "C#",
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
        "class_declaration" | "record_declaration" | "struct_declaration" | "interface_declaration"
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
        "method_declaration" | "constructor_declaration" | "local_function_statement"
    ) {
        node.child_by_field_name("name")
            .and_then(|name| name.utf8_text(source).ok())
            .map(|name| match (type_name, node.kind()) {
                (Some(type_name), "method_declaration" | "constructor_declaration") => {
                    format!("{type_name}.{name}")
                }
                _ => name.to_owned(),
            })
    } else {
        None
    };
    let symbol = declared_symbol.as_deref().or(enclosing_symbol);

    if node.kind() == "invocation_expression"
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
    let callable = invocation.child_by_field_name("function")?;
    if callable.kind() == "member_access_expression" {
        let receiver = callable.child_by_field_name("expression")?;
        let name = method_identifier(callable.child_by_field_name("name")?)?;
        return observation_from_nodes("csharp", receiver, name, invocation, source, symbol);
    }
    if callable.kind() == "conditional_access_expression" {
        let receiver = callable.child_by_field_name("condition")?;
        let mut cursor = callable.walk();
        let name = method_identifier(
            callable
                .named_children(&mut cursor)
                .find(|child| child.kind() == "member_binding_expression")?
                .child_by_field_name("name")?,
        )?;
        return observation_from_nodes("csharp", receiver, name, invocation, source, symbol);
    }
    None
}

fn method_identifier<'tree>(name: Node<'tree>) -> Option<Node<'tree>> {
    match name.kind() {
        "identifier" => Some(name),
        "generic_name" => name.named_child(0),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source_detection::SourceObservationKind;

    #[test]
    fn observes_csharp_methods_constructors_and_conditional_calls() {
        let observations = observe_csharp(
            r#"
sealed class OrderService
{
    public OrderService()
    {
        Audit.Save(this);
    }

    public void PlaceOrder(Order order)
    {
        Orders.Insert(order);
        Events?.Publish(order);
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
                    symbol: "OrderService.PlaceOrder".to_owned(),
                    resource: "Orders".to_owned(),
                    method: "Insert".to_owned(),
                    line: 11,
                },
                SourceObservation {
                    kind: SourceObservationKind::MessagePublish,
                    symbol: "OrderService.PlaceOrder".to_owned(),
                    resource: "Events".to_owned(),
                    method: "Publish".to_owned(),
                    line: 12,
                },
                SourceObservation {
                    kind: SourceObservationKind::OtherMethodCall,
                    symbol: "OrderService.OrderService".to_owned(),
                    resource: "Audit".to_owned(),
                    method: "Save".to_owned(),
                    line: 6,
                },
            ]
        );
    }

    #[test]
    fn retains_framework_specific_calls_for_reviewed_bindings() {
        let observations =
            observe_csharp("void PlaceOrder(Order order) { Repository.Save(order); }\n").unwrap();
        assert_eq!(
            observations,
            vec![SourceObservation {
                kind: SourceObservationKind::OtherMethodCall,
                symbol: "PlaceOrder".to_owned(),
                resource: "Repository".to_owned(),
                method: "Save".to_owned(),
                line: 1,
            }]
        );
    }

    #[test]
    fn observes_generic_calls_by_their_method_identifier() {
        let observations = observe_csharp(
            "void PlaceOrder(Order order) { Orders.Insert<Order>(order); Events?.Publish<Order>(order); }\n",
        )
        .unwrap();
        assert_eq!(observations.len(), 2);
        assert_eq!(observations[0].kind, SourceObservationKind::DbWrite);
        assert_eq!(observations[0].method, "Insert");
        assert_eq!(observations[1].kind, SourceObservationKind::MessagePublish);
        assert_eq!(observations[1].method, "Publish");
    }

    #[test]
    fn rejects_invalid_csharp() {
        let error = observe_csharp("class OrderService { void Save( }").unwrap_err();
        assert_eq!(error, "C# source contains a syntax error");
    }
}
