//! Mechanical observations extracted from Ruby syntax.

use crate::source_detection::{SourceObservation, observation_from_nodes, observe_tree};
use tree_sitter::Node;

pub fn observe_ruby(source: &str) -> Result<Vec<SourceObservation>, String> {
    observe_tree(
        source,
        &tree_sitter_ruby::LANGUAGE.into(),
        "Ruby",
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
    let declared_type = if matches!(node.kind(), "class" | "module") {
        node.child_by_field_name("name")
            .or_else(|| node.named_child(0))
            .and_then(|name| crate::source_detection::canonical_node_text(name, source))
            .map(|name| match enclosing_type {
                Some(parent) => format!("{parent}::{name}"),
                None => name,
            })
    } else {
        None
    };
    let type_name = declared_type.as_deref().or(enclosing_type);
    let declared_symbol = if matches!(node.kind(), "method" | "singleton_method") {
        node.child_by_field_name("name")
            .and_then(|name| name.utf8_text(source).ok())
            .map(|name| match (type_name, node.kind()) {
                (Some(type_name), "singleton_method") => format!("{type_name}.{name}"),
                (Some(type_name), _) => format!("{type_name}#{name}"),
                (None, _) => name.to_owned(),
            })
    } else {
        None
    };
    let symbol = declared_symbol.as_deref().or(enclosing_symbol);

    if node.kind() == "call"
        && let Some(observation) = observation_for_call(node, source, symbol)
    {
        observations.push(observation);
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_observations(child, source, type_name, symbol, observations);
    }
}

fn observation_for_call(
    call: Node<'_>,
    source: &[u8],
    symbol: Option<&str>,
) -> Option<SourceObservation> {
    let receiver = call.child_by_field_name("receiver")?;
    let name = call.child_by_field_name("method")?;
    observation_from_nodes("ruby", receiver, name, call, source, symbol)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source_detection::SourceObservationKind;

    #[test]
    fn observes_ruby_instance_and_singleton_method_calls() {
        let observations = observe_ruby(
            r#"
class OrderService
  def place_order(order)
    orders.insert(order)
    order_events.publish(order)
  end

  def self.cancel_order(order)
    orders.delete(order)
  end
end
"#,
        )
        .unwrap();

        assert_eq!(
            observations,
            vec![
                SourceObservation {
                    kind: SourceObservationKind::DbWrite,
                    symbol: "OrderService#place_order".to_owned(),
                    resource: "orders".to_owned(),
                    method: "insert".to_owned(),
                    line: 4,
                },
                SourceObservation {
                    kind: SourceObservationKind::DbWrite,
                    symbol: "OrderService.cancel_order".to_owned(),
                    resource: "orders".to_owned(),
                    method: "delete".to_owned(),
                    line: 9,
                },
                SourceObservation {
                    kind: SourceObservationKind::MessagePublish,
                    symbol: "OrderService#place_order".to_owned(),
                    resource: "order_events".to_owned(),
                    method: "publish".to_owned(),
                    line: 5,
                },
            ]
        );
    }

    #[test]
    fn retains_framework_specific_calls_for_reviewed_bindings() {
        let observations = observe_ruby(
            r#"
def place_order(order)
  repository.save(order)
end
"#,
        )
        .unwrap();
        assert_eq!(
            observations,
            vec![SourceObservation {
                kind: SourceObservationKind::OtherMethodCall,
                symbol: "place_order".to_owned(),
                resource: "repository".to_owned(),
                method: "save".to_owned(),
                line: 3,
            }]
        );
    }

    #[test]
    fn ignores_bare_calls_comments_and_strings() {
        let observations = observe_ruby(
            r#"
def place_order(order)
  insert(order)
  # orders.insert(order)
  "order_events.publish(order)"
end
"#,
        )
        .unwrap();
        assert!(observations.is_empty());
    }

    #[test]
    fn rejects_invalid_ruby() {
        let error = observe_ruby("def place_order(\n").unwrap_err();
        assert_eq!(error, "Ruby source contains a syntax error");
    }
}
