//! Closed, versioned vocabulary shared by the built-in Detector and Rule Compiler.
//!
//! Domains are descriptive taxonomy. Detection remains driven by explicit,
//! typed repository facts; a domain never causes source semantics to be
//! inferred or a Rule to be selected implicitly.

use crate::canonical_digest;
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

pub const SIGNAL_DOMAIN_CATALOG_VERSION: &str = "1";
pub const TYPED_FACT_DETECTOR_ID: &str = "typed-repository-fact";
pub const TYPED_FACT_DETECTOR_VERSION: &str = "3";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct SignalDomainDefinition {
    pub id: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    pub signals: &'static [&'static str],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct SignalDefinition {
    pub id: &'static str,
    pub domain: &'static str,
    pub detector_id: &'static str,
    pub detector_version: &'static str,
    pub bindings: &'static [&'static str],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct FactBindingDefinition {
    pub binding: &'static str,
    pub fact_field: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct RepositoryFactDefinition {
    pub id: &'static str,
    pub bindings: &'static [FactBindingDefinition],
    pub emits: &'static [&'static str],
}

const SIGNAL_DOMAIN_DEFINITIONS: [SignalDomainDefinition; 2] = [
    SignalDomainDefinition {
        id: "data-persistence",
        title: "Data persistence",
        description: "Durable writes to application-owned data",
        signals: &["persistent-data-write"],
    },
    SignalDomainDefinition {
        id: "distributed-integration",
        title: "Distributed integration",
        description: "Effects crossing a process or service boundary",
        signals: &["distributed-effect", "message-or-event-publish"],
    },
];

const SIGNAL_DEFINITIONS: [SignalDefinition; 3] = [
    SignalDefinition {
        id: "distributed-effect",
        domain: "distributed-integration",
        detector_id: TYPED_FACT_DETECTOR_ID,
        detector_version: TYPED_FACT_DETECTOR_VERSION,
        bindings: &["integration", "operation"],
    },
    SignalDefinition {
        id: "message-or-event-publish",
        domain: "distributed-integration",
        detector_id: TYPED_FACT_DETECTOR_ID,
        detector_version: TYPED_FACT_DETECTOR_VERSION,
        bindings: &["integration", "operation"],
    },
    SignalDefinition {
        id: "persistent-data-write",
        domain: "data-persistence",
        detector_id: TYPED_FACT_DETECTOR_ID,
        detector_version: TYPED_FACT_DETECTOR_VERSION,
        bindings: &["data", "operation"],
    },
];

