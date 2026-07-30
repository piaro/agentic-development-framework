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

// Method vocabularies follow the projects' current official persistence
// references. Keep uncertain dual-use APIs as `suggested_kind: None`.
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
    rules
}

fn db_rule(framework: &'static str, rationale: &'static str) -> FrameworkRule {
    FrameworkRule {
        framework,
        suggested_kind: Some(SourceObservationKind::DbWrite),
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
