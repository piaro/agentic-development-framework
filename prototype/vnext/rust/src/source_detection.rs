//! Registry and language-neutral output for source-code detectors.
//!
//! The registry is the single mechanical boundary for adding a language. A
//! language is discoverable independently from whether a detector exists, so
//! project inventory can report unsupported source instead of silently
//! excluding it.

use crate::go_detection::observe_go;
use crate::java_detection::observe_java;
use crate::kotlin_detection::observe_kotlin;
use crate::python_detection::observe_python;
use crate::ruby_detection::observe_ruby;
use crate::rust_detection::observe_rust;
use crate::script_detection::{observe_javascript, observe_jsx, observe_tsx, observe_typescript};
use std::path::Path;

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

pub fn classify_method(method: &str) -> SourceObservationKind {
    match method {
        "insert" | "Insert" | "update" | "Update" | "delete" | "Delete" => {
            SourceObservationKind::DbWrite
        }
        "publish" | "Publish" | "send_message" | "sendMessage" | "SendMessage" => {
            SourceObservationKind::MessagePublish
        }
        _ => SourceObservationKind::OtherMethodCall,
    }
}

type ObserveSource = fn(&str) -> Result<Vec<SourceObservation>, String>;

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
    // Inventory-only entries make unsupported source visible in observation
    // drafts and coverage instead of silently treating it as non-source.
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
        observe: None,
    },
    LanguageDetector {
        language: "csharp",
        extensions: &["cs"],
        observe: None,
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
    LANGUAGE_DETECTORS
        .iter()
        .find(|detector| detector.extensions.contains(&extension))
}

pub fn source_pathspecs() -> Vec<String> {
    LANGUAGE_DETECTORS
        .iter()
        .flat_map(|detector| detector.extensions)
        .map(|extension| format!("*.{extension}"))
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
        assert!(!detector_for_language("php").unwrap().is_supported());
        assert_eq!(
            detector_for_path("src/example.tsx").unwrap().language,
            "tsx"
        );
        assert_eq!(
            detector_for_path("src/example.java").unwrap().language,
            "java"
        );
        assert!(detector_for_path("README.md").is_none());
    }
}

#[cfg(test)]
mod probe {
    use super::*;

    fn dump(label: &str, result: Result<Vec<SourceObservation>, String>) {
        match result {
            Err(error) => println!("[{label}] ERR: {error}"),
            Ok(observations) => {
                if observations.is_empty() {
                    println!("[{label}] (none)");
                }
                for o in observations {
                    println!(
                        "[{label}] {:?} symbol={:?} resource={:?} method={:?} line={}",
                        o.kind, o.symbol, o.resource, o.method, o.line
                    );
                }
            }
        }
    }

    fn run(label: &str, language: &str, source: &str) {
        dump(label, detector_for_language(language).unwrap().observe(source));
    }

