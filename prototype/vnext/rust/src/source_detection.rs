//! Registry and language-neutral output for source-code detectors.
//!
//! The registry is the single mechanical boundary for adding a language. A
//! language is discoverable independently from whether a detector exists, so
//! project inventory can report unsupported source instead of silently
//! excluding it.

use crate::csharp_detection::observe_csharp;
use crate::go_detection::observe_go;
use crate::java_detection::observe_java;
use crate::kotlin_detection::observe_kotlin;
use crate::php_detection::observe_php;
use crate::python_detection::observe_python;
use crate::ruby_detection::observe_ruby;
use crate::rust_detection::observe_rust;
use crate::script_detection::{observe_javascript, observe_jsx, observe_tsx, observe_typescript};
use std::path::Path;
use tree_sitter::{Language, Node, Parser};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SourceObservationKind {
    DbWrite,
    MessagePublish,
    OtherMethodCall,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct SourceObservation {
    pub kind: SourceObservationKind,
    pub symbol: String,
    pub resource: String,
    pub method: String,
    pub line: usize,
}

pub fn classify_method(language: &str, method: &str) -> SourceObservationKind {
    // Only language-native spellings of the deliberately small built-in
    // vocabulary are classified here. Framework-specific or ambiguous names
    // (for example Go's `Send` or Rails' `save`) require a reviewed method
    // Binding Record.
    let (db_write, message_publish): (&[&str], &[&str]) = match language {
        "go" | "csharp" => (&["Insert", "Update", "Delete"], &["Publish", "SendMessage"]),
        "java" | "kotlin" | "php" | "javascript" | "jsx" | "typescript" | "tsx" => {
            (&["insert", "update", "delete"], &["publish", "sendMessage"])
        }
        "python" | "ruby" | "rust" => (
            &["insert", "update", "delete"],
            &["publish", "send_message"],
        ),
        _ => (&[], &[]),
    };
    if db_write.contains(&method) {
        SourceObservationKind::DbWrite
    } else if message_publish.contains(&method) {
        SourceObservationKind::MessagePublish
    } else {
        SourceObservationKind::OtherMethodCall
    }
}

type ObserveSource = fn(&str) -> Result<Vec<SourceObservation>, String>;
pub(crate) type CollectObservations =
    for<'tree> fn(Node<'tree>, &'tree [u8], &mut Vec<SourceObservation>);

pub(crate) fn observe_tree(
    source: &str,
    language: &Language,
    label: &str,
    collect: CollectObservations,
) -> Result<Vec<SourceObservation>, String> {
    let mut parser = Parser::new();
    parser
        .set_language(language)
        .map_err(|error| format!("cannot initialize {label} parser: {error}"))?;
    let tree = parser
        .parse(source, None)
        .ok_or_else(|| format!("{label} parser returned no syntax tree"))?;
    let root = tree.root_node();
    if root.has_error() {
        return Err(format!("{label} source contains a syntax error"));
    }

    let mut observations = Vec::new();
    collect(root, source.as_bytes(), &mut observations);
    // Keep equal observations: two calls can share a line, symbol, receiver,
    // and method while still representing two distinct source operations.
    observations.sort();
    Ok(observations)
}

pub(crate) fn observation_from_nodes(
    language: &str,
    receiver: Node<'_>,
    method_node: Node<'_>,
    call: Node<'_>,
    source: &[u8],
    symbol: Option<&str>,
) -> Option<SourceObservation> {
    let method = method_node.utf8_text(source).ok()?;
    Some(SourceObservation {
        kind: classify_method(language, method),
        symbol: symbol.unwrap_or("<module>").to_owned(),
        resource: canonical_node_text(receiver, source)?,
        method: method.to_owned(),
        line: call.start_position().row + 1,
    })
}

pub(crate) fn canonical_node_text(node: Node<'_>, source: &[u8]) -> Option<String> {
    if node.child_count() == 0 {
        return node.utf8_text(source).ok().map(str::to_owned);
    }
    let mut text = String::new();
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if !child.is_extra() {
            text.push_str(&canonical_node_text(child, source)?);
        }
    }
    Some(text)
}

pub struct LanguageDetector {
    pub language: &'static str,
    pub extensions: &'static [&'static str],
    observe: Option<ObserveSource>,
}

