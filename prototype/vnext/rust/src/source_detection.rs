//! Registry and language-neutral output for source-code detectors.
//!
//! The registry is the single mechanical boundary for adding a language. A
//! language is discoverable independently from whether a detector exists, so
//! project inventory can report unsupported source instead of silently
//! excluding it.

use crate::go_detection::observe_go;
use crate::java_detection::observe_java;
use crate::python_detection::observe_python;
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
        observe: None,
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
        observe: None,
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
        assert!(!detector_for_language("kotlin").unwrap().is_supported());
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