const REPOSITORY_FACT_DEFINITIONS: [RepositoryFactDefinition; 2] = [
    RepositoryFactDefinition {
        id: "db_write",
        bindings: &[
            FactBindingDefinition {
                binding: "data",
                fact_field: "data",
            },
            FactBindingDefinition {
                binding: "operation",
                fact_field: "operation",
            },
        ],
        emits: &["persistent-data-write"],
    },
    RepositoryFactDefinition {
        id: "message_publish",
        bindings: &[
            FactBindingDefinition {
                binding: "integration",
                fact_field: "integration",
            },
            FactBindingDefinition {
                binding: "operation",
                fact_field: "operation",
            },
        ],
        emits: &["distributed-effect", "message-or-event-publish"],
    },
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DetectorIdentity {
    pub id: &'static str,
    pub version: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SignalDomainCatalog {
    pub schema_version: &'static str,
    pub catalog_version: &'static str,
    pub detector: DetectorIdentity,
    pub domains: Vec<SignalDomainDefinition>,
    pub signals: Vec<SignalDefinition>,
    pub fact_kinds: Vec<RepositoryFactDefinition>,
    pub digest: String,
}

impl SignalDomainCatalog {
    pub fn build() -> Result<Self, SignalCatalogError> {
        validate_catalog()?;
        let body = json!({
            "schema_version": "1",
            "catalog_version": SIGNAL_DOMAIN_CATALOG_VERSION,
            "detector": {
                "id": TYPED_FACT_DETECTOR_ID,
                "version": TYPED_FACT_DETECTOR_VERSION,
            },
            "domains": SIGNAL_DOMAIN_DEFINITIONS,
            "signals": SIGNAL_DEFINITIONS,
            "fact_kinds": REPOSITORY_FACT_DEFINITIONS,
        });
        let digest =
            canonical_digest(&body).map_err(|error| SignalCatalogError::new(error.to_string()))?;
        Ok(Self {
            schema_version: "1",
            catalog_version: SIGNAL_DOMAIN_CATALOG_VERSION,
            detector: DetectorIdentity {
                id: TYPED_FACT_DETECTOR_ID,
                version: TYPED_FACT_DETECTOR_VERSION,
            },
            domains: SIGNAL_DOMAIN_DEFINITIONS.to_vec(),
            signals: SIGNAL_DEFINITIONS.to_vec(),
            fact_kinds: REPOSITORY_FACT_DEFINITIONS.to_vec(),
            digest,
        })
    }

    pub fn as_value(&self) -> Value {
        serde_json::to_value(self).expect("Signal Domain Catalog contains serializable fields")
    }

    pub fn render_text(&self) -> String {
        let mut output = format!(
            "Signal Domain Catalog {}\ndetector: {} {}\ndigest: {}\n",
            self.catalog_version, self.detector.id, self.detector.version, self.digest
        );
        for domain in &self.domains {
            output.push_str(&format!(
                "\n{}: {}\n  {}\n  signals: {}\n",
                domain.id,
                domain.title,
                domain.description,
                domain.signals.join(", ")
            ));
        }
        output.push_str("\nrepository fact kinds:\n");
        for fact in &self.fact_kinds {
            output.push_str(&format!("- {} -> {}\n", fact.id, fact.emits.join(", ")));
        }
        output
    }
}

pub fn signal_definition(id: &str) -> Option<&'static SignalDefinition> {
    SIGNAL_DEFINITIONS
        .iter()
        .find(|definition| definition.id == id)
}

pub fn repository_fact_definition(id: &str) -> Option<&'static RepositoryFactDefinition> {
    REPOSITORY_FACT_DEFINITIONS
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

fn validate_catalog() -> Result<(), SignalCatalogError> {
    let domain_ids = unique_ids(
        "Signal Domain",
        SIGNAL_DOMAIN_DEFINITIONS
            .iter()
            .map(|definition| definition.id),
    )?;
    unique_ids(
        "Signal",
        SIGNAL_DEFINITIONS.iter().map(|definition| definition.id),
    )?;
    unique_ids(
        "repository fact kind",
        REPOSITORY_FACT_DEFINITIONS
            .iter()
            .map(|definition| definition.id),
    )?;

    let mut domain_members = BTreeMap::<&str, BTreeSet<&str>>::new();
    for domain in &SIGNAL_DOMAIN_DEFINITIONS {
        domain_members.insert(domain.id, domain.signals.iter().copied().collect());
    }
    for signal in &SIGNAL_DEFINITIONS {
        if !domain_ids.contains(signal.domain) {
            return Err(SignalCatalogError::new(format!(
                "signal {} references unknown domain {}",
                signal.id, signal.domain
            )));
        }
        if !domain_members
            .get(signal.domain)
            .is_some_and(|signals| signals.contains(signal.id))
        {
            return Err(SignalCatalogError::new(format!(
                "domain {} does not declare signal {}",
                signal.domain, signal.id
            )));
        }
    }
    for domain in &SIGNAL_DOMAIN_DEFINITIONS {
        for signal in domain.signals {
            let Some(definition) = signal_definition(signal) else {
                return Err(SignalCatalogError::new(format!(
                    "domain {} references unknown signal {}",
                    domain.id, signal
                )));
            };
            if definition.domain != domain.id {
                return Err(SignalCatalogError::new(format!(
                    "signal {} belongs to domain {}, not {}",
                    signal, definition.domain, domain.id
                )));
            }
        }
    }
    for fact in &REPOSITORY_FACT_DEFINITIONS {
        let bindings = fact
            .bindings
            .iter()
            .map(|binding| binding.binding)
            .collect::<BTreeSet<_>>();
        if bindings.len() != fact.bindings.len() {
            return Err(SignalCatalogError::new(format!(
                "repository fact kind {} has duplicate bindings",
                fact.id
            )));
        }
        for signal in fact.emits {
            let definition = signal_definition(signal).ok_or_else(|| {
                SignalCatalogError::new(format!(
                    "repository fact kind {} emits unknown signal {}",
                    fact.id, signal
                ))
            })?;
            let expected = definition.bindings.iter().copied().collect::<BTreeSet<_>>();
            if bindings != expected {
                return Err(SignalCatalogError::new(format!(
                    "repository fact kind {} bindings do not match signal {}",
                    fact.id, signal
                )));
            }
        }
    }
    Ok(())
}

fn unique_ids<'a>(
    kind: &str,
    ids: impl Iterator<Item = &'a str>,
) -> Result<BTreeSet<&'a str>, SignalCatalogError> {
    let mut found = BTreeSet::new();
    for id in ids {
        if !found.insert(id) {
            return Err(SignalCatalogError::new(format!(
                "duplicate {kind} ID: {id}"
            )));
        }
    }
    Ok(found)
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
    fn catalog_defines_domains_detector_and_exact_bindings() {
        let catalog = SignalDomainCatalog::build().unwrap();
        assert_eq!(catalog.domains.len(), 2);
        assert_eq!(catalog.signals.len(), 3);
        assert_eq!(catalog.fact_kinds.len(), 2);
        assert!(catalog.digest.starts_with("sha256:"));

        let definition = signal_definition("persistent-data-write").unwrap();
        assert_eq!(definition.domain, "data-persistence");
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

    #[test]
    fn fact_definitions_are_the_single_mapping_to_signals() {
        let definition = repository_fact_definition("message_publish").unwrap();
        assert_eq!(
            definition.emits,
            ["distributed-effect", "message-or-event-publish"]
        );
        assert_eq!(
            definition.bindings,
            [
                FactBindingDefinition {
                    binding: "integration",
                    fact_field: "integration",
                },
                FactBindingDefinition {
                    binding: "operation",
                    fact_field: "operation",
                },
            ]
        );
    }
}