impl LanguageDetector {
    pub fn is_supported(&self) -> bool {
        self.observe.is_some()
    }

    pub fn observe(&self, source: &str) -> Result<Vec<SourceObservation>, String> {
        self.observe
            .ok_or_else(|| format!("language {} is not supported", self.language))?(source)
    }
}

static LANGUAGE_DETECTORS: &[LanguageDetector] = &[
    LanguageDetector {
        language: "python",
        extensions: &["py"],
        observe: Some(observe_python),
    },
    LanguageDetector {
        language: "javascript",
        extensions: &["js", "mjs", "cjs"],
        observe: Some(observe_javascript),
    },
    LanguageDetector {
        language: "jsx",
        extensions: &["jsx"],
        observe: Some(observe_jsx),
    },
    LanguageDetector {
        language: "typescript",
        extensions: &["ts", "mts", "cts"],
        observe: Some(observe_typescript),
    },
    LanguageDetector {
        language: "tsx",
        extensions: &["tsx"],
        observe: Some(observe_tsx),
    },
    LanguageDetector {
        language: "java",
        extensions: &["java"],
        observe: Some(observe_java),
    },
    LanguageDetector {
        language: "kotlin",
        extensions: &["kt", "kts"],
        observe: Some(observe_kotlin),
    },
    LanguageDetector {
        language: "go",
        extensions: &["go"],
        observe: Some(observe_go),
    },
    LanguageDetector {
        language: "rust",
        extensions: &["rs"],
        observe: Some(observe_rust),
    },
    LanguageDetector {
        language: "ruby",
        extensions: &["rb"],
        observe: Some(observe_ruby),
    },
    LanguageDetector {
        language: "php",
        extensions: &["php"],
        observe: Some(observe_php),
    },
    LanguageDetector {
        language: "csharp",
        extensions: &["cs"],
        observe: Some(observe_csharp),
    },
    LanguageDetector {
        language: "swift",
        extensions: &["swift"],
        observe: None,
    },
    LanguageDetector {
        language: "scala",
        extensions: &["scala", "sc"],
        observe: None,
    },
    LanguageDetector {
        language: "c",
        extensions: &["c", "h"],
        observe: None,
    },
    LanguageDetector {
        language: "cpp",
        extensions: &["cc", "cpp", "cxx", "hh", "hpp", "hxx"],
        observe: None,
    },
];

pub fn detector_for_language(language: &str) -> Option<&'static LanguageDetector> {
    LANGUAGE_DETECTORS
        .iter()
        .find(|detector| detector.language == language)
}

pub fn detector_for_path(path: &str) -> Option<&'static LanguageDetector> {
    let extension = Path::new(path).extension()?.to_str()?;
    LANGUAGE_DETECTORS.iter().find(|detector| {
        detector
            .extensions
            .iter()
            .any(|candidate| candidate.eq_ignore_ascii_case(extension))
    })
}

