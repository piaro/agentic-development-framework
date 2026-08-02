//! Non-authoritative framework hints for reviewed method bindings.
//!
//! These rules never affect repository facts or coverage. They only annotate
//! `project observe` drafts with mechanically explainable candidates that a
//! reviewer may turn into an accepted Binding Record.

use crate::source_detection::{SourceObservation, SourceObservationKind};
use serde::Deserialize;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::Path;

const DJANGO: &str = "django-orm";
const SQLALCHEMY: &str = "sqlalchemy";
const PRISMA: &str = "prisma";
const SPRING_DATA_JPA: &str = "spring-data-jpa";
const EF_CORE: &str = "entity-framework-core";
const RAILS_ACTIVE_RECORD: &str = "rails-active-record";
const LARAVEL_ELOQUENT: &str = "laravel-eloquent";
const GORM: &str = "gorm";
const AMAZON_SQS: &str = "amazon-sqs";
const APACHE_KAFKA: &str = "apache-kafka";
const RABBITMQ: &str = "rabbitmq";
const CELERY: &str = "celery";
const GOOGLE_CLOUD_PUBSUB: &str = "google-cloud-pubsub";
const AZURE_SERVICE_BUS: &str = "azure-service-bus";
const NATS: &str = "nats";
const REDIS_STREAMS: &str = "redis-streams";
const PYTHON_REQUESTS: &str = "python-requests";
const PYTHON_HTTPX: &str = "python-httpx";
const WEB_FETCH: &str = "web-fetch";
const AXIOS: &str = "axios";
const JAVA_HTTP_CLIENT: &str = "java-http-client";
const SPRING_WEBCLIENT: &str = "spring-webclient";
const GO_NET_HTTP: &str = "go-net-http";
const DOTNET_HTTP_CLIENT: &str = "dotnet-http-client";
const AMAZON_S3: &str = "amazon-s3";
const GOOGLE_CLOUD_STORAGE: &str = "google-cloud-storage";
const AZURE_BLOB_STORAGE: &str = "azure-blob-storage";

// Method vocabularies follow the projects' current official persistence and
// publishing references. Keep uncertain dual-use APIs with no suggested kinds.
//
// Django: https://docs.djangoproject.com/en/6.0/ref/models/querysets/
// SQLAlchemy: https://docs.sqlalchemy.org/en/20/orm/session_basics.html
// Prisma: https://docs.prisma.io/docs/orm/prisma-client/queries/crud
// Spring Data JPA:
// https://docs.spring.io/spring-data/jpa/reference/repositories/core-concepts.html
// EF Core: https://learn.microsoft.com/en-us/ef/core/saving/
// Rails: https://guides.rubyonrails.org/active_record_basics.html
// Laravel: https://laravel.com/docs/13.x/eloquent
// GORM: https://gorm.io/docs/
// Amazon SQS:
// https://docs.aws.amazon.com/AWSSimpleQueueService/latest/SQSDeveloperGuide/
// Apache Kafka:
// https://kafka.apache.org/41/javadoc/org/apache/kafka/clients/producer/KafkaProducer.html
// RabbitMQ: https://www.rabbitmq.com/tutorials
// Celery: https://docs.celeryq.dev/en/stable/userguide/calling.html
// Google Cloud Pub/Sub: https://cloud.google.com/pubsub/docs/publisher
// Azure Service Bus:
// https://learn.microsoft.com/dotnet/api/overview/azure/messaging.servicebus-readme
// NATS: https://docs.nats.io/using-nats/developer/sending
// Redis Streams: https://redis.io/docs/latest/commands/xadd/
// Requests: https://requests.readthedocs.io/en/stable/api/
// HTTPX: https://www.python-httpx.org/api/
// Fetch: https://developer.mozilla.org/en-US/docs/Web/API/Window/fetch
// Axios: https://axios-http.com/docs/api_intro
// Java HttpClient:
// https://docs.oracle.com/en/java/javase/25/docs/api/java.net.http/java/net/http/HttpClient.html
// Spring WebClient:
// https://docs.spring.io/spring-framework/reference/web/webflux-webclient.html
// Go net/http: https://pkg.go.dev/net/http#Client
// .NET HttpClient:
// https://learn.microsoft.com/dotnet/api/system.net.http.httpclient
// Amazon S3:
// https://docs.aws.amazon.com/AmazonS3/latest/userguide/upload-objects.html
// Google Cloud Storage: https://cloud.google.com/storage/docs/uploading-objects
// Azure Blob Storage:
// https://learn.microsoft.com/azure/storage/blobs/storage-blob-upload

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SuggestedFactKind {
    DbWrite,
    MessagePublish,
    ExternalCall,
    ObjectWrite,
}

impl SuggestedFactKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::DbWrite => "db_write",
            Self::MessagePublish => "message_publish",
            Self::ExternalCall => "external_call",
            Self::ObjectWrite => "object_write",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct FrameworkCandidate {
    pub framework: String,
    pub symbol: String,
    pub resource: String,
    pub method: String,
    pub line: usize,
    pub binding_key: String,
    pub suggested_fact_kinds: Vec<SuggestedFactKind>,
    pub method_binding_required: bool,
    pub evidence: Vec<String>,
    pub rationale: String,
}

#[derive(Debug, Default, Clone)]
pub struct FrameworkCatalog {
    project_evidence: BTreeMap<String, BTreeSet<String>>,
    release_rules: Vec<ReleaseFrameworkRule>,
    release_project_evidence: BTreeMap<String, BTreeSet<String>>,
}