    #[test]
    fn probe_all() {
        run("py-empty", "python", "");
        run("py-async", "python", "async def place(o):\n    orders.insert(o)\n");
        run("py-class", "python", "class S:\n    def place(self, o):\n        self.orders.insert(o)\n");
        run("py-classbody", "python", "class S:\n    orders.insert(1)\n");
        run("py-module", "python", "orders.insert(1)\n");
        run("py-nested", "python", "def outer():\n    def inner():\n        orders.insert(1)\n");
        run("py-lambda", "python", "handler = lambda o: orders.insert(o)\n");
        run("py-chain", "python", "def place(o):\n    db.table('orders').insert(o)\n");
        run("py-multiline", "python", "def place(o):\n    client.get(\n        'orders'\n    ).insert(o)\n");
        run("py-dup", "python", "def place(o):\n    orders.insert(o)\n    orders.insert(o)\n");
        run("py-bare", "python", "def place(o):\n    insert(o)\n");
        run("py-attr-nocall", "python", "def place(o):\n    x = orders.insert\n");
        run("py-decorator", "python", "@app.route('/x')\ndef place(o):\n    orders.insert(o)\n");

        run("ts-objlit", "typescript", "const svc = {\n  place(o: O) { orders.insert(o); },\n};\n");
        run("ts-classfield", "typescript", "class S {\n  place = (o: O) => { orders.insert(o); };\n}\n");
        run("ts-classmethod", "typescript", "class S {\n  place(o: O) { orders.insert(o); }\n}\n");
        run("ts-optchain", "typescript", "function place(o: O) { orders?.insert(o); }\n");
        run("ts-nonnull", "typescript", "function place(o: O) { orders!.insert(o); }\n");
        run("ts-this", "typescript", "class S {\n  place(o: O) { this.orders.insert(o); }\n}\n");
        run("ts-computed", "typescript", "function place(o: O) { orders[\"insert\"](o); }\n");
        run("ts-generic", "typescript", "function place(o: O) { orders.insert<Order>(o); }\n");
        run("ts-default", "typescript", "export default function (o: O) { orders.insert(o); }\n");
        run("ts-assign", "typescript", "exports.place = function (o) { orders.insert(o); };\n");
        run("ts-multiline", "typescript", "function place(o: O) {\n  client\n    .table('orders')\n    .insert(o);\n}\n");
        run("js-jsx-in-js", "javascript", "export function B() {\n  return <div />;\n}\n");
        run("js-flow-ish", "javascript", "const x = 1;\norders.insert(x);\n");

        run("java-static", "java", "class S {\n  void place(Order o) {\n    Orders.insert(o);\n  }\n}\n");
        run("java-this", "java", "class S {\n  void place(Order o) {\n    this.orders.insert(o);\n  }\n}\n");
        run("java-bare", "java", "class S {\n  void place(Order o) {\n    insert(o);\n  }\n}\n");
        run("java-chain", "java", "class S {\n  void place(Order o) {\n    db.table(\"orders\").insert(o);\n  }\n}\n");
        run("java-lambda", "java", "class S {\n  void place(java.util.List<Order> os) {\n    os.forEach(o -> orders.insert(o));\n  }\n}\n");
        run("java-initializer", "java", "class S {\n  static { orders.insert(1); }\n}\n");
        run("java-anon", "java", "class S {\n  Runnable r = new Runnable() {\n    public void run() { orders.insert(1); }\n  };\n}\n");
        run("java-two-classes", "java", "class A {\n  void save() { orders.insert(1); }\n}\nclass B {\n  void save() { customers.insert(1); }\n}\n");
        run("java-record", "java", "record Order(String id) {}\n");
        run("java-super", "java", "class S extends P {\n  void place() { super.insert(1); }\n}\n");

        run("go-recv-field", "go", "package p\nfunc (s *S) Place(o O) {\n    s.orders.Insert(o)\n}\n");
        run("go-two-recv", "go", "package p\nfunc (a *A) Save() { orders.Insert(1) }\nfunc (b *B) Save() { customers.Insert(1) }\n");
        run("go-pkg-call", "go", "package p\nfunc Place(o O) {\n    sql.Insert(o)\n}\n");
        run("go-chain", "go", "package p\nfunc Place(o O) {\n    db.Table(\"orders\").Insert(o)\n}\n");
        run("go-defer", "go", "package p\nfunc Place(o O) {\n    defer orders.Insert(o)\n}\n");
        run("go-goroutine", "go", "package p\nfunc Place(o O) {\n    go orders.Insert(o)\n}\n");
        run("go-send", "go", "package p\nfunc Place(o O) {\n    queue.Send(o)\n}\n");

        run("rs-impl", "rust", "impl S {\n    fn place(&self, o: Order) {\n        self.orders.insert(o);\n    }\n}\n");
        run("rs-turbofish", "rust", "fn place(o: Order) {\n    orders.insert::<Order>(o);\n}\n");
        run("rs-assoc", "rust", "fn place(o: Order) {\n    Orders::insert(o);\n}\n");
        run("rs-macro", "rust", "fn place(o: Order) {\n    println!(\"orders.insert\");\n}\n");
        run("rs-chain", "rust", "fn place(o: Order) {\n    self.orders.lock().unwrap().insert(o);\n}\n");
        run("rs-await", "rust", "async fn place(o: Order) {\n    orders.insert(o).await;\n}\n");
        run("rs-closure", "rust", "fn place(o: Order) {\n    let f = || orders.insert(o);\n}\n");
        run("rs-multiline", "rust", "fn place(o: Order) {\n    client\n        .table(\"orders\")\n        .insert(o);\n}\n");
        run("rs-two-impls", "rust", "impl A {\n    fn save(&self) { orders.insert(1); }\n}\nimpl B {\n    fn save(&self) { customers.insert(1); }\n}\n");
    }
}
