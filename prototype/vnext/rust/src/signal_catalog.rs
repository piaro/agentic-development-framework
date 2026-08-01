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

pub const SIGNAL_DOMAIN_CATALOG_VERSION: &str = "2";
pub const TYPED_FACT_DETECTOR_ID: &str = "typed-repository-fact";
pub const TYPED_FACT_DETECTOR_VERSION: &str = "3";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SignalDomainDefinition {
    pub id: String,
    pub title: String,
    pub description: String,
    pub signals: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SignalDefinition {
    pub id: String,
    pub domain: String,
    pub detector_id: String,
    pub detector_version: String,
    pub bindings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FactBindingDefinition {
    pub binding: String,
    pub fact_field: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RepositoryFactDefinition {
    pub id: String,
    pub bindings: Vec<FactBindingDefinition>,
    pub emits: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DetectorIdentity {
    pub id: String,
    pub version: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SignalDomainCatalog {
    pub schema_version: String,
    pub catalog_version: String,
    pub detector: DetectorIdentity,
    pub domains: Vec<SignalDomainDefinition>,
    pub signals: Vec<SignalDefinition>,
    pub fact_kinds: Vec<RepositoryFactDefinition>,
    pub digest: String,
}

impl SignalDomainCatalog {
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

/// Validated, deterministic lookup boundary used by every Signal consumer.
///
/// The first implementation contains only built-in definitions. Keeping the
/// definitions owned and indexed here allows signed Release and reviewed
/// Project catalogs to be merged later without changing Detector or Rule
/// Compiler lookup behavior.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignalCatalogRegistry {
    catalog_version: String,
    detector: DetectorIdentity,
    domains: BTreeMap<String, SignalDomainDefinition>,
    signals: BTreeMap<String, SignalDefinition>,
    fact_kinds: BTreeMap<String, RepositoryFactDefinition>,
    digest: String,
}

impl SignalCatalogRegistry {
    pub fn built_in() -> Result<Self, SignalCatalogError> {
        Self::from_definitions(
            SIGNAL_DOMAIN_CATALOG_VERSION,
            DetectorIdentity {
                id: TYPED_FACT_DETECTOR_ID.to_owned(),
                version: TYPED_FACT_DETECTOR_VERSION.to_owned(),
            },
            vec![
                SignalDomainDefinition {
                    id: "data-persistence".to_owned(),
                    title: "Data persistence".to_owned(),
                    description: "Durable writes to application-owned data".to_owned(),
                    signals: strings(&["object-storage-write", "persistent-data-write"]),
                },
                SignalDomainDefinition {
                    id: "distributed-integration".to_owned(),
                    title: "Distributed integration".to_owned(),
                    description: "Effects crossing a process or service boundary".to_owned(),
                    signals: strings(&[
                        "distributed-effect",
                        "external-system-call",
                        "message-or-event-publish",
                    ]),
                },
            ],
            vec![
                SignalDefinition {
                    id: "distributed-effect".to_owned(),
                    domain: "distributed-integration".to_owned(),
                    detector_id: TYPED_FACT_DETECTOR_ID.to_owned(),
                    detector_version: TYPED_FACT_DETECTOR_VERSION.to_owned(),
                    bindings: strings(&["integration", "operation"]),
                },
                SignalDefinition {
                    id: "external-system-call".to_owned(),
                    domain: "distributed-integration".to_owned(),
                    detector_id: TYPED_FACT_DETECTOR_ID.to_owned(),
                    detector_version: TYPED_FACT_DETECTOR_VERSION.to_owned(),
                    bindings: strings(&["integration", "operation"]),
                },
                SignalDefinition {
                    id: "message-or-event-publish".to_owned(),
                    domain: "distributed-integration".to_owned(),
                    detector_id: TYPED_FACT_DETECTOR_ID.to_owned(),
                    detector_version: TYPED_FACT_DETECTOR_VERSION.to_owned(),
                    bindings: strings(&["integration", "operation"]),
                },
                SignalDefinition {
                    id: "object-storage-write".to_owned(),
                    domain: "data-persistence".to_owned(),
                    detector_id: TYPED_FACT_DETECTOR_ID.to_owned(),
                    detector_version: TYPED_FACT_DETECTOR_VERSION.to_owned(),
                    bindings: strings(&["data", "operation"]),
                },
                SignalDefinition {
                    id: "persistent-data-write".to_owned(),
                    domain: "data-persistence".to_owned(),
                    detector_id: TYPED_FACT_DETECTOR_ID.to_owned(),
                    detector_version: TYPED_FACT_DETECTOR_VERSION.to_owned(),
                    bindings: strings(&["data", "operation"]),
                },
            ],
            vec![
                RepositoryFactDefinition {
                    id: "db_write".to_owned(),
                    bindings: vec![
                        FactBindingDefinition {
                            binding: "data".to_owned(),
                            fact_field: "data".to_owned(),
                        },
                        FactBindingDefinition {
                            binding: "operation".to_owned(),
                            fact_field: "operation".to_owned(),
                        },
                    ],
                    emits: strings(&["persistent-data-write"]),
                },
                RepositoryFactDefinition {
                    id: "external_call".to_owned(),
                    bindings: vec![
                        FactBindingDefinition {
                            binding: "integration".to_owned(),
                            fact_field: "integration".to_owned(),
                        },
                        FactBindingDefinition {
                            binding: "operation".to_owned(),
                            fact_field: "operation".to_owned(),
                        },
                    ],
                    emits: strings(&["distributed-effect", "external-system-call"]),
                },
                RepositoryFactDefinition {
                    id: "message_publish".to_owned(),
                    bindings: vec![
                        FactBindingDefinition {
                            binding: "integration".to_owned(),
                            fact_field: "integration".to_owned(),
                        },
                        FactBindingDefinition {
                            binding: "operation".to_owned(),
                            fact_field: "operation".to_owned(),
                        },
                    ],
                    emits: strings(&["distributed-effect", "message-or-event-publish"]),
                },
                RepositoryFactDefinition {
                    id: "object_write".to_owned(),
                    bindings: vec![
                        FactBindingDefinition {
                            binding: "data".to_owned(),
                            fact_field: "data".to_owned(),
                        },
                        FactBindingDefinition {
                            binding: "operation".to_owned(),
                            fact_field: "operation".to_owned(),
                        },
                    ],
                    emits: strings(&["object-storage-write", "persistent-data-write"]),
                },
            ],
        )
    }

    pub fn signal_definition(&self, id: &str) -> Option<&SignalDefinition> {
        self.signals.get(id)
    }

    pub fn repository_fact_definition(&self, id: &str) -> Option<&RepositoryFactDefinition> {
        self.fact_kinds.get(id)
    }

    pub fn validate_signal_candidate(
        &self,
        signal: &str,
        detector_id: &str,
        detector_version: &str,
        bindings: &BTreeMap<String, String>,
    ) -> Result<(), SignalCatalogError> {
        let definition = self
            .signal_definition(signal)
            .ok_or_else(|| SignalCatalogError::new(format!("unknown signal: {signal}")))?;
        if definition.detector_id != detector_id || definition.detector_version != detector_version
        {
            return Err(SignalCatalogError::new(format!(
                "signal {signal} must be produced by {} {}",
                definition.detector_id, definition.detector_version
            )));
        }
        let actual = bindings.keys().map(String::as_str).collect::<BTreeSet<_>>();
        let expected = definition
            .bindings
            .iter()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        if actual != expected {
            return Err(SignalCatalogError::new(format!(
                "signal {signal} bindings must be: {}",
                definition.bindings.join(", ")
            )));
        }
        Ok(())
    }

    pub fn catalog(&self) -> SignalDomainCatalog {
        SignalDomainCatalog {
            schema_version: "1".to_owned(),
            catalog_version: self.catalog_version.clone(),
            detector: self.detector.clone(),
            domains: self.domains.values().cloned().collect(),
            signals: self.signals.values().cloned().collect(),
            fact_kinds: self.fact_kinds.values().cloned().collect(),
            digest: self.digest.clone(),
        }
    }

    pub fn digest(&self) -> &str {
        &self.digest
    }

    pub(crate) fn from_definitions(
        catalog_version: impl Into<String>,
        detector: DetectorIdentity,
        domains: Vec<SignalDomainDefinition>,
        signals: Vec<SignalDefinition>,
        fact_kinds: Vec<RepositoryFactDefinition>,
    ) -> Result<Self, SignalCatalogError> {
        let catalog_version = catalog_version.into();
        if catalog_version.is_empty() {
            return Err(SignalCatalogError::new(
                "Signal Catalog version must not be empty",
            ));
        }
        if detector.id.is_empty() || detector.version.is_empty() {
            return Err(SignalCatalogError::new(
                "Signal Catalog detector identity must not be empty",
            ));
        }
        let domains = index_domains(domains)?;
        let signals = index_signals(signals)?;
        let fact_kinds = index_fact_kinds(fact_kinds)?;
        validate_catalog(&detector, &domains, &signals, &fact_kinds)?;
        let body = catalog_body(&catalog_version, &detector, &domains, &signals, &fact_kinds);
        let digest =
            canonical_digest(&body).map_err(|error| SignalCatalogError::new(error.to_string()))?;
        Ok(Self {
            catalog_version,
            detector,
            domains,
            signals,
            fact_kinds,
            digest,
        })
    }
}

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

#[cfg(test)]
pub(crate) fn test_signal_registry() -> SignalCatalogRegistry {
    SignalCatalogRegistry::from_definitions(
        "test",
        DetectorIdentity {
            id: TYPED_FACT_DETECTOR_ID.to_owned(),
            version: TYPED_FACT_DETECTOR_VERSION.to_owned(),
        },
        vec![SignalDomainDefinition {
            id: "test-effects".to_owned(),
            title: "Test effects".to_owned(),
            description: "Registry injection test domain".to_owned(),
            signals: strings(&["test-resource-write"]),
        }],
        vec![SignalDefinition {
            id: "test-resource-write".to_owned(),
            domain: "test-effects".to_owned(),
            detector_id: TYPED_FACT_DETECTOR_ID.to_owned(),
            detector_version: TYPED_FACT_DETECTOR_VERSION.to_owned(),
            bindings: strings(&["operation", "target"]),
        }],
        vec![RepositoryFactDefinition {
            id: "test_write".to_owned(),
            bindings: vec![
                FactBindingDefinition {
                    binding: "operation".to_owned(),
                    fact_field: "operation".to_owned(),
                },
                FactBindingDefinition {
                    binding: "target".to_owned(),
                    fact_field: "target".to_owned(),
                },
            ],
            emits: strings(&["test-resource-write"]),
        }],
    )
    .unwrap()
}

fn index_domains(
    definitions: Vec<SignalDomainDefinition>,
) -> Result<BTreeMap<String, SignalDomainDefinition>, SignalCatalogError> {
    let mut indexed = BTreeMap::new();
    for definition in definitions {
        let id = definition.id.clone();
        require_id("Signal Domain", &id)?;
        if indexed.insert(id.clone(), definition).is_some() {
            return Err(SignalCatalogError::new(format!(
                "duplicate Signal Domain ID: {id}"
            )));
        }
    }
    Ok(indexed)
}

fn index_signals(
    definitions: Vec<SignalDefinition>,
) -> Result<BTreeMap<String, SignalDefinition>, SignalCatalogError> {
    let mut indexed = BTreeMap::new();
    for definition in definitions {
        let id = definition.id.clone();
        require_id("Signal", &id)?;
        if indexed.insert(id.clone(), definition).is_some() {
            return Err(SignalCatalogError::new(format!(
                "duplicate Signal ID: {id}"
            )));
        }
    }
    Ok(indexed)
}

fn index_fact_kinds(
    definitions: Vec<RepositoryFactDefinition>,
) -> Result<BTreeMap<String, RepositoryFactDefinition>, SignalCatalogError> {
    let mut indexed = BTreeMap::new();
    for definition in definitions {
        let id = definition.id.clone();
        require_id("repository fact kind", &id)?;
        if indexed.insert(id.clone(), definition).is_some() {
            return Err(SignalCatalogError::new(format!(
                "duplicate repository fact kind ID: {id}"
            )));
        }
    }
    Ok(indexed)
}

fn require_id(kind: &str, id: &str) -> Result<(), SignalCatalogError> {
    if id.is_empty() {
        Err(SignalCatalogError::new(format!(
            "{kind} ID must not be empty"
        )))
    } else {
        Ok(())
    }
}

fn validate_catalog(
    detector: &DetectorIdentity,
    domains: &BTreeMap<String, SignalDomainDefinition>,
    signals: &BTreeMap<String, SignalDefinition>,
    fact_kinds: &BTreeMap<String, RepositoryFactDefinition>,
) -> Result<(), SignalCatalogError> {
    if domains.is_empty() || signals.is_empty() || fact_kinds.is_empty() {
        return Err(SignalCatalogError::new(
            "Signal Catalog must define domains, signals, and repository fact kinds",
        ));
    }
    for domain in domains.values() {
        let members = unique_nonempty_values(
            &format!("domain {} signals", domain.id),
            domain.signals.iter().map(String::as_str),
        )?;
        for signal in members {
            let definition = signals.get(signal).ok_or_else(|| {
                SignalCatalogError::new(format!(
                    "domain {} references unknown signal {}",
                    domain.id, signal
                ))
            })?;
            if definition.domain != domain.id {
                return Err(SignalCatalogError::new(format!(
                    "signal {} belongs to domain {}, not {}",
                    signal, definition.domain, domain.id
                )));
            }
        }
    }
    for signal in signals.values() {
        let domain = domains.get(&signal.domain).ok_or_else(|| {
            SignalCatalogError::new(format!(
                "signal {} references unknown domain {}",
                signal.id, signal.domain
            ))
        })?;
        if !domain.signals.contains(&signal.id) {
            return Err(SignalCatalogError::new(format!(
                "domain {} does not declare signal {}",
                signal.domain, signal.id
            )));
        }
        if signal.detector_id != detector.id || signal.detector_version != detector.version {
            return Err(SignalCatalogError::new(format!(
                "signal {} detector identity does not match the catalog",
                signal.id
            )));
        }
        unique_nonempty_values(
            &format!("signal {} bindings", signal.id),
            signal.bindings.iter().map(String::as_str),
        )?;
    }
    let mut emitted_signals = BTreeSet::new();
    for fact in fact_kinds.values() {
        let bindings = unique_nonempty_values(
            &format!("repository fact kind {} bindings", fact.id),
            fact.bindings.iter().map(|binding| binding.binding.as_str()),
        )?;
        unique_nonempty_values(
            &format!("repository fact kind {} fields", fact.id),
            fact.bindings
                .iter()
                .map(|binding| binding.fact_field.as_str()),
        )?;
        let emitted = unique_nonempty_values(
            &format!("repository fact kind {} signals", fact.id),
            fact.emits.iter().map(String::as_str),
        )?;
        for signal in emitted {
            let definition = signals.get(signal).ok_or_else(|| {
                SignalCatalogError::new(format!(
                    "repository fact kind {} emits unknown signal {}",
                    fact.id, signal
                ))
            })?;
            let expected = definition
                .bindings
                .iter()
                .map(String::as_str)
                .collect::<BTreeSet<_>>();
            if bindings != expected {
                return Err(SignalCatalogError::new(format!(
                    "repository fact kind {} bindings do not match signal {}",
                    fact.id, signal
                )));
            }
            emitted_signals.insert(signal);
        }
    }
    for signal in signals.keys() {
        if !emitted_signals.contains(signal.as_str()) {
            return Err(SignalCatalogError::new(format!(
                "signal {signal} is not emitted by a repository fact kind"
            )));
        }
    }
    Ok(())
}

fn unique_nonempty_values<'a>(
    label: &str,
    values: impl Iterator<Item = &'a str>,
) -> Result<BTreeSet<&'a str>, SignalCatalogError> {
    let mut found = BTreeSet::new();
    for value in values {
        if value.is_empty() {
            return Err(SignalCatalogError::new(format!(
                "{label} must not contain an empty value"
            )));
        }
        if !found.insert(value) {
            return Err(SignalCatalogError::new(format!(
                "{label} contains duplicate value: {value}"
            )));
        }
    }
    if found.is_empty() {
        return Err(SignalCatalogError::new(format!(
            "{label} must not be empty"
        )));
    }
    Ok(found)
}

fn catalog_body(
    catalog_version: &str,
    detector: &DetectorIdentity,
    domains: &BTreeMap<String, SignalDomainDefinition>,
    signals: &BTreeMap<String, SignalDefinition>,
    fact_kinds: &BTreeMap<String, RepositoryFactDefinition>,
) -> Value {
    json!({
        "schema_version": "1",
        "catalog_version": catalog_version,
        "detector": detector,
        "domains": domains.values().collect::<Vec<_>>(),
        "signals": signals.values().collect::<Vec<_>>(),
        "fact_kinds": fact_kinds.values().collect::<Vec<_>>(),
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignalCatalogError {
    message: String,
}

impl SignalCatalogError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
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
    fn built_in_registry_defines_domains_detector_and_exact_bindings() {
        let registry = SignalCatalogRegistry::built_in().unwrap();
        let catalog = registry.catalog();
        assert_eq!(catalog.domains.len(), 2);
        assert_eq!(catalog.signals.len(), 5);
        assert_eq!(catalog.fact_kinds.len(), 4);
        assert_eq!(catalog.digest, registry.digest());

        let definition = registry.signal_definition("persistent-data-write").unwrap();
        assert_eq!(definition.domain, "data-persistence");
        assert_eq!(definition.detector_id, TYPED_FACT_DETECTOR_ID);
        assert_eq!(definition.detector_version, TYPED_FACT_DETECTOR_VERSION);
        assert_eq!(definition.bindings, strings(&["data", "operation"]));

        let error = registry
            .validate_signal_candidate(
                &definition.id,
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
        let registry = SignalCatalogRegistry::built_in().unwrap();
        let definition = registry
            .repository_fact_definition("message_publish")
            .unwrap();
        assert_eq!(
            definition.emits,
            strings(&["distributed-effect", "message-or-event-publish"])
        );
        assert_eq!(
            definition.bindings,
            [
                FactBindingDefinition {
                    binding: "integration".to_owned(),
                    fact_field: "integration".to_owned(),
                },
                FactBindingDefinition {
                    binding: "operation".to_owned(),
                    fact_field: "operation".to_owned(),
                },
            ]
        );
        assert_eq!(
            registry
                .repository_fact_definition("external_call")
                .unwrap()
                .emits,
            strings(&["distributed-effect", "external-system-call"])
        );
        assert_eq!(
            registry
                .repository_fact_definition("object_write")
                .unwrap()
                .emits,
            strings(&["object-storage-write", "persistent-data-write"])
        );
    }

    #[test]
    fn registry_rejects_conflicting_ids_before_merge() {
        let mut catalog = SignalCatalogRegistry::built_in().unwrap().catalog();
        catalog.signals.push(catalog.signals[0].clone());
        let error = SignalCatalogRegistry::from_definitions(
            catalog.catalog_version,
            catalog.detector,
            catalog.domains,
            catalog.signals,
            catalog.fact_kinds,
        )
        .unwrap_err();
        assert_eq!(error.to_string(), "duplicate Signal ID: distributed-effect");
    }
}