pub fn source_pathspecs() -> Vec<String> {
    LANGUAGE_DETECTORS
        .iter()
        .flat_map(|detector| detector.extensions)
        .map(|extension| format!(":(icase)*.{extension}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_separates_supported_detectors_from_inventory_only_languages() {
        assert!(detector_for_language("typescript").unwrap().is_supported());
        assert!(detector_for_language("java").unwrap().is_supported());
        assert!(detector_for_language("go").unwrap().is_supported());
        assert!(detector_for_language("rust").unwrap().is_supported());
        assert!(detector_for_language("kotlin").unwrap().is_supported());
        assert!(detector_for_language("ruby").unwrap().is_supported());
        assert!(detector_for_language("php").unwrap().is_supported());
        assert!(detector_for_language("csharp").unwrap().is_supported());
        assert!(!detector_for_language("swift").unwrap().is_supported());
        assert_eq!(
            detector_for_path("src/example.tsx").unwrap().language,
            "tsx"
        );
        assert_eq!(
            detector_for_path("src/example.java").unwrap().language,
            "java"
        );
        assert_eq!(detector_for_path("src/Main.JAVA").unwrap().language, "java");
        assert!(detector_for_path("README.md").is_none());
    }

    #[test]
    fn keeps_repeated_calls_on_the_same_line() {
        let observations = detector_for_language("python")
            .unwrap()
            .observe("def place(order):\n    orders.insert(order); orders.insert(order)\n")
            .unwrap();
        assert_eq!(observations.len(), 2);
    }

    #[test]
    fn built_in_method_names_follow_each_language_convention() {
        assert_eq!(
            classify_method("python", "send_message"),
            SourceObservationKind::MessagePublish
        );
        assert_eq!(
            classify_method("java", "send_message"),
            SourceObservationKind::OtherMethodCall
        );
        assert_eq!(
            classify_method("java", "sendMessage"),
            SourceObservationKind::MessagePublish
        );
        assert_eq!(
            classify_method("go", "Send"),
            SourceObservationKind::OtherMethodCall
        );
        assert_eq!(
            classify_method("go", "SendMessage"),
            SourceObservationKind::MessagePublish
        );
    }
}

#[cfg(test)]
mod conformance {
    //! Behavior that every language detector must share.
    //!
    //! Each language module asserts how it reads its own syntax. These tests
    //! pin what the Binding Record and the coverage gaps depend on: the same
    //! operation is classified the same way in every language, text that merely
    //! looks like a call is never reported, and source the grammar cannot read
    //! fails instead of reporting an empty result.

    use super::*;
    use std::collections::BTreeMap;

    /// One operation written once per language.
    ///
    /// Receiver names are deliberately identical in every fixture, including
    /// where they read unnaturally for the language, so a failing assertion
    /// points at the detector instead of at a naming convention.
    struct Fixture {
        language: &'static str,
        /// A single named function that writes, publishes, and calls one method
        /// that only a reviewed Binding Record can classify.
        operation: &'static str,
        /// Name the detector must report for the enclosing function.
        symbol: &'static str,
        write_method: &'static str,
        publish_method: &'static str,
        unclassified_method: &'static str,
        /// The same three calls written only inside a comment and a string.
        text_only: &'static str,
        /// A call without a receiver, so it carries no resource identity.
        receiverless_call: &'static str,
        /// Source the grammar cannot read.
        unparseable: &'static str,
        /// Message the detector must report for `unparseable`.
        parse_error: &'static str,
    }

    const FIXTURES: &[Fixture] = &[
        Fixture {
            language: "python",
            operation: r#"
def place_order(order):
    orders.insert(order)
    orderEvents.publish(order)
    repository.save(order)
"#,
            symbol: "place_order",
            write_method: "insert",
            publish_method: "publish",
            unclassified_method: "save",
            text_only: r#"
def place_order(order):
    # orders.insert(order)
    return "orderEvents.publish(order) repository.save(order)"
"#,
            receiverless_call: r#"
def place_order(order):
    insert(order)
"#,
            unparseable: "def place_order(:\n",
            parse_error: "Python source contains a syntax error",
        },
        Fixture {
            language: "javascript",
            operation: r#"
function placeOrder(order) {
  orders.insert(order);
  orderEvents.publish(order);
  repository.save(order);
}
"#,
            symbol: "placeOrder",
            write_method: "insert",
            publish_method: "publish",
            unclassified_method: "save",
            text_only: r#"
function placeOrder(order) {
  // orders.insert(order)
  return "orderEvents.publish(order) repository.save(order)";
}
"#,
            receiverless_call: r#"
function placeOrder(order) {
  insert(order);
}
"#,
            unparseable: "function placeOrder(] {",
            parse_error: "JavaScript source contains a syntax error",
        },
        Fixture {
            language: "jsx",
            operation: r#"
function placeOrder(order) {
  orders.insert(order);
  orderEvents.publish(order);
  repository.save(order);
  return <form name="placeOrder" />;
}
"#,
            symbol: "placeOrder",
            write_method: "insert",
            publish_method: "publish",
            unclassified_method: "save",
            text_only: r#"
function placeOrder(order) {
  // orders.insert(order)
  return <form title="orderEvents.publish(order) repository.save(order)" />;
}
"#,
            receiverless_call: r#"
function placeOrder(order) {
  insert(order);
  return <form />;
}
"#,
            unparseable: "function placeOrder(] {",
            parse_error: "JSX source contains a syntax error",
        },
        Fixture {
            language: "typescript",
            operation: r#"
export function placeOrder(order: Order): void {
  orders.insert(order);
  orderEvents.publish(order);
  repository.save(order);
}
"#,
            symbol: "placeOrder",
            write_method: "insert",
            publish_method: "publish",
            unclassified_method: "save",
            text_only: r#"
export function placeOrder(order: Order): string {
  // orders.insert(order)
  return "orderEvents.publish(order) repository.save(order)";
}
"#,
            receiverless_call: r#"
export function placeOrder(order: Order): void {
  insert(order);
}
"#,
            unparseable: "function placeOrder(: void {}",
            parse_error: "TypeScript source contains a syntax error",
        },
        Fixture {
            language: "tsx",
            operation: r#"
export function placeOrder(order: Order): JSX.Element {
  orders.insert(order);
  orderEvents.publish(order);
  repository.save(order);
  return <form name="placeOrder" />;
}
"#,
            symbol: "placeOrder",
            write_method: "insert",
            publish_method: "publish",
            unclassified_method: "save",
            text_only: r#"
export function placeOrder(order: Order): JSX.Element {
  // orders.insert(order)
  return <form title="orderEvents.publish(order) repository.save(order)" />;
}
"#,
            receiverless_call: r#"
export function placeOrder(order: Order): void {
  insert(order);
}
"#,
            unparseable: "function placeOrder(: void {}",
            parse_error: "TSX source contains a syntax error",
        },
        Fixture {
            language: "java",
            operation: r#"
final class OrderService {
    void placeOrder(Order order) {
        orders.insert(order);
        orderEvents.publish(order);
        repository.save(order);
    }
}
"#,
            symbol: "OrderService.placeOrder",
            write_method: "insert",
            publish_method: "publish",
            unclassified_method: "save",
            text_only: r#"
final class OrderService {
    String placeOrder(Order order) {
        // orders.insert(order);
        return "orderEvents.publish(order) repository.save(order)";
    }
}
"#,
            receiverless_call: r#"
final class OrderService {
    void placeOrder(Order order) {
        insert(order);
    }
}
"#,
            unparseable: "class OrderService { void save( }",
            parse_error: "Java source contains a syntax error",
        },
        Fixture {
            language: "kotlin",
            operation: r#"
fun placeOrder(order: Order) {
    orders.insert(order)
    orderEvents.publish(order)
    repository.save(order)
}
"#,
            symbol: "placeOrder",
            write_method: "insert",
            publish_method: "publish",
            unclassified_method: "save",
            text_only: r#"
fun placeOrder(order: Order): String {
    // orders.insert(order)
    return "orderEvents.publish(order) repository.save(order)"
}
"#,
            receiverless_call: r#"
fun placeOrder(order: Order) {
    insert(order)
}
"#,
            unparseable: "fun placeOrder( {",
            parse_error: "Kotlin source contains a syntax error",
        },
        Fixture {
            language: "go",
            operation: r#"
package ordering

func PlaceOrder(order Order) {
    orders.Insert(order)
    orderEvents.Publish(order)
    repository.Save(order)
}
"#,
            symbol: "PlaceOrder",
            write_method: "Insert",
            publish_method: "Publish",
            unclassified_method: "Save",
            text_only: r#"
package ordering

func PlaceOrder(order Order) string {
    // orders.Insert(order)
    return "orderEvents.Publish(order) repository.Save(order)"
}
"#,
            receiverless_call: r#"
package ordering

func PlaceOrder(order Order) {
    insert(order)
}
"#,
            unparseable: "package ordering\nfunc PlaceOrder( {",
            parse_error: "Go source contains a syntax error",
        },
        Fixture {
            language: "rust",
            operation: r#"
fn place_order(order: Order) {
    orders.insert(order);
    orderEvents.publish(order);
    repository.save(order);
}
"#,
            symbol: "place_order",
            write_method: "insert",
            publish_method: "publish",
            unclassified_method: "save",
            text_only: r#"
fn place_order(order: Order) -> &'static str {
    // orders.insert(order);
    "orderEvents.publish(order) repository.save(order)"
}
"#,
            receiverless_call: r#"
fn place_order(order: Order) {
    insert(order);
}
"#,
            unparseable: "fn place_order( {",
            parse_error: "Rust source contains a syntax error",
        },
        Fixture {
            language: "ruby",
            operation: r#"
def place_order(order)
  orders.insert(order)
  orderEvents.publish(order)
  repository.save(order)
end
"#,
            symbol: "place_order",
            write_method: "insert",
            publish_method: "publish",
            unclassified_method: "save",
            text_only: r#"
def place_order(order)
  # orders.insert(order)
  "orderEvents.publish(order) repository.save(order)"
end
"#,
            receiverless_call: r#"
def place_order(order)
  insert(order)
end
"#,
            unparseable: "def place_order(\n",
            parse_error: "Ruby source contains a syntax error",
        },
        Fixture {
            language: "php",
            operation: r#"<?php
function placeOrder($order) {
    orders::insert($order);
    orderEvents::publish($order);
    repository::save($order);
}
"#,
            symbol: "placeOrder",
            write_method: "insert",
            publish_method: "publish",
            unclassified_method: "save",
            text_only: r#"<?php
function placeOrder($order) {
    // orders::insert($order);
    return "orderEvents::publish repository::save";
}
"#,
            receiverless_call: r#"<?php
function placeOrder($order) {
    insert($order);
}
"#,
            unparseable: "<?php\nfunction placeOrder( {\n",
            parse_error: "PHP source contains a syntax error",
        },
        Fixture {
            language: "csharp",
            operation: r#"
sealed class OrderService
{
    void PlaceOrder(Order order)
    {
        orders.Insert(order);
        orderEvents.Publish(order);
        repository.Save(order);
    }
}
"#,
            symbol: "OrderService.PlaceOrder",
            write_method: "Insert",
            publish_method: "Publish",
            unclassified_method: "Save",
            text_only: r#"
sealed class OrderService
{
    string PlaceOrder(Order order)
    {
        // orders.Insert(order);
        return "orderEvents.Publish(order) repository.Save(order)";
    }
}
"#,
            receiverless_call: r#"
sealed class OrderService
{
    void PlaceOrder(Order order)
    {
        Insert(order);
    }
}
"#,
            unparseable: "class OrderService { void PlaceOrder( }",
            parse_error: "C# source contains a syntax error",
        },
    ];

    fn detector(language: &str) -> &'static LanguageDetector {
        detector_for_language(language)
            .unwrap_or_else(|| panic!("{language} is missing from the registry"))
    }

    fn observe(fixture: &Fixture, source: &str) -> Vec<SourceObservation> {
        detector(fixture.language)
            .observe(source)
            .unwrap_or_else(|error| panic!("{}: {error}", fixture.language))
    }

    #[test]
    fn every_language_classifies_the_same_operation_the_same_way() {
        for fixture in FIXTURES {
            let observations = observe(fixture, fixture.operation);
            let mut expected = vec![
                (
                    SourceObservationKind::DbWrite,
                    "orders",
                    fixture.write_method,
                ),
                (
                    SourceObservationKind::MessagePublish,
                    "orderEvents",
                    fixture.publish_method,
                ),
                (
                    SourceObservationKind::OtherMethodCall,
                    "repository",
                    fixture.unclassified_method,
                ),
            ];
            expected.sort();
            let mut actual: Vec<_> = observations
                .iter()
                .map(|observation| {
                    (
                        observation.kind,
                        observation.resource.as_str(),
                        observation.method.as_str(),
                    )
                })
                .collect();
            actual.sort();
            assert_eq!(actual, expected, "{}", fixture.language);
            for observation in &observations {
                assert_eq!(observation.symbol, fixture.symbol, "{}", fixture.language);
            }
        }
    }

    #[test]
    fn every_language_reports_the_line_the_call_is_written_on() {
        // A reviewer resolves an observation by opening the file at this line,
        // so an off-by-one is a review defect rather than a cosmetic one.
        for fixture in FIXTURES {
            for observation in observe(fixture, fixture.operation) {
                let line = fixture
                    .operation
                    .lines()
                    .nth(observation.line - 1)
                    .unwrap_or_else(|| {
                        panic!(
                            "{} reports line {} outside the source",
                            fixture.language, observation.line
                        )
                    });
                assert!(
                    line.contains(&observation.method),
                    "{} reports {} at line {}, which reads {line:?}",
                    fixture.language,
                    observation.method,
                    observation.line
                );
            }
        }
    }

    #[test]
    fn no_language_reports_a_call_written_only_in_a_comment_or_a_string() {
        for fixture in FIXTURES {
            assert_eq!(
                observe(fixture, fixture.text_only),
                Vec::new(),
                "{}",
                fixture.language
            );
        }
    }

    #[test]
    fn no_language_reports_a_call_without_a_receiver() {
        // Without a receiver there is no resource identity to bind, so
        // reporting one would invent a physical identity the source lacks.
        for fixture in FIXTURES {
            assert_eq!(
                observe(fixture, fixture.receiverless_call),
                Vec::new(),
                "{}",
                fixture.language
            );
        }
    }

    #[test]
    fn every_language_fails_closed_on_source_it_cannot_read() {
        // An empty result reads as "no data write here", so unreadable source
        // must surface as a parse error and become a coverage gap.
        for fixture in FIXTURES {
            let error = detector(fixture.language)
                .observe(fixture.unparseable)
                .expect_err(fixture.language);
            assert_eq!(error, fixture.parse_error, "{}", fixture.language);
        }
    }

    #[test]
    fn every_language_accepts_source_without_any_call() {
        for fixture in FIXTURES {
            assert_eq!(observe(fixture, ""), Vec::new(), "{}", fixture.language);
        }
    }

    #[test]
    fn every_language_returns_deterministically_sorted_observations() {
        // Observations feed candidate fingerprints and report digests, so the
        // order must not depend on syntax tree traversal.
        for fixture in FIXTURES {
            let observations = observe(fixture, fixture.operation);
            assert!(
                observations.windows(2).all(|pair| pair[0] <= pair[1]),
                "{} returns unsorted observations",
                fixture.language
            );
        }
    }

    #[test]
    fn every_supported_language_is_covered_by_the_shared_contract() {
        // Adding a language to the registry must also extend this table, so a
        // new detector cannot ship without the shared assertions.
        for candidate in LANGUAGE_DETECTORS
            .iter()
            .filter(|candidate| candidate.is_supported())
        {
            assert!(
                FIXTURES
                    .iter()
                    .any(|fixture| fixture.language == candidate.language),
                "{} has a detector but no conformance fixture",
                candidate.language
            );
        }
        for fixture in FIXTURES {
            assert!(
                detector(fixture.language).is_supported(),
                "{} has a conformance fixture but no detector",
                fixture.language
            );
        }
    }

    #[test]
    fn inventory_only_languages_report_unsupported_instead_of_an_empty_result() {
        for candidate in LANGUAGE_DETECTORS
            .iter()
            .filter(|candidate| !candidate.is_supported())
        {
            let error = candidate
                .observe("orders.insert(order)")
                .expect_err(candidate.language);
            assert_eq!(
                error,
                format!("language {} is not supported", candidate.language)
            );
        }
    }

    #[test]
    fn no_file_extension_is_claimed_by_two_languages() {
        // Path lookup returns the first match, so a shared extension would
        // silently hide one language from project inventory.
        let mut owners = BTreeMap::new();
        for candidate in LANGUAGE_DETECTORS {
            for extension in candidate.extensions {
                if let Some(existing) = owners.insert(*extension, candidate.language) {
                    panic!(
                        ".{extension} is claimed by both {existing} and {}",
                        candidate.language
                    );
                }
            }
        }
    }

    #[test]
    fn path_lookup_and_language_lookup_resolve_the_same_detector() {
        for candidate in LANGUAGE_DETECTORS {
            for extension in candidate.extensions {
                let resolved = detector_for_path(&format!("src/example.{extension}"))
                    .unwrap_or_else(|| panic!(".{extension} is not resolved by path"));
                assert_eq!(resolved.language, candidate.language);
            }
        }
    }

    #[test]
    fn every_registry_extension_has_a_git_pathspec() {
        // The pathspecs decide which files project inventory can see at all, so
        // a registered extension that is missing here is invisible source.
        let pathspecs = source_pathspecs();
        for candidate in LANGUAGE_DETECTORS {
            for extension in candidate.extensions {
                assert!(
                    pathspecs.contains(&format!(":(icase)*.{extension}")),
                    "{} has no pathspec for .{extension}",
                    candidate.language
                );
            }
        }
        assert_eq!(
            pathspecs.len(),
            LANGUAGE_DETECTORS
                .iter()
                .map(|candidate| candidate.extensions.len())
                .sum::<usize>()
        );
    }
}
