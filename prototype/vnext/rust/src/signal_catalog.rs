//! Closed vocabulary shared by the built-in Detector and Rule Compiler.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

pub const TYPED_FACT_DETECTOR_ID: &str = "typed-repository-fact";
pub const TYPED_FACT_DETECTOR_VERSION: &str = "3";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SignalDefinition {
    pub id: &'static str,
    pub detector_id: &'static str,
    pub detector_version: &'static str,
    pub bindings: &'static [&'static str],
}

const SIGNAL_DEFINITIONS: [SignalDefinition; 3] = [
    SignalDefinition {
        id: "distributed-effect",
        detector_id: TYPED_FACT_DETECTOR_ID,
        detector_version: TYPED_FACT_DETECTOR_VERSION,
        bindings: &["integration", "operation"],
    },
    SignalDefinition {
        id: "message-or-event-publish",
        detector_id: TYPED_FACT_DETECTOR_ID,
        detector_version: TYPED_FACT_DETECTOR_VERSION,
        bindings: &["integration", "operation"],
    },
    SignalDefinition {
        id: "persistent-data-write",
        detector_id: TYPED_FACT_DETECTOR_ID,
        detector_version: TYPED_FACT_DETECTOR_VERSION,
        bindings: &["data", "operation"],
    },
];

pub fn signal_definition(id: &str) -> Option<&'static SignalDefinition> {
    SIGNAL_DEFINITIONS
        .iter()
        .find(|definition| definition.id == id)
}

pub fn validate_signal_candidate(
    signal: &str,
    detector_id: &str,
    detector_version: &str,
    bindings: &BTreeMap<String, String>,
) -> Result<(), SignalCatalogError> {
    let definition = signal_definition(signal)
        .ok_or_else(|| SignalCatalogError::new(format!("unknown signal: {signal}")))?;
    if definition.detector_id != detector_id || definition.detector_version != detector_version {
        return Err(SignalCatalogError::new(format!(
            "signal {signal} must be produced by {} {}",
            definition.detector_id, definition.detector_version
        )));
    }
    let actual = bindings.keys().map(String::as_str).collect::<BTreeSet<_>>();
    let expected = definition.bindings.iter().copied().collect::<BTreeSet<_>>();
    if actual != expected {
        return Err(SignalCatalogError::new(format!(
            "signal {signal} bindings must be: {}",
            definition.bindings.join(", ")
        )));
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignalCatalogError {
    message: String,
}

impl SignalCatalogError {
    fn new(message: String) -> Self {
        Self { message }
    }
}

impl fmt::Display for SignalCatalogError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for SignalCatalogError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_defines_the_detector_and_exact_bindings() {
        let definition = signal_definition("persistent-data-write").unwrap();
        assert_eq!(definition.detector_id, TYPED_FACT_DETECTOR_ID);
        assert_eq!(definition.detector_version, TYPED_FACT_DETECTOR_VERSION);
        assert_eq!(definition.bindings, ["data", "operation"]);

        let error = validate_signal_candidate(
            definition.id,
            TYPED_FACT_DETECTOR_ID,
            TYPED_FACT_DETECTOR_VERSION,
            &BTreeMap::from([("operation".to_owned(), "operation.test".to_owned())]),
        )
        .unwrap_err();
        assert_eq!(
            error.to_string(),
            "signal persistent-data-write bindings must be: data, operation"
        );
    }
}
