//! Non-authoritative framework hints for reviewed method bindings.
//!
//! These rules never affect repository facts or coverage. They only annotate
//! `project observe` drafts with mechanically explainable candidates that a
//! reviewer may turn into an accepted Binding Record.

use crate::source_detection::{SourceObservation, SourceObservationKind};
use std::collections::{BTreeMap, BTreeSet};

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

// Method vocabularies follow the projects' current official persistence and
// publishing references. Keep uncertain dual-use APIs as `suggested_kind: None`.
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

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct FrameworkCandidate {
    pub framework: &'static str,
    pub symbol: String,
    pub resource: String,
    pub method: String,
    pub line: usize,
    pub binding_key: String,
    pub suggested_kind: Option<SourceObservationKind>,
    pub method_binding_required: bool,
    pub evidence: Vec<String>,
    pub rationale: &'static str,
}

#[derive(Debug, Default)]
pub struct FrameworkCatalog {
    project_evidence: BTreeMap<&'static str, BTreeSet<String>>,
}

impl FrameworkCatalog {
    pub fn record_manifest(&mut self, path: &str, content: &str) {
        let content = content.to_ascii_lowercase();
        for (framework, markers) in [
            (DJANGO, &["django"][..]),
            (SQLALCHEMY, &["sqlalchemy"][..]),
            (PRISMA, &["@prisma/client", "\"prisma\""][..]),
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
        ] {
            if markers.iter().any(|marker| content.contains(marker)) {
                self.project_evidence
                    .entry(framework)
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
                    .cloned()
                    .unwrap_or_default();
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
                candidates.push(FrameworkCandidate {
                    framework: rule.framework,
                    symbol: observation.symbol.clone(),
                    resource: observation.resource.clone(),
                    method: observation.method.clone(),
                    line: observation.line,
                    binding_key: format!("{}.{}", observation.resource, observation.method),
                    suggested_kind: rule.suggested_kind,
                    method_binding_required: observation.kind
                        == SourceObservationKind::OtherMethodCall,
                    evidence: evidence.into_iter().collect(),
                    rationale: rule.rationale,
                });
            }
        }
        candidates.sort();
        candidates
    }
}

#[derive(Clone, Copy)]
struct FrameworkRule {
    framework: &'static str,
    suggested_kind: Option<SourceObservationKind>,
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
                    suggested_kind: None,
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

fn db_rule(framework: &'static str, rationale: &'static str) -> FrameworkRule {
    FrameworkRule {
        framework,
        suggested_kind: Some(SourceObservationKind::DbWrite),
        rationale,
    }
}

fn message_rule(framework: &'static str, rationale: &'static str) -> FrameworkRule {
    FrameworkRule {
        framework,
        suggested_kind: Some(SourceObservationKind::MessagePublish),
        rationale,
    }
}

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
        SourceObservation {
            kind: SourceObservationKind::OtherMethodCall,
            symbol: "place_order".to_owned(),
            resource: "client".to_owned(),
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
            let candidates =
                catalog.candidates("src/service", language, source, &[observation(method)]);
            assert_eq!(candidates.len(), 1, "{framework}");
            assert_eq!(candidates[0].framework, framework);
            assert_eq!(
                candidates[0].suggested_kind,
                Some(SourceObservationKind::DbWrite)
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
                candidates[0].suggested_kind,
                Some(SourceObservationKind::MessagePublish)
            );
        }
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
        assert_eq!(candidates[0].suggested_kind, None);
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
}