#[derive(Debug, Clone)]
struct ReleaseFrameworkRule {
    key: String,
    framework: String,
    languages: BTreeSet<String>,
    methods: BTreeSet<String>,
    manifest_markers: Vec<String>,
    source_markers: Vec<String>,
    receiver_markers: Vec<String>,
    suggested_fact_kinds: Vec<SuggestedFactKind>,
    rationale: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReleaseCatalogSource {
    schema_version: String,
    namespace: String,
    rules: Vec<ReleaseRuleSource>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReleaseRuleSource {
    id: String,
    framework: String,
    languages: Vec<String>,
    methods: Vec<String>,
    #[serde(default)]
    manifest_markers: Vec<String>,
    #[serde(default)]
    source_markers: Vec<String>,
    #[serde(default)]
    receiver_markers: Vec<String>,
    #[serde(default)]
    suggested_fact_kinds: Vec<String>,
    rationale: String,
}

impl FrameworkCatalog {
    /// Merge non-authoritative detector hints from a signed Framework Release.
    ///
    /// External framework identities are always qualified as `namespace/name`.
    /// This keeps them disjoint from built-in identities and makes ownership
    /// visible in every generated review candidate.
    pub fn with_release_source(source: &Value) -> Result<Self, FrameworkCatalogError> {
        let parsed: ReleaseCatalogSource = serde_json::from_value(source.clone())
            .map_err(|error| catalog_error(format!("invalid Framework Catalog: {error}")))?;
        validate_identifier(&parsed.namespace, "Framework Catalog namespace", true)?;
        if parsed.namespace == "agentic" || parsed.namespace.starts_with("agentic.") {
            return Err(catalog_error(
                "Framework Catalog namespace agentic is reserved for built-in rules",
            ));
        }
        if parsed.schema_version != "1" {
            return Err(catalog_error(format!(
                "unsupported Framework Catalog Schema: {}",
                parsed.schema_version
            )));
        }
        if parsed.rules.is_empty() {
            return Err(catalog_error("Framework Catalog rules must be non-empty"));
        }

        let mut seen_rule_ids = BTreeSet::new();
        let mut seen_matches = BTreeSet::new();
        let mut release_rules = Vec::new();
        for rule in parsed.rules {
            validate_identifier(&rule.id, "Framework Catalog rule ID", false)?;
            validate_identifier(&rule.framework, "Framework Catalog framework name", false)?;
            if !seen_rule_ids.insert(rule.id.clone()) {
                return Err(catalog_error(format!(
                    "duplicate Framework Catalog rule ID: {:?}",
                    rule.id
                )));
            }
            let languages = unique_non_empty(rule.languages, "rule languages")?;
            for language in &languages {
                let supported = crate::source_detection::detector_for_language(language)
                    .is_some_and(|detector| detector.is_supported());
                if !supported {
                    return Err(catalog_error(format!(
                        "Framework Catalog rule {:?} uses unsupported language {:?}",
                        rule.id, language
                    )));
                }
            }
            let methods = unique_non_empty(rule.methods, "rule methods")?;
            let manifest_markers = normalized_markers(rule.manifest_markers, "manifest markers")?;
            let source_markers = normalized_markers(rule.source_markers, "source markers")?;
            let receiver_markers = normalized_markers(rule.receiver_markers, "receiver markers")?;
            if manifest_markers.is_empty() && source_markers.is_empty() {
                return Err(catalog_error(format!(
                    "Framework Catalog rule {:?} requires a manifest or source marker",
                    rule.id
                )));
            }
            if rule.rationale.trim().is_empty() {
                return Err(catalog_error(format!(
                    "Framework Catalog rule {:?} rationale must be non-empty",
                    rule.id
                )));
            }
            let suggested_fact_kind_count = rule.suggested_fact_kinds.len();
            let suggested_fact_kinds = rule
                .suggested_fact_kinds
                .iter()
                .map(|kind| parse_suggested_fact_kind(kind, &rule.id))
                .collect::<Result<BTreeSet<_>, _>>()?;
            if suggested_fact_kinds.len() != suggested_fact_kind_count {
                return Err(catalog_error(format!(
                    "Framework Catalog rule {:?} suggested fact kinds must be unique",
                    rule.id
                )));
            }
            let suggested_fact_kinds = suggested_fact_kinds.into_iter().collect::<Vec<_>>();
            let framework = format!("{}/{}", parsed.namespace, rule.framework);
            for language in &languages {
                for method in &methods {
                    if !seen_matches.insert((framework.clone(), language.clone(), method.clone())) {
                        return Err(catalog_error(format!(
                            "duplicate Framework Catalog match for {framework} {language}.{method}"
                        )));
                    }
                }
            }
            release_rules.push(ReleaseFrameworkRule {
                key: format!("{}/{}", parsed.namespace, rule.id),
                framework,
                languages,
                methods,
                manifest_markers,
                source_markers,
                receiver_markers,
                suggested_fact_kinds,
                rationale: rule.rationale,
            });
        }
        release_rules.sort_by(|left, right| left.key.cmp(&right.key));
        Ok(Self {
            release_rules,
            ..Self::default()
        })
    }

    pub fn record_manifest(&mut self, path: &str, content: &str) {
        let content = content.to_ascii_lowercase();
        for (framework, markers) in [
            (DJANGO, &["django"][..]),
            (SQLALCHEMY, &["sqlalchemy"][..]),
            // The `prisma` package is a development CLI and is also used by
            // non-Prisma ORMs against Prisma Postgres. Only the runtime client
            // is project-level evidence for Prisma Client call candidates.
            (PRISMA, &["@prisma/client"][..]),
            (
                SPRING_DATA_JPA,
                &[
                    "spring-data-jpa",
                    "spring-boot-starter-data-jpa",
                    "springframework.data.jpa",
                    "hibernate-core",
                ][..],
            ),
            (EF_CORE, &["microsoft.entityframeworkcore"][..]),
            (
                RAILS_ACTIVE_RECORD,
                &["gem \"rails\"", "gem 'rails'", "activerecord"][..],
            ),
            (
                LARAVEL_ELOQUENT,
                &["laravel/framework", "illuminate/database"][..],
            ),
            (GORM, &["gorm.io/gorm"][..]),
            (
                AMAZON_SQS,
                &[
                    "boto3",
                    "@aws-sdk/client-sqs",
                    "software.amazon.awssdk.services.sqs",
                    "awssdk.sqs",
                    "aws-sdk-go-v2/service/sqs",
                    "aws-sdk-sqs",
                ][..],
            ),
            (
                APACHE_KAFKA,
                &[
                    "kafka-python",
                    "confluent-kafka",
                    "kafkajs",
                    "org.apache.kafka",
                    "confluent.kafka",
                    "segmentio/kafka-go",
                    "shopify/sarama",
                    "twmb/franz-go",
                    "ruby-kafka",
                    "php-rdkafka",
                    "rdkafka",
                ][..],
            ),
            (
                RABBITMQ,
                &[
                    "pika",
                    "aio-pika",
                    "\"amqplib\"",
                    "com.rabbitmq",
                    "rabbitmq.client",
                    "rabbitmq/amqp091-go",
                    "php-amqplib",
                    "bunny",
                    "lapin",
                ][..],
            ),
            (CELERY, &["celery"][..]),
            (
                GOOGLE_CLOUD_PUBSUB,
                &[
                    "google-cloud-pubsub",
                    "google/cloud-pubsub",
                    "@google-cloud/pubsub",
                    "google.cloud:google-cloud-pubsub",
                    "google.cloud.pubsub",
                    "cloud.google.com/go/pubsub",
                ][..],
            ),
            (
                AZURE_SERVICE_BUS,
                &[
                    "azure-servicebus",
                    "azure-service-bus",
                    "azure.messaging.servicebus",
                    "@azure/service-bus",
                    "azservicebus",
                ][..],
            ),
            (
                NATS,
                &[
                    "nats-py",
                    "\"nats\"",
                    "nats.ws",
                    "io.nats",
                    "nats.client",
                    "nats-io/nats.go",
                    "nats-pure",
                ][..],
            ),
            (
                REDIS_STREAMS,
                &[
                    "redis",
                    "ioredis",
                    "jedis",
                    "lettuce-core",
                    "go-redis",
                    "stackexchange.redis",
                ][..],
            ),
            (
                PYTHON_REQUESTS,
                &["requests==", "requests>=", "requests~=", "\"requests\""][..],
            ),
            (
                PYTHON_HTTPX,
                &["httpx==", "httpx>=", "httpx~=", "\"httpx\""][..],
            ),
            (AXIOS, &["\"axios\"", "'axios'"][..]),
            (
                SPRING_WEBCLIENT,
                &["spring-webflux", "spring-boot-starter-webflux"][..],
            ),
            (
                AMAZON_S3,
                &[
                    "boto3",
                    "@aws-sdk/client-s3",
                    "@aws-sdk/lib-storage",
                    "software.amazon.awssdk.services.s3",
                    "awssdk.s3",
                    "aws-sdk-go-v2/service/s3",
                    "aws-sdk-s3",
                ][..],
            ),
            (
                GOOGLE_CLOUD_STORAGE,
                &[
                    "google-cloud-storage",
                    "google/cloud-storage",
                    "@google-cloud/storage",
                    "google.cloud:google-cloud-storage",
                    "google.cloud.storage",
                    "cloud.google.com/go/storage",
                ][..],
            ),
            (
                AZURE_BLOB_STORAGE,
                &[
                    "azure-storage-blob",
                    "azure.storage.blobs",
                    "azure-storage-blobs",
                    "@azure/storage-blob",
                    "azblob",
                ][..],
            ),
        ] {
            if markers.iter().any(|marker| content.contains(marker)) {
                self.project_evidence
                    .entry(framework.to_owned())
                    .or_default()
                    .insert(format!("project-manifest:{path}"));
            }
        }
        for rule in &self.release_rules {
            if rule
                .manifest_markers
                .iter()
                .any(|marker| content.contains(marker))
            {
                self.release_project_evidence
                    .entry(rule.key.clone())
                    .or_default()
                    .insert(format!("project-manifest:{path}"));
            }
        }
    }

    pub fn candidates(
        &self,
        path: &str,
        language: &str,
        source: &str,
        observations: &[SourceObservation],
    ) -> Vec<FrameworkCandidate> {
        let source_lower = source.to_ascii_lowercase();
        let mut candidates = Vec::new();
        for observation in observations {
            for rule in framework_rules(language, &observation.method) {
                let mut evidence = self
                    .project_evidence
                    .get(rule.framework)
                    .into_iter()
                    .flatten()
                    .filter(|evidence| manifest_evidence_applies(evidence, path))
                    .cloned()
                    .collect::<BTreeSet<_>>();
                evidence.extend(source_evidence(
                    rule.framework,
                    path,
                    &source_lower,
                    &observation.resource,
                    &observation.method,
                ));
                if evidence.is_empty() {
                    continue;
                }
                if !candidate_receiver_is_plausible(
                    rule.framework,
                    language,
                    &source_lower,
                    &observation.resource,
                    &observation.method,
                ) {
                    continue;
                }
                candidates.push(FrameworkCandidate {
                    framework: rule.framework.to_owned(),
                    symbol: observation.symbol.clone(),
                    resource: observation.resource.clone(),
                    method: observation.method.clone(),
                    line: observation.line,
                    binding_key: format!("{}.{}", observation.resource, observation.method),
                    suggested_fact_kinds: match rule.suggested_fact_kind {
                        Some(SuggestedFactKind::ObjectWrite) => vec![
                            SuggestedFactKind::ExternalCall,
                            SuggestedFactKind::ObjectWrite,
                        ],
                        Some(kind) => vec![kind],
                        None => Vec::new(),
                    },
                    method_binding_required: observation.kind
                        == SourceObservationKind::OtherMethodCall,
                    evidence: evidence.into_iter().collect(),
                    rationale: rule.rationale.to_owned(),
                });
            }
            let resource_lower = observation.resource.to_ascii_lowercase();
            for rule in self.release_rules.iter().filter(|rule| {
                rule.languages.contains(language) && rule.methods.contains(&observation.method)
            }) {
                let mut evidence = self
                    .release_project_evidence
                    .get(&rule.key)
                    .into_iter()
                    .flatten()
                    .filter(|evidence| manifest_evidence_applies(evidence, path))
                    .cloned()
                    .collect::<BTreeSet<_>>();
                evidence.extend(
                    rule.source_markers
                        .iter()
                        .filter(|marker| source_lower.contains(marker.as_str()))
                        .map(|marker| format!("source-marker:{}:{marker}", rule.key)),
                );
                if evidence.is_empty()
                    || !rule.receiver_markers.is_empty()
                        && !rule
                            .receiver_markers
                            .iter()
                            .any(|marker| resource_lower.contains(marker))
                {
                    continue;
                }
                candidates.push(FrameworkCandidate {
                    framework: rule.framework.clone(),
                    symbol: observation.symbol.clone(),
                    resource: observation.resource.clone(),
                    method: observation.method.clone(),
                    line: observation.line,
                    binding_key: format!("{}.{}", observation.resource, observation.method),
                    suggested_fact_kinds: rule.suggested_fact_kinds.clone(),
                    method_binding_required: observation.kind
                        == SourceObservationKind::OtherMethodCall,
                    evidence: evidence.into_iter().collect(),
                    rationale: rule.rationale.clone(),
                });
            }
        }
        candidates.sort();
        candidates
    }
}

fn validate_identifier(
    value: &str,
    label: &str,
    allow_dots: bool,
) -> Result<(), FrameworkCatalogError> {
    let bytes = value.as_bytes();
    let valid = bytes.first().is_some_and(u8::is_ascii_lowercase)
        && bytes.iter().enumerate().all(|(index, byte)| {
            if byte.is_ascii_lowercase() || byte.is_ascii_digit() {
                return true;
            }
            let separator = matches!(byte, b'-' | b'_') || allow_dots && *byte == b'.';
            separator
                && index > 0
                && bytes
                    .get(index + 1)
                    .is_some_and(|next| next.is_ascii_lowercase() || next.is_ascii_digit())
        });
    if !valid {
        return Err(catalog_error(format!("{label} is invalid: {value:?}")));
    }
    Ok(())
}

fn unique_non_empty(
    values: Vec<String>,
    label: &str,
) -> Result<BTreeSet<String>, FrameworkCatalogError> {
    if values.is_empty() || values.iter().any(|value| value.trim().is_empty()) {
        return Err(catalog_error(format!(
            "Framework Catalog {label} must be non-empty"
        )));
    }
    let count = values.len();
    let unique = values.into_iter().collect::<BTreeSet<_>>();
    if unique.len() != count {
        return Err(catalog_error(format!(
            "Framework Catalog {label} must be unique"
        )));
    }
    Ok(unique)
}

fn normalized_markers(
    values: Vec<String>,
    label: &str,
) -> Result<Vec<String>, FrameworkCatalogError> {
    if values.iter().any(|value| value.trim().is_empty()) {
        return Err(catalog_error(format!(
            "Framework Catalog {label} must not contain empty values"
        )));
    }
    let count = values.len();
    let unique = values
        .into_iter()
        .map(|value| value.to_ascii_lowercase())
        .collect::<BTreeSet<_>>();
    if unique.len() != count {
        return Err(catalog_error(format!(
            "Framework Catalog {label} must be case-insensitively unique"
        )));
    }
    Ok(unique.into_iter().collect())
}

fn parse_suggested_fact_kind(
    kind: &str,
    rule_id: &str,
) -> Result<SuggestedFactKind, FrameworkCatalogError> {
    match kind {
        "db_write" => Ok(SuggestedFactKind::DbWrite),
        "message_publish" => Ok(SuggestedFactKind::MessagePublish),
        "external_call" => Ok(SuggestedFactKind::ExternalCall),
        "object_write" => Ok(SuggestedFactKind::ObjectWrite),
        other => Err(catalog_error(format!(
            "Framework Catalog rule {rule_id:?} has unsupported suggested fact kind {other:?}"
        ))),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameworkCatalogError {
    message: String,
}

impl fmt::Display for FrameworkCatalogError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for FrameworkCatalogError {}

fn catalog_error(message: impl Into<String>) -> FrameworkCatalogError {
    FrameworkCatalogError {
        message: message.into(),
    }
}

fn manifest_evidence_applies(evidence: &str, source_path: &str) -> bool {
    let Some(manifest_path) = evidence.strip_prefix("project-manifest:") else {
        return true;
    };
    let Some((directory, _)) = manifest_path.rsplit_once('/') else {
        return true;
    };
    source_path == directory
        || source_path
            .strip_prefix(directory)
            .is_some_and(|remainder| remainder.starts_with('/'))
}

fn candidate_receiver_is_plausible(
    framework: &str,
    language: &str,
    source: &str,
    resource: &str,
    method: &str,
) -> bool {
    if framework == PRISMA {
        let resource = resource.to_ascii_lowercase();
        if resource.contains("prisma") {
            return true;
        }
        let root = resource
            .split(|character: char| !character.is_ascii_alphanumeric() && character != '_')
            .next()
            .unwrap_or_default();
        return !root.is_empty()
            && ["const", "let", "var"].iter().any(|declaration| {
                source.contains(&format!("{declaration} {root} = new prismaclient"))
            });
    }

    if framework == DJANGO
        && language == "python"
        && method == "update"
        && is_explicit_python_collection(source, resource)
    {
        return false;
    }
    true
}

fn is_explicit_python_collection(source: &str, resource: &str) -> bool {
    if resource == "__dict__" || resource.ends_with(".__dict__") {
        return true;
    }
    let constructors = ["{", "dict(", "set(", "list(", "["];
    if resource
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        && constructors
            .iter()
            .any(|constructor| source.contains(&format!("{resource} = {constructor}")))
    {
        return true;
    }
    if let Some(field) = resource.strip_prefix("self.") {
        return constructors.iter().any(|constructor| {
            source.contains(&format!("self.{field} = {constructor}"))
                || source.contains(&format!("\"{field}\": {constructor}"))
                || source.contains(&format!("'{field}': {constructor}"))
        });
    }
    false
}

pub(crate) fn is_framework_manifest_path(path: &str) -> bool {
    let path = Path::new(path);
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    matches!(
        name,
        "pyproject.toml"
            | "requirements.txt"
            | "Pipfile"
            | "setup.py"
            | "setup.cfg"
            | "package.json"
            | "pom.xml"
            | "build.gradle"
            | "build.gradle.kts"
            | "Directory.Packages.props"
            | "Gemfile"
            | "composer.json"
            | "go.mod"
    ) || name.starts_with("requirements") && name.ends_with(".txt")
        || path.extension().and_then(|extension| extension.to_str()) == Some("csproj")
}

#[derive(Clone, Copy)]
struct FrameworkRule {
    framework: &'static str,
    suggested_fact_kind: Option<SuggestedFactKind>,
    rationale: &'static str,
}

fn framework_rules(language: &str, method: &str) -> Vec<FrameworkRule> {
    let mut rules = Vec::new();
    match language {
        "python" => {
            if DJANGO_METHODS.contains(&method) {
                rules.push(db_rule(
                    DJANGO,
                    "Django Model or QuerySet persistence API can write model state.",
                ));
            }
            if SQLALCHEMY_WRITE_METHODS.contains(&method) {
                rules.push(db_rule(
                    SQLALCHEMY,
                    "SQLAlchemy Session persistence API can flush pending state to the database.",
                ));
            } else if method == "execute" {
                rules.push(FrameworkRule {
                    framework: SQLALCHEMY,
                    suggested_fact_kind: None,
                    rationale:
                        "SQLAlchemy execute accepts both read and write statements; inspect this call before choosing a kind.",
                });
            }
        }
        "javascript" | "jsx" | "typescript" | "tsx" if PRISMA_METHODS.contains(&method) => {
            rules.push(db_rule(
                PRISMA,
                "Prisma Client mutation API writes one or more model records.",
            ));
        }
        "java" | "kotlin" if SPRING_DATA_JPA_METHODS.contains(&method) => {
            rules.push(db_rule(
                SPRING_DATA_JPA,
                "Spring Data JPA or EntityManager persistence API can write entity state.",
            ));
        }
        "csharp" if EF_CORE_METHODS.contains(&method) => {
            rules.push(db_rule(
                EF_CORE,
                "Entity Framework Core persistence API applies tracked or direct changes to the database.",
            ));
        }
        "ruby" if RAILS_ACTIVE_RECORD_METHODS.contains(&method) => {
            rules.push(db_rule(
                RAILS_ACTIVE_RECORD,
                "Active Record persistence API writes model state.",
            ));
        }
        "php" if LARAVEL_ELOQUENT_METHODS.contains(&method) => {
            rules.push(db_rule(
                LARAVEL_ELOQUENT,
                "Eloquent model or query builder persistence API writes model state.",
            ));
        }
        "go" if GORM_METHODS.contains(&method) => {
            rules.push(db_rule(
                GORM,
                "GORM mutation API writes model state or executes a reviewed SQL command.",
            ));
        }
        _ => {}
    }
    for (framework, methods, rationale) in messaging_rules(language) {
        if methods.contains(&method) {
            rules.push(message_rule(framework, rationale));
        }
    }
    for (framework, methods, rationale) in external_call_rules(language) {
        if methods.contains(&method) {
            rules.push(external_call_rule(framework, rationale));
        }
    }
    for (framework, methods, suggested_fact_kind, rationale) in object_storage_rules(language) {
        if methods.contains(&method) {
            rules.push(FrameworkRule {
                framework,
                suggested_fact_kind,
                rationale,
            });
        }
    }
    rules
}

fn messaging_rules(language: &str) -> Vec<(&'static str, &'static [&'static str], &'static str)> {
    let mut rules = Vec::new();
    let rationale = |framework| match framework {
        AMAZON_SQS => "Amazon SQS send API publishes one or more queue messages.",
        APACHE_KAFKA => "Kafka producer API publishes records to a topic.",
        RABBITMQ => "RabbitMQ producer API publishes a message to an exchange or queue.",
        CELERY => "Celery Calling API sends a task message to a broker.",
        GOOGLE_CLOUD_PUBSUB => "Google Cloud Pub/Sub publisher API publishes a topic message.",
        AZURE_SERVICE_BUS => {
            "Azure Service Bus sender API publishes one or more brokered messages."
        }
        NATS => "NATS publish API sends a message to a subject or stream.",
        REDIS_STREAMS => "Redis XADD appends a message to a stream.",
        _ => unreachable!("unknown messaging framework"),
    };
    match language {
        "python" => {
            rules.push((
                AMAZON_SQS,
                &["send_message", "send_messages", "send_message_batch"][..],
                rationale(AMAZON_SQS),
            ));
            rules.push((
                APACHE_KAFKA,
                &["send", "send_batch", "produce", "produce_batch"][..],
                rationale(APACHE_KAFKA),
            ));
            rules.push((
                RABBITMQ,
                &["basic_publish", "publish"][..],
                rationale(RABBITMQ),
            ));
            rules.push((
                CELERY,
                &["delay", "apply_async", "send_task"][..],
                rationale(CELERY),
            ));
            rules.push((
                GOOGLE_CLOUD_PUBSUB,
                &["publish"][..],
                rationale(GOOGLE_CLOUD_PUBSUB),
            ));
            rules.push((
                AZURE_SERVICE_BUS,
                &["send_messages"][..],
                rationale(AZURE_SERVICE_BUS),
            ));
            rules.push((NATS, &["publish"][..], rationale(NATS)));
            rules.push((REDIS_STREAMS, &["xadd"][..], rationale(REDIS_STREAMS)));
        }
        "javascript" | "jsx" | "typescript" | "tsx" => {
            rules.push((
                AMAZON_SQS,
                &["send", "sendMessage", "sendMessageBatch"][..],
                rationale(AMAZON_SQS),
            ));
            rules.push((
                APACHE_KAFKA,
                &["send", "sendBatch", "produce"][..],
                rationale(APACHE_KAFKA),
            ));
            rules.push((
                RABBITMQ,
                &["publish", "sendToQueue"][..],
                rationale(RABBITMQ),
            ));
            rules.push((
                GOOGLE_CLOUD_PUBSUB,
                &["publish", "publishMessage"][..],
                rationale(GOOGLE_CLOUD_PUBSUB),
            ));
            rules.push((
                AZURE_SERVICE_BUS,
                &["sendMessages"][..],
                rationale(AZURE_SERVICE_BUS),
            ));
            rules.push((NATS, &["publish"][..], rationale(NATS)));
            rules.push((
                REDIS_STREAMS,
                &["xAdd", "xadd"][..],
                rationale(REDIS_STREAMS),
            ));
        }
        "java" | "kotlin" => {
            rules.push((
                AMAZON_SQS,
                &["sendMessage", "sendMessageBatch"][..],
                rationale(AMAZON_SQS),
            ));
            rules.push((APACHE_KAFKA, &["send"][..], rationale(APACHE_KAFKA)));
            rules.push((RABBITMQ, &["basicPublish", "send"][..], rationale(RABBITMQ)));
            rules.push((
                GOOGLE_CLOUD_PUBSUB,
                &["publish"][..],
                rationale(GOOGLE_CLOUD_PUBSUB),
            ));
            rules.push((
                AZURE_SERVICE_BUS,
                &["sendMessage", "sendMessages"][..],
                rationale(AZURE_SERVICE_BUS),
            ));
            rules.push((NATS, &["publish"][..], rationale(NATS)));
            rules.push((REDIS_STREAMS, &["xadd"][..], rationale(REDIS_STREAMS)));
        }
        "csharp" => {
            rules.push((
                AMAZON_SQS,
                &["SendMessageAsync", "SendMessageBatchAsync"][..],
                rationale(AMAZON_SQS),
            ));
            rules.push((
                APACHE_KAFKA,
                &["Produce", "ProduceAsync"][..],
                rationale(APACHE_KAFKA),
            ));
            rules.push((
                RABBITMQ,
                &["BasicPublish", "BasicPublishAsync"][..],
                rationale(RABBITMQ),
            ));
            rules.push((
                GOOGLE_CLOUD_PUBSUB,
                &["Publish", "PublishAsync"][..],
                rationale(GOOGLE_CLOUD_PUBSUB),
            ));
            rules.push((
                AZURE_SERVICE_BUS,
                &["SendMessageAsync", "SendMessagesAsync"][..],
                rationale(AZURE_SERVICE_BUS),
            ));
            rules.push((NATS, &["PublishAsync"][..], rationale(NATS)));
            rules.push((
                REDIS_STREAMS,
                &["StreamAdd", "StreamAddAsync"][..],
                rationale(REDIS_STREAMS),
            ));
        }
        "go" => {
            rules.push((
                AMAZON_SQS,
                &["SendMessage", "SendMessageBatch"][..],
                rationale(AMAZON_SQS),
            ));
            rules.push((
                APACHE_KAFKA,
                &[
                    "SendMessage",
                    "SendMessages",
                    "WriteMessages",
                    "Produce",
                    "ProduceSync",
                ][..],
                rationale(APACHE_KAFKA),
            ));
            rules.push((
                RABBITMQ,
                &["Publish", "PublishWithContext"][..],
                rationale(RABBITMQ),
            ));
            rules.push((
                GOOGLE_CLOUD_PUBSUB,
                &["Publish"][..],
                rationale(GOOGLE_CLOUD_PUBSUB),
            ));
            rules.push((
                AZURE_SERVICE_BUS,
                &["SendMessage", "SendMessages"][..],
                rationale(AZURE_SERVICE_BUS),
            ));
            rules.push((
                NATS,
                &["Publish", "PublishMsg", "PublishRequest", "PublishAsync"][..],
                rationale(NATS),
            ));
            rules.push((REDIS_STREAMS, &["XAdd"][..], rationale(REDIS_STREAMS)));
        }
        "ruby" => {
            rules.push((
                AMAZON_SQS,
                &["send_message", "send_message_batch"][..],
                rationale(AMAZON_SQS),
            ));
            rules.push((
                APACHE_KAFKA,
                &["produce", "produce_sync", "deliver_messages"][..],
                rationale(APACHE_KAFKA),
            ));
            rules.push((RABBITMQ, &["publish"][..], rationale(RABBITMQ)));
            rules.push((
                GOOGLE_CLOUD_PUBSUB,
                &["publish", "publish_async"][..],
                rationale(GOOGLE_CLOUD_PUBSUB),
            ));
            rules.push((NATS, &["publish"][..], rationale(NATS)));
            rules.push((REDIS_STREAMS, &["xadd"][..], rationale(REDIS_STREAMS)));
        }
        "php" => {
            rules.push((
                AMAZON_SQS,
                &["sendMessage", "sendMessageBatch"][..],
                rationale(AMAZON_SQS),
            ));
            rules.push((APACHE_KAFKA, &["produce"][..], rationale(APACHE_KAFKA)));
            rules.push((RABBITMQ, &["basic_publish"][..], rationale(RABBITMQ)));
            rules.push((
                GOOGLE_CLOUD_PUBSUB,
                &["publish", "publishBatch"][..],
                rationale(GOOGLE_CLOUD_PUBSUB),
            ));
            rules.push((REDIS_STREAMS, &["xadd"][..], rationale(REDIS_STREAMS)));
        }
        "rust" => {
            rules.push((APACHE_KAFKA, &["send"][..], rationale(APACHE_KAFKA)));
            rules.push((RABBITMQ, &["basic_publish"][..], rationale(RABBITMQ)));
            rules.push((NATS, &["publish"][..], rationale(NATS)));
            rules.push((REDIS_STREAMS, &["xadd"][..], rationale(REDIS_STREAMS)));
        }
        _ => {}
    }
    rules
}

fn external_call_rules(
    language: &str,
) -> Vec<(&'static str, &'static [&'static str], &'static str)> {
    let mut rules = Vec::new();
    match language {
        "python" => {
            rules.push((
                PYTHON_REQUESTS,
                REQUESTS_METHODS,
                "Requests sends an HTTP request to an external endpoint.",
            ));
            rules.push((
                PYTHON_HTTPX,
                HTTPX_METHODS,
                "HTTPX sends an HTTP request to an external endpoint.",
            ));
        }
        "javascript" | "jsx" | "typescript" | "tsx" => {
            rules.push((
                WEB_FETCH,
                &["fetch"][..],
                "Fetch starts an HTTP request through an explicit global receiver.",
            ));
            rules.push((
                AXIOS,
                AXIOS_METHODS,
                "Axios sends an HTTP request to an external endpoint.",
            ));
        }
        "java" | "kotlin" => {
            rules.push((
                JAVA_HTTP_CLIENT,
                &["send", "sendAsync"][..],
                "Java HttpClient sends a reviewed HttpRequest.",
            ));
            rules.push((
                SPRING_WEBCLIENT,
                &["retrieve", "exchangeToMono", "exchangeToFlux"][..],
                "Spring WebClient exchanges a prepared HTTP request.",
            ));
        }
        "go" => rules.push((
            GO_NET_HTTP,
            &["Do", "Get", "Head", "Post", "PostForm"][..],
            "Go net/http Client sends an HTTP request.",
        )),
        "csharp" => rules.push((
            DOTNET_HTTP_CLIENT,
            &[
                "Send",
                "SendAsync",
                "GetAsync",
                "GetByteArrayAsync",
                "GetStreamAsync",
                "GetStringAsync",
                "PostAsync",
                "PutAsync",
                "PatchAsync",
                "DeleteAsync",
            ][..],
            ".NET HttpClient sends an HTTP request to an external endpoint.",
        )),
        _ => {}
    }
    rules
}

type ObjectStorageRule = (
    &'static str,
    &'static [&'static str],
    Option<SuggestedFactKind>,
    &'static str,
);

fn object_storage_rules(language: &str) -> Vec<ObjectStorageRule> {
    let write = Some(SuggestedFactKind::ObjectWrite);
    let mut rules = Vec::new();
    match language {
        "python" => {
            rules.push((
                AMAZON_S3,
                &["put_object", "upload_file", "upload_fileobj"][..],
                write,
                "Amazon S3 put or upload API writes an object.",
            ));
            rules.push((
                GOOGLE_CLOUD_STORAGE,
                &[
                    "upload_from_filename",
                    "upload_from_file",
                    "upload_from_string",
                ][..],
                write,
                "Google Cloud Storage Blob upload API writes an object.",
            ));
            rules.push((
                AZURE_BLOB_STORAGE,
                &["upload_blob"][..],
                write,
                "Azure BlobClient upload API writes a blob.",
            ));
        }
        "javascript" | "jsx" | "typescript" | "tsx" => {
            rules.push((
                AMAZON_S3,
                &["send"][..],
                None,
                "S3Client send can execute read or write commands; inspect the command before choosing a kind.",
            ));
            rules.push((
                GOOGLE_CLOUD_STORAGE,
                &["save"][..],
                write,
                "Google Cloud Storage File save API writes an object.",
            ));
            rules.push((
                AZURE_BLOB_STORAGE,
                &["upload", "uploadData", "uploadFile", "uploadStream"][..],
                write,
                "Azure BlobClient upload API writes a blob.",
            ));
        }
        "java" | "kotlin" => {
            rules.push((
                AMAZON_S3,
                &["putObject"][..],
                write,
                "Amazon S3 putObject writes an object.",
            ));
            rules.push((
                GOOGLE_CLOUD_STORAGE,
                &["create", "createFrom"][..],
                write,
                "Google Cloud Storage create API writes an object.",
            ));
            rules.push((
                AZURE_BLOB_STORAGE,
                &["upload", "uploadFromFile", "uploadWithResponse"][..],
                write,
                "Azure BlobClient upload API writes a blob.",
            ));
        }
        "csharp" => {
            rules.push((
                AMAZON_S3,
                &["PutObject", "PutObjectAsync"][..],
                write,
                "Amazon S3 PutObject API writes an object.",
            ));
            rules.push((
                GOOGLE_CLOUD_STORAGE,
                &["UploadObject", "UploadObjectAsync"][..],
                write,
                "Google Cloud Storage upload API writes an object.",
            ));
            rules.push((
                AZURE_BLOB_STORAGE,
                &[
                    "Upload",
                    "UploadAsync",
                    "UploadFromUri",
                    "UploadFromUriAsync",
                ][..],
                write,
                "Azure BlobClient upload API writes a blob.",
            ));
        }
        "go" => {
            rules.push((
                AMAZON_S3,
                &["PutObject"][..],
                write,
                "Amazon S3 PutObject API writes an object.",
            ));
            rules.push((
                AZURE_BLOB_STORAGE,
                &["UploadBuffer", "UploadFile", "UploadStream"][..],
                write,
                "Azure Blob Storage upload API writes a blob.",
            ));
        }
        "ruby" | "rust" => rules.push((
            AMAZON_S3,
            &["put_object"][..],
            write,
            "Amazon S3 put_object writes an object.",
        )),
        "php" => {
            rules.push((
                AMAZON_S3,
                &["putObject"][..],
                write,
                "Amazon S3 putObject writes an object.",
            ));
            rules.push((
                GOOGLE_CLOUD_STORAGE,
                &["upload"][..],
                write,
                "Google Cloud Storage Bucket upload API writes an object.",
            ));
        }
        _ => {}
    }
    rules
}

fn db_rule(framework: &'static str, rationale: &'static str) -> FrameworkRule {
    FrameworkRule {
        framework,
        suggested_fact_kind: Some(SuggestedFactKind::DbWrite),
        rationale,
    }
}

fn message_rule(framework: &'static str, rationale: &'static str) -> FrameworkRule {
    FrameworkRule {
        framework,
        suggested_fact_kind: Some(SuggestedFactKind::MessagePublish),
        rationale,
    }
}

fn external_call_rule(framework: &'static str, rationale: &'static str) -> FrameworkRule {
    FrameworkRule {
        framework,
        suggested_fact_kind: Some(SuggestedFactKind::ExternalCall),
        rationale,
    }
}

const REQUESTS_METHODS: &[&str] = &[
    "request", "send", "get", "options", "head", "post", "put", "patch", "delete",
];

const HTTPX_METHODS: &[&str] = &[
    "request", "send", "stream", "get", "options", "head", "post", "put", "patch", "delete",
];

const AXIOS_METHODS: &[&str] = &[
    "request", "get", "options", "head", "post", "put", "patch", "delete",
];

fn source_evidence(
    framework: &str,
    path: &str,
    source: &str,
    resource: &str,
    method: &str,
) -> BTreeSet<String> {
    let resource_lower = resource.to_ascii_lowercase();
    let path_lower = path.to_ascii_lowercase();
    let mut evidence = BTreeSet::new();
    let marker_found = match framework {
        DJANGO => {
            source.contains("from django")
                || source.contains("import django")
                || source.contains("models.model")
                || resource_lower.contains(".objects")
        }
        SQLALCHEMY => {
            source.contains("sqlalchemy")
                || source.contains("asyncsession")
                || resource_lower.contains("session")
        }
        PRISMA => {
            source.contains("@prisma/client")
                || source.contains("prismaclient")
                || resource_lower.starts_with("prisma.")
        }
        SPRING_DATA_JPA => {
            source.contains("org.springframework.data")
                || source.contains("jakarta.persistence")
                || source.contains("javax.persistence")
                || source.contains("entitymanager")
                || resource_lower.ends_with("repository")
        }
        EF_CORE => {
            source.contains("microsoft.entityframeworkcore")
                || source.contains("dbcontext")
                || matches!(
                    method,
                    "SaveChanges"
                        | "SaveChangesAsync"
                        | "ExecuteUpdate"
                        | "ExecuteUpdateAsync"
                        | "ExecuteDelete"
                        | "ExecuteDeleteAsync"
                )
        }
        RAILS_ACTIVE_RECORD => {
            source.contains("activerecord")
                || source.contains("applicationrecord")
                || path_lower.starts_with("app/models/")
        }
        LARAVEL_ELOQUENT => {
            source.contains("illuminate\\database\\eloquent")
                || source.contains("app\\models\\")
                || path_lower.starts_with("app/models/")
        }
        GORM => {
            source.contains("gorm.io/gorm")
                || source.contains("*gorm.db")
                || source.contains("gorm.g[")
        }
        AMAZON_SQS => {
            source.contains("@aws-sdk/client-sqs")
                || source.contains("software.amazon.awssdk.services.sqs")
                || source.contains("amazon.sqs")
                || source.contains("aws::sqs")
                || source.contains("aws\\sqs")
                || source.contains("awssdk.sqs")
                || source.contains("service/sqs")
                || source.contains("client(\"sqs\")")
                || source.contains("client('sqs')")
                || source.contains("resource(\"sqs\")")
                || source.contains("resource('sqs')")
                || resource_lower.contains("sqs")
        }
        APACHE_KAFKA => {
            source.contains("kafka")
                || source.contains("sarama")
                || source.contains("rdkafka")
                || source.contains("franz-go")
                || resource_lower.contains("kafka")
        }
        RABBITMQ => {
            source.contains("rabbitmq")
                || source.contains("pika")
                || source.contains("aio_pika")
                || source.contains("amqplib")
                || source.contains("amqp091")
                || source.contains("phpamqplib")
                || source.contains("lapin")
        }
        CELERY => {
            source.contains("celery")
                || source.contains("@shared_task")
                || source.contains("@app.task")
        }
        GOOGLE_CLOUD_PUBSUB => {
            source.contains("google.cloud.pubsub")
                || source.contains("from google.cloud import pubsub")
                || source.contains("google/cloud/pubsub")
                || source.contains("google\\cloud\\pubsub")
                || source.contains("google-cloud-pubsub")
                || source.contains("@google-cloud/pubsub")
                || source.contains("cloud.google.com/go/pubsub")
                || resource_lower.contains("pubsub")
        }
        AZURE_SERVICE_BUS => {
            source.contains("azure.messaging.servicebus")
                || source.contains("@azure/service-bus")
                || source.contains("azure.servicebus")
                || source.contains("azservicebus")
                || resource_lower.contains("servicebus")
        }
        NATS => source.contains("nats") || resource_lower.contains("jetstream"),
        REDIS_STREAMS => {
            source.contains("redis") || source.contains("jedis") || resource_lower.contains("redis")
        }
        PYTHON_REQUESTS => {
            source.contains("import requests")
                || source.contains("from requests")
                || resource_lower == "requests"
        }
        PYTHON_HTTPX => {
            source.contains("import httpx")
                || source.contains("from httpx")
                || resource_lower == "httpx"
        }
        WEB_FETCH => matches!(resource_lower.as_str(), "window" | "globalthis" | "self"),
        AXIOS => {
            source.contains("from \"axios\"")
                || source.contains("from 'axios'")
                || source.contains("require(\"axios\")")
                || source.contains("require('axios')")
                || resource_lower == "axios"
        }
        JAVA_HTTP_CLIENT => {
            source.contains("java.net.http.httpclient") || resource_lower.contains("httpclient")
        }
        SPRING_WEBCLIENT => {
            source.contains("org.springframework.web.reactive.function.client.webclient")
                || resource_lower.contains("webclient")
        }
        GO_NET_HTTP => source.contains("\"net/http\"") || resource_lower == "http.defaultclient",
        DOTNET_HTTP_CLIENT => {
            source.contains("system.net.http") || resource_lower.contains("httpclient")
        }
        AMAZON_S3 => {
            source.contains("@aws-sdk/client-s3")
                || source.contains("@aws-sdk/lib-storage")
                || source.contains("software.amazon.awssdk.services.s3")
                || source.contains("amazon.s3")
                || source.contains("aws::s3")
                || source.contains("aws\\s3")
                || source.contains("awssdk.s3")
                || source.contains("service/s3")
                || source.contains("client(\"s3\")")
                || source.contains("client('s3')")
                || source.contains("resource(\"s3\")")
                || source.contains("resource('s3')")
                || resource_lower.contains("s3")
        }
        GOOGLE_CLOUD_STORAGE => {
            source.contains("google.cloud.storage")
                || source.contains("from google.cloud import storage")
                || source.contains("google/cloud/storage")
                || source.contains("google\\cloud\\storage")
                || source.contains("google-cloud-storage")
                || source.contains("@google-cloud/storage")
                || source.contains("cloud.google.com/go/storage")
        }
        AZURE_BLOB_STORAGE => {
            source.contains("azure.storage.blob")
                || source.contains("azure.storage.blobs")
                || source.contains("@azure/storage-blob")
                || source.contains("azblob")
                || resource_lower.contains("blobclient")
        }
        _ => false,
    };
    if marker_found {
        evidence.insert(format!("source-marker:{framework}"));
    }
    evidence
}

const DJANGO_METHODS: &[&str] = &[
    "save",
    "asave",
    "delete",
    "adelete",
    "create",
    "acreate",
    "bulk_create",
    "abulk_create",
    "bulk_update",
    "abulk_update",
    "update",
    "aupdate",
    "update_or_create",
    "aupdate_or_create",
    "get_or_create",
    "aget_or_create",
];

const SQLALCHEMY_WRITE_METHODS: &[&str] = &[
    "add",
    "add_all",
    "delete",
    "flush",
    "commit",
    "merge",
    "bulk_save_objects",
    "bulk_insert_mappings",
    "bulk_update_mappings",
];

const PRISMA_METHODS: &[&str] = &[
    "create",
    "createMany",
    "createManyAndReturn",
    "update",
    "updateMany",
    "updateManyAndReturn",
    "upsert",
    "delete",
    "deleteMany",
    "$executeRaw",
    "$executeRawUnsafe",
];

const SPRING_DATA_JPA_METHODS: &[&str] = &[
    "save",
    "saveAll",
    "saveAndFlush",
    "saveAllAndFlush",
    "delete",
    "deleteById",
    "deleteAll",
    "deleteAllById",
    "deleteAllInBatch",
    "deleteAllByIdInBatch",
    "persist",
    "merge",
    "remove",
    "flush",
];

const EF_CORE_METHODS: &[&str] = &[
    "SaveChanges",
    "SaveChangesAsync",
    "ExecuteUpdate",
    "ExecuteUpdateAsync",
    "ExecuteDelete",
    "ExecuteDeleteAsync",
    "ExecuteSql",
    "ExecuteSqlAsync",
    "ExecuteSqlRaw",
    "ExecuteSqlRawAsync",
    "ExecuteSqlInterpolated",
    "ExecuteSqlInterpolatedAsync",
];

const RAILS_ACTIVE_RECORD_METHODS: &[&str] = &[
    "save",
    "save!",
    "create",
    "create!",
    "update",
    "update!",
    "update_attribute",
    "update_attribute!",
    "update_columns",
    "destroy",
    "destroy!",
    "delete",
    "insert",
    "insert!",
    "insert_all",
    "insert_all!",
    "upsert",
    "upsert_all",
    "update_all",
    "delete_all",
    "destroy_all",
    "destroy_by",
    "delete_by",
    "find_or_create_by",
    "find_or_create_by!",
    "create_or_find_by",
    "create_or_find_by!",
    "touch",
    "increment!",
    "decrement!",
    "toggle!",
];

const LARAVEL_ELOQUENT_METHODS: &[&str] = &[
    "save",
    "saveOrFail",
    "saveQuietly",
    "create",
    "createQuietly",
    "forceCreate",
    "forceCreateQuietly",
    "firstOrCreate",
    "createOrFirst",
    "incrementOrCreate",
    "update",
    "updateQuietly",
    "updateOrCreate",
    "upsert",
    "delete",
    "deleteQuietly",
    "destroy",
    "forceDelete",
    "forceDeleteQuietly",
    "restore",
    "restoreQuietly",
    "insert",
    "insertOrIgnore",
    "insertUsing",
    "updateOrInsert",
    "increment",
    "decrement",
    "saveMany",
    "createMany",
    "createManyQuietly",
];

const GORM_METHODS: &[&str] = &[
    "Create",
    "CreateInBatches",
    "Save",
    "Update",
    "Updates",
    "UpdateColumn",
    "UpdateColumns",
    "Delete",
    "Exec",
    "FirstOrCreate",
];

#[cfg(test)]
mod tests {
    use super::*;

    fn observation(method: &str) -> SourceObservation {
        observation_on("client", method)
    }

    fn observation_on(resource: &str, method: &str) -> SourceObservation {
        SourceObservation {
            kind: SourceObservationKind::OtherMethodCall,
            symbol: "place_order".to_owned(),
            resource: resource.to_owned(),
            method: method.to_owned(),
            line: 7,
        }
    }

    #[test]
    fn requires_framework_evidence_before_suggesting_a_binding() {
        let catalog = FrameworkCatalog::default();
        assert!(
            catalog
                .candidates("src/service.py", "python", "", &[observation("save")])
                .is_empty()
        );
    }

    #[test]
    fn covers_the_eight_reviewed_orm_families() {
        let fixtures = [
            ("python", "from django.db import models", "save", DJANGO),
            (
                "python",
                "from sqlalchemy.orm import Session",
                "add",
                SQLALCHEMY,
            ),
            (
                "typescript",
                "import { PrismaClient } from '@prisma/client'",
                "create",
                PRISMA,
            ),
            (
                "java",
                "import org.springframework.data.repository.CrudRepository;",
                "save",
                SPRING_DATA_JPA,
            ),
            (
                "csharp",
                "using Microsoft.EntityFrameworkCore;",
                "SaveChangesAsync",
                EF_CORE,
            ),
            (
                "ruby",
                "class Order < ApplicationRecord",
                "save!",
                RAILS_ACTIVE_RECORD,
            ),
            (
                "php",
                "use Illuminate\\Database\\Eloquent\\Model;",
                "save",
                LARAVEL_ELOQUENT,
            ),
            ("go", "import \"gorm.io/gorm\"", "Create", GORM),
        ];
        let catalog = FrameworkCatalog::default();
        for (language, source, method, framework) in fixtures {
            let observation = if framework == PRISMA {
                observation_on("prisma.order", method)
            } else {
                observation(method)
            };
            let candidates = catalog.candidates("src/service", language, source, &[observation]);
            assert_eq!(candidates.len(), 1, "{framework}");
            assert_eq!(candidates[0].framework, framework);
            assert_eq!(
                candidates[0].suggested_fact_kinds,
                vec![SuggestedFactKind::DbWrite]
            );
            assert!(candidates[0].method_binding_required);
        }
    }

    #[test]
    fn covers_the_eight_reviewed_messaging_families() {
        let fixtures = [
            (
                "python",
                "import boto3\nsqs = boto3.client('sqs')",
                "send_message",
                AMAZON_SQS,
            ),
            (
                "java",
                "import org.apache.kafka.clients.producer.KafkaProducer;",
                "send",
                APACHE_KAFKA,
            ),
            (
                "typescript",
                "import amqp from 'amqplib'",
                "sendToQueue",
                RABBITMQ,
            ),
            ("python", "from celery import shared_task", "delay", CELERY),
            (
                "ruby",
                "require \"google/cloud/pubsub\"",
                "publish_async",
                GOOGLE_CLOUD_PUBSUB,
            ),
            (
                "csharp",
                "using Azure.Messaging.ServiceBus;",
                "SendMessagesAsync",
                AZURE_SERVICE_BUS,
            ),
            (
                "go",
                "import \"github.com/nats-io/nats.go\"",
                "Publish",
                NATS,
            ),
            (
                "typescript",
                "import { createClient } from 'redis'",
                "xAdd",
                REDIS_STREAMS,
            ),
        ];
        let catalog = FrameworkCatalog::default();
        for (language, source, method, framework) in fixtures {
            let candidates =
                catalog.candidates("src/service", language, source, &[observation(method)]);
            assert_eq!(candidates.len(), 1, "{framework}");
            assert_eq!(candidates[0].framework, framework);
            assert_eq!(
                candidates[0].suggested_fact_kinds,
                vec![SuggestedFactKind::MessagePublish]
            );
        }
    }

    #[test]
    fn covers_the_eight_reviewed_external_call_families() {
        let fixtures = [
            (
                "python",
                "import requests",
                "requests",
                "post",
                PYTHON_REQUESTS,
            ),
            ("python", "import httpx", "httpx", "get", PYTHON_HTTPX),
            ("typescript", "", "window", "fetch", WEB_FETCH),
            (
                "typescript",
                "import axios from 'axios'",
                "axios",
                "post",
                AXIOS,
            ),
            (
                "java",
                "import java.net.http.HttpClient;",
                "httpClient",
                "sendAsync",
                JAVA_HTTP_CLIENT,
            ),
            (
                "kotlin",
                "import org.springframework.web.reactive.function.client.WebClient",
                "webClient.get().uri(\"/orders\")",
                "retrieve",
                SPRING_WEBCLIENT,
            ),
            ("go", "import \"net/http\"", "client", "Do", GO_NET_HTTP),
            (
                "csharp",
                "using System.Net.Http;",
                "httpClient",
                "SendAsync",
                DOTNET_HTTP_CLIENT,
            ),
        ];
        let catalog = FrameworkCatalog::default();
        for (language, source, resource, method, framework) in fixtures {
            let candidates = catalog.candidates(
                "src/service",
                language,
                source,
                &[observation_on(resource, method)],
            );
            assert_eq!(candidates.len(), 1, "{framework}");
            assert_eq!(candidates[0].framework, framework);
            assert_eq!(
                candidates[0].suggested_fact_kinds,
                vec![SuggestedFactKind::ExternalCall]
            );
            assert!(candidates[0].method_binding_required);
        }
    }

    #[test]
    fn covers_three_reviewed_object_storage_families() {
        let fixtures = [
            (
                "python",
                "import boto3\ns3 = boto3.client('s3')",
                "s3",
                "put_object",
                AMAZON_S3,
            ),
            (
                "typescript",
                "import { Storage } from '@google-cloud/storage'",
                "file",
                "save",
                GOOGLE_CLOUD_STORAGE,
            ),
            (
                "csharp",
                "using Azure.Storage.Blobs;",
                "blobClient",
                "UploadAsync",
                AZURE_BLOB_STORAGE,
            ),
        ];
        let catalog = FrameworkCatalog::default();
        for (language, source, resource, method, framework) in fixtures {
            let candidates = catalog.candidates(
                "src/service",
                language,
                source,
                &[observation_on(resource, method)],
            );
            assert_eq!(candidates.len(), 1, "{framework}");
            assert_eq!(candidates[0].framework, framework);
            assert_eq!(
                candidates[0].suggested_fact_kinds,
                vec![
                    SuggestedFactKind::ExternalCall,
                    SuggestedFactKind::ObjectWrite,
                ]
            );
        }
    }

    #[test]
    fn s3_javascript_send_requires_command_specific_review() {
        let catalog = FrameworkCatalog::default();
        let candidates = catalog.candidates(
            "src/service.ts",
            "typescript",
            "import { S3Client, PutObjectCommand } from '@aws-sdk/client-s3'",
            &[observation_on("s3", "send")],
        );
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].framework, AMAZON_S3);
        assert!(candidates[0].suggested_fact_kinds.is_empty());
    }

    #[test]
    fn ambiguous_send_without_framework_evidence_is_not_suggested() {
        let catalog = FrameworkCatalog::default();
        assert!(
            catalog
                .candidates("src/service.ts", "typescript", "", &[observation("send")])
                .is_empty()
        );
    }

    #[test]
    fn fetch_requires_an_explicit_global_receiver() {
        let catalog = FrameworkCatalog::default();
        assert!(
            catalog
                .candidates(
                    "src/service.ts",
                    "typescript",
                    "fetch('/orders')",
                    &[observation_on("client", "fetch")],
                )
                .is_empty()
        );
    }

    #[test]
    fn project_manifest_enables_candidates_without_local_imports() {
        let mut catalog = FrameworkCatalog::default();
        catalog.record_manifest("pyproject.toml", "dependencies = [\"Django>=6\"]");
        let candidates =
            catalog.candidates("shop/service.py", "python", "", &[observation("save")]);
        assert_eq!(candidates[0].framework, DJANGO);
        assert_eq!(
            candidates[0].evidence,
            vec!["project-manifest:pyproject.toml"]
        );

        let mut catalog = FrameworkCatalog::default();
        catalog.record_manifest(
            "composer.json",
            r#"{"require":{"google/cloud-pubsub":"^1"}}"#,
        );
        let candidates =
            catalog.candidates("app/service.php", "php", "", &[observation("publish")]);
        assert_eq!(candidates[0].framework, GOOGLE_CLOUD_PUBSUB);
        assert_eq!(
            candidates[0].evidence,
            vec!["project-manifest:composer.json"]
        );

        let mut catalog = FrameworkCatalog::default();
        catalog.record_manifest(
            "package.json",
            r#"{"dependencies":{"axios":"^1","@azure/storage-blob":"^12"}}"#,
        );
        let candidates = catalog.candidates(
            "src/service.ts",
            "typescript",
            "",
            &[observation_on("httpClient", "post")],
        );
        assert_eq!(candidates[0].framework, AXIOS);
        assert_eq!(
            candidates[0].suggested_fact_kinds,
            vec![SuggestedFactKind::ExternalCall]
        );
        let candidates = catalog.candidates(
            "src/archive.ts",
            "typescript",
            "",
            &[observation_on("blobClient", "uploadData")],
        );
        assert_eq!(candidates[0].framework, AZURE_BLOB_STORAGE);
        assert_eq!(
            candidates[0].suggested_fact_kinds,
            vec![
                SuggestedFactKind::ExternalCall,
                SuggestedFactKind::ObjectWrite,
            ]
        );
    }

    #[test]
    fn nested_manifests_do_not_leak_framework_evidence_to_siblings() {
        let mut catalog = FrameworkCatalog::default();
        catalog.record_manifest(
            "apps/prisma/package.json",
            r#"{"dependencies":{"@prisma/client":"latest"}}"#,
        );

        assert!(
            catalog
                .candidates(
                    "apps/cache/service.ts",
                    "typescript",
                    "caches.delete(key)",
                    &[observation_on("caches", "delete")],
                )
                .is_empty()
        );
        assert_eq!(
            catalog
                .candidates(
                    "apps/prisma/service.ts",
                    "typescript",
                    "prisma.order.delete({ where: { id } })",
                    &[observation_on("prisma.order", "delete")],
                )
                .len(),
            1
        );
    }

    #[test]
    fn prisma_requires_a_receiver_or_explicit_client_alias() {
        let mut catalog = FrameworkCatalog::default();
        catalog.record_manifest(
            "package.json",
            r#"{"dependencies":{"@prisma/client":"latest"}}"#,
        );
        assert!(
            catalog
                .candidates(
                    "src/main.ts",
                    "typescript",
                    "NestFactory.create(AppModule)",
                    &[observation_on("NestFactory", "create")],
                )
                .is_empty()
        );
        assert_eq!(
            catalog
                .candidates(
                    "src/main.ts",
                    "typescript",
                    "const db = new PrismaClient(); db.order.create({ data })",
                    &[observation_on("db.order", "create")],
                )
                .len(),
            1
        );
    }

    #[test]
    fn django_does_not_suggest_explicit_python_collection_updates() {
        let mut catalog = FrameworkCatalog::default();
        catalog.record_manifest("pyproject.toml", "dependencies = [\"Django>=6\"]");
        let source = "d = {}\nd.update(value)\nself.__dict__.update(value)\n";
        assert!(
            catalog
                .candidates(
                    "shop/service.py",
                    "python",
                    source,
                    &[
                        observation_on("d", "update"),
                        observation_on("self.__dict__", "update"),
                    ],
                )
                .is_empty()
        );
    }

    #[test]
    fn sqlalchemy_execute_never_invents_a_write_classification() {
        let catalog = FrameworkCatalog::default();
        let candidates = catalog.candidates(
            "src/service.py",
            "python",
            "from sqlalchemy.orm import Session",
            &[observation("execute")],
        );
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].framework, SQLALCHEMY);
        assert!(candidates[0].suggested_fact_kinds.is_empty());
    }

    #[test]
    fn existing_builtin_classification_does_not_request_a_method_binding() {
        let catalog = FrameworkCatalog::default();
        let mut builtin = observation("update");
        builtin.kind = SourceObservationKind::DbWrite;
        let candidates = catalog.candidates(
            "src/service.py",
            "python",
            "from django.db import models",
            &[builtin],
        );
        assert!(!candidates[0].method_binding_required);
    }

    #[test]
    fn release_catalog_adds_namespaced_review_candidates() {
        let source = serde_json::json!({
            "schema_version": "1",
            "namespace": "example.queue",
            "rules": [{
                "id": "publisher",
                "framework": "client",
                "languages": ["python"],
                "methods": ["enqueue"],
                "manifest_markers": ["example-queue"],
                "source_markers": ["import example_queue"],
                "receiver_markers": ["queue"],
                "suggested_fact_kinds": ["message_publish"],
                "rationale": "Example Queue enqueue publishes a broker message."
            }]
        });
        let mut catalog = FrameworkCatalog::with_release_source(&source).unwrap();
        catalog.record_manifest("pyproject.toml", "dependencies = ['example-queue']");
        let candidates = catalog.candidates(
            "src/jobs.py",
            "python",
            "queue.enqueue(job)",
            &[observation_on("queue", "enqueue")],
        );
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].framework, "example.queue/client");
        assert_eq!(
            candidates[0].suggested_fact_kinds,
            vec![SuggestedFactKind::MessagePublish]
        );
        assert!(candidates[0].method_binding_required);
    }

    #[test]
    fn release_catalog_rejects_reserved_namespaces_and_duplicate_matches() {
        let reserved = serde_json::json!({
            "schema_version": "1",
            "namespace": "agentic",
            "rules": []
        });
        assert!(
            FrameworkCatalog::with_release_source(&reserved)
                .unwrap_err()
                .to_string()
                .contains("reserved")
        );

        let duplicate = serde_json::json!({
            "schema_version": "1",
            "namespace": "example.queue",
            "rules": [
                {
                    "id": "first",
                    "framework": "client",
                    "languages": ["python"],
                    "methods": ["enqueue"],
                    "source_markers": ["example_queue"],
                    "rationale": "First rule."
                },
                {
                    "id": "second",
                    "framework": "client",
                    "languages": ["python"],
                    "methods": ["enqueue"],
                    "source_markers": ["example_queue"],
                    "rationale": "Second rule."
                }
            ]
        });
        assert!(
            FrameworkCatalog::with_release_source(&duplicate)
                .unwrap_err()
                .to_string()
                .contains("duplicate Framework Catalog match")
        );
    }
}
