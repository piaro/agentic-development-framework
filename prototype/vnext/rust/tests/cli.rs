use agentic_vnext_rust::schema::validate_json_document;
use agentic_vnext_rust::{canonical_digest, canonical_json};
use ed25519_dalek::{Signer, SigningKey};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{ChildStdin, ChildStdout, Command, Output, Stdio};
use std::thread;

#[test]
fn help_version_and_uninitialized_project_have_actionable_cli_behavior() {
    let help = Command::new(env!("CARGO_BIN_EXE_agentic-vnext-rust"))
        .arg("--help")
        .output()
        .unwrap();
    assert_success(&help);
    assert!(help.stderr.is_empty());
    assert!(String::from_utf8_lossy(&help.stdout).contains("agentic project init"));

    let version = Command::new(env!("CARGO_BIN_EXE_agentic-vnext-rust"))
        .arg("--version")
        .output()
        .unwrap();
    assert_success(&version);
    assert!(version.stderr.is_empty());
    assert!(String::from_utf8_lossy(&version.stdout).starts_with("agentic 0.1.0"));

    let root = temporary_test_root("uninitialized");
    fs::create_dir_all(&root).unwrap();
    run_git(&root, &["init", "--quiet"]);
    let next = Command::new(env!("CARGO_BIN_EXE_agentic-vnext-rust"))
        .args(["next", "change.example", "--project"])
        .arg(&root)
        .output()
        .unwrap();
    assert!(!next.status.success());
    let stderr = String::from_utf8_lossy(&next.stderr);
    assert!(stderr.contains("Project is not initialized"));
    assert!(stderr.contains("agentic project init"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn migration_inspect_reports_current_project_without_writing() {
    let root = legacy_migration_project("migration-current");
    let before = git_output(
        &root,
        &["status", "--porcelain=v1", "--untracked-files=all"],
    );
    assert!(before.is_empty());
    let output = Command::new(env!("CARGO_BIN_EXE_agentic-vnext-rust"))
        .args(["migration", "inspect", "--format", "json", "--project"])
        .arg(&root)
        .output()
        .unwrap();
    assert_success(&output);
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    validate_output_schema(&report, "migration-inspection-report.schema.json");
    assert_eq!(report["source_state"], "current");
    assert_eq!(report["readiness"], "review-required");
    assert_eq!(report["installation"]["kit_version"], "3.0.0");
    assert_eq!(report["inventory"]["contracts"]["files"], 1);
    assert_eq!(report["inventory"]["contracts"]["tracked_files"], 1);
    assert_eq!(report["inventory"]["decisions"]["formats"]["markdown"], 1);
    assert!(
        report["components"]
            .as_array()
            .unwrap()
            .iter()
            .any(|component| component["id"] == "project-source-inventory"
                && component["classification"] == "mechanical")
    );
    assert!(
        report["components"]
            .as_array()
            .unwrap()
            .iter()
            .any(|component| component["id"] == "contracts"
                && component["classification"] == "review-required")
    );
    assert!(
        git_output(
            &root,
            &["status", "--porcelain=v1", "--untracked-files=all"]
        )
        .is_empty()
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn migration_inspect_distinguishes_vnext_and_mixed_projects() {
    let vnext = TestProject::new();
    let output = vnext.run(&["migration", "inspect", "--format", "json"]);
    assert_success(&output);
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    validate_output_schema(&report, "migration-inspection-report.schema.json");
    assert_eq!(report["source_state"], "vnext");
    assert_eq!(report["readiness"], "already-vnext");

    let mixed = legacy_migration_project("migration-mixed");
    fs::write(
        mixed.join(".agentic/framework.lock"),
        "schema_version: '2'\n",
    )
    .unwrap();
    run_git(&mixed, &["add", ".agentic/framework.lock"]);
    run_git(&mixed, &["commit", "--quiet", "-m", "add mixed marker"]);
    let output = Command::new(env!("CARGO_BIN_EXE_agentic-vnext-rust"))
        .args(["migration", "inspect", "--format", "json", "--project"])
        .arg(&mixed)
        .output()
        .unwrap();
    assert_success(&output);
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["source_state"], "mixed");
    assert_eq!(report["readiness"], "blocked");
    assert!(
        report["findings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|finding| finding["category"] == "mixed-runtime-state")
    );
    let _ = fs::remove_dir_all(mixed);
}

#[test]
fn migration_inspect_blocks_a_dirty_revision_but_still_returns_the_report() {
    let root = legacy_migration_project("migration-dirty");
    fs::write(
        root.join("untracked.txt"),
        "not part of the fixed revision\n",
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_agentic-vnext-rust"))
        .args(["migration", "inspect", "--format", "text", "--project"])
        .arg(&root)
        .output()
        .unwrap();
    assert_success(&output);
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(text.contains("Migration inspection: blocked"));
    assert!(text.contains("dirty-worktree"));
    assert!(text.contains("No files were changed"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn migration_draft_describes_reviewed_actions_without_writing() {
    let root = legacy_migration_project("migration-draft");
    let revision = git_output(&root, &["rev-parse", "HEAD"]);
    let output = Command::new(env!("CARGO_BIN_EXE_agentic-vnext-rust"))
        .args(["migration", "draft", "--format", "json", "--project"])
        .arg(&root)
        .output()
        .unwrap();
    assert_success(&output);
    let draft: Value = serde_json::from_slice(&output.stdout).unwrap();
    validate_output_schema(&draft, "migration-draft.schema.json");
    assert_eq!(draft["schema_version"], "2");
    assert_eq!(draft["kind"], "migration-draft");
    assert_eq!(draft["source_revision"], revision);
    assert_eq!(draft["status"], "review-required");
    let actions = draft["actions"].as_array().unwrap();
    assert!(actions.iter().any(|action| {
        action["id"] == "project-source-inventory"
            && action["operation"] == "inventory-only"
            && action["requires_human_review"] == false
    }));
    assert!(actions.iter().any(|action| {
        action["id"] == "contracts"
            && action["operation"] == "transform-after-review"
            && action["requires_human_review"] == true
            && action["review"].is_null()
    }));
    assert!(actions.iter().any(|action| {
        action["id"] == "framework-release" && action["operation"] == "install-from-release"
    }));
    assert!(
        draft["next"]
            .as_str()
            .unwrap()
            .contains("No files were changed")
    );
    assert!(
        git_output(
            &root,
            &["status", "--porcelain=v1", "--untracked-files=all"]
        )
        .is_empty()
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn migration_draft_requires_a_clean_migratable_current_project() {
    let dirty = legacy_migration_project("migration-draft-dirty");
    fs::write(dirty.join("untracked.txt"), "dirty\n").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_agentic-vnext-rust"))
        .args(["migration", "draft", "--project"])
        .arg(&dirty)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("resolve the findings from `agentic migration inspect` first")
    );

    let vnext = TestProject::new();
    let output = vnext.run(&["migration", "draft"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("project already uses vNext"));
    let _ = fs::remove_dir_all(dirty);
}

#[test]
fn migration_validate_draft_accepts_explicit_reviews_and_ignores_only_the_draft() {
    let root = legacy_migration_project("migration-validate-draft");
    let output = Command::new(env!("CARGO_BIN_EXE_agentic-vnext-rust"))
        .args(["migration", "draft", "--format", "json", "--project"])
        .arg(&root)
        .output()
        .unwrap();
    assert_success(&output);
    let mut draft: Value = serde_json::from_slice(&output.stdout).unwrap();
    complete_migration_reviews(&mut draft);
    validate_output_schema(&draft, "migration-draft.schema.json");
    let draft_path = root.join(".agentic/migration-draft.json");
    fs::write(&draft_path, serde_json::to_vec_pretty(&draft).unwrap()).unwrap();
    let before = git_output(
        &root,
        &["status", "--porcelain=v1", "--untracked-files=all"],
    );
    assert!(before.contains(".agentic/migration-draft.json"));

    let output = Command::new(env!("CARGO_BIN_EXE_agentic-vnext-rust"))
        .args([
            "migration",
            "validate-draft",
            "--draft",
            ".agentic/migration-draft.json",
            "--format",
            "json",
            "--project",
        ])
        .arg(&root)
        .output()
        .unwrap();
    assert_success(&output);
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    validate_output_schema(&report, "migration-draft-validation-report.schema.json");
    assert_eq!(report["status"], "valid");
    assert!(report["issues"].as_array().unwrap().is_empty());
    assert_eq!(
        git_output(
            &root,
            &["status", "--porcelain=v1", "--untracked-files=all"]
        ),
        before
    );

    fs::write(root.join("unrelated.txt"), "dirty\n").unwrap();
    let blocked = Command::new(env!("CARGO_BIN_EXE_agentic-vnext-rust"))
        .args([
            "migration",
            "validate-draft",
            "--draft",
            ".agentic/migration-draft.json",
            "--format",
            "json",
            "--project",
        ])
        .arg(&root)
        .output()
        .unwrap();
    assert!(!blocked.status.success());
    let report: Value = serde_json::from_slice(&blocked.stdout).unwrap();
    validate_output_schema(&report, "migration-draft-validation-report.schema.json");
    assert_eq!(report["status"], "blocked");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn migration_validate_draft_rejects_missing_reviews_and_generated_field_changes() {
    let root = legacy_migration_project("migration-invalid-draft");
    let output = Command::new(env!("CARGO_BIN_EXE_agentic-vnext-rust"))
        .args(["migration", "draft", "--format", "json", "--project"])
        .arg(&root)
        .output()
        .unwrap();
    assert_success(&output);
    let mut draft: Value = serde_json::from_slice(&output.stdout).unwrap();
    let draft_path = root.join("migration-draft.json");
    fs::write(&draft_path, serde_json::to_vec_pretty(&draft).unwrap()).unwrap();
    let incomplete = Command::new(env!("CARGO_BIN_EXE_agentic-vnext-rust"))
        .args([
            "migration",
            "validate-draft",
            "--draft",
            "migration-draft.json",
            "--format",
            "json",
            "--project",
        ])
        .arg(&root)
        .output()
        .unwrap();
    assert!(!incomplete.status.success());
    let report: Value = serde_json::from_slice(&incomplete.stdout).unwrap();
    validate_output_schema(&report, "migration-draft-validation-report.schema.json");
    assert!(
        report["issues"]
            .as_array()
            .unwrap()
            .iter()
            .any(|issue| issue["category"] == "missing-review")
    );

    complete_migration_reviews(&mut draft);
    draft["actions"][0]["instruction"] = json!("tampered");
    fs::write(&draft_path, serde_json::to_vec_pretty(&draft).unwrap()).unwrap();
    let tampered = Command::new(env!("CARGO_BIN_EXE_agentic-vnext-rust"))
        .args([
            "migration",
            "validate-draft",
            "--draft",
            "migration-draft.json",
            "--format",
            "json",
            "--project",
        ])
        .arg(&root)
        .output()
        .unwrap();
    assert!(!tampered.status.success());
    let report: Value = serde_json::from_slice(&tampered.stdout).unwrap();
    validate_output_schema(&report, "migration-draft-validation-report.schema.json");
    assert!(
        report["issues"]
            .as_array()
            .unwrap()
            .iter()
            .any(|issue| issue["category"] == "modified-draft")
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn migration_validate_draft_rejects_a_stale_source_revision() {
    let root = legacy_migration_project("migration-stale-draft");
    let output = Command::new(env!("CARGO_BIN_EXE_agentic-vnext-rust"))
        .args(["migration", "draft", "--format", "json", "--project"])
        .arg(&root)
        .output()
        .unwrap();
    assert_success(&output);
    let mut draft: Value = serde_json::from_slice(&output.stdout).unwrap();
    complete_migration_reviews(&mut draft);
    fs::write(
        root.join("migration-draft.json"),
        serde_json::to_vec_pretty(&draft).unwrap(),
    )
    .unwrap();
    fs::write(root.join("new-source.py"), "orders.insert(order)\n").unwrap();
    run_git(&root, &["add", "new-source.py"]);
    run_git(&root, &["commit", "--quiet", "-m", "advance source"]);

    let output = Command::new(env!("CARGO_BIN_EXE_agentic-vnext-rust"))
        .args([
            "migration",
            "validate-draft",
            "--draft",
            "migration-draft.json",
            "--format",
            "json",
            "--project",
        ])
        .arg(&root)
        .output()
        .unwrap();
    assert!(!output.status.success());
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    validate_output_schema(&report, "migration-draft-validation-report.schema.json");
    assert!(
        report["issues"]
            .as_array()
            .unwrap()
            .iter()
            .any(|issue| issue["category"] == "stale-revision")
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn migration_generate_candidate_writes_only_an_isolated_incomplete_bundle() {
    let root = legacy_migration_project("migration-candidate");
    let legacy_config = fs::read(root.join(".agentic/config.yaml")).unwrap();
    let draft_output = Command::new(env!("CARGO_BIN_EXE_agentic-vnext-rust"))
        .args(["migration", "draft", "--format", "json", "--project"])
        .arg(&root)
        .output()
        .unwrap();
    assert_success(&draft_output);
    let mut draft: Value = serde_json::from_slice(&draft_output.stdout).unwrap();
    complete_migration_reviews(&mut draft);
    draft["actions"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|action| action["id"] == "decisions")
        .unwrap()["review"]["decision"] = json!("preserve-history");
    fs::write(
        root.join("migration-draft.json"),
        serde_json::to_vec_pretty(&draft).unwrap(),
    )
    .unwrap();

    let output_relative = ".agentic/migration-candidates/review-1";
    let output = Command::new(env!("CARGO_BIN_EXE_agentic-vnext-rust"))
        .args([
            "migration",
            "generate-candidate",
            "--draft",
            "migration-draft.json",
            "--output",
            output_relative,
            "--format",
            "json",
            "--project",
        ])
        .arg(&root)
        .output()
        .unwrap();
    assert_success(&output);
    let manifest: Value = serde_json::from_slice(&output.stdout).unwrap();
    validate_output_schema(&manifest, "migration-candidate-manifest.schema.json");
    assert_eq!(manifest["status"], "incomplete");
    assert_eq!(manifest["output_root"], output_relative);
    assert!(
        manifest["pending_actions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|action| action["id"] == "contracts")
    );
    assert!(
        manifest["pending_actions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|action| action["id"] == "framework-release")
    );
    assert!(
        manifest["pending_actions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|action| {
                action["id"] == "decisions"
                    && action["decision"] == "preserve-history"
                    && action["target_paths"].as_array().unwrap().is_empty()
            })
    );

    let candidate = root.join(output_relative);
    let stored_manifest = read_yaml(&candidate.join("migration-manifest.yaml"));
    assert_eq!(stored_manifest, manifest);
    let config = read_yaml(&candidate.join(".agentic/config.yaml"));
    assert_eq!(config["schema_version"], "1");
    assert_eq!(config["project_sources"]["contracts"], "contracts");
    let stored_draft = read_yaml(&candidate.join("migration-draft.json"));
    assert_eq!(stored_draft, draft);
    for file in manifest["generated_files"].as_array().unwrap() {
        let bytes = fs::read(candidate.join(file["path"].as_str().unwrap())).unwrap();
        assert_eq!(
            file["digest"],
            format!("sha256:{:x}", Sha256::digest(bytes))
        );
    }
    assert!(!candidate.join(".agentic/framework.lock").exists());
    assert!(!candidate.join("contracts").exists());
    assert_eq!(
        fs::read(root.join(".agentic/config.yaml")).unwrap(),
        legacy_config
    );

    let manifest_before = fs::read(candidate.join("migration-manifest.yaml")).unwrap();
    let repeated = Command::new(env!("CARGO_BIN_EXE_agentic-vnext-rust"))
        .args([
            "migration",
            "generate-candidate",
            "--draft",
            "migration-draft.json",
            "--output",
            output_relative,
            "--project",
        ])
        .arg(&root)
        .output()
        .unwrap();
    assert!(!repeated.status.success());
    assert!(String::from_utf8_lossy(&repeated.stderr).contains("refusing to overwrite"));
    assert_eq!(
        fs::read(candidate.join("migration-manifest.yaml")).unwrap(),
        manifest_before
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn migration_generate_candidate_rejects_unreviewed_drafts_and_unsafe_outputs() {
    let root = legacy_migration_project("migration-candidate-invalid");
    let draft_output = Command::new(env!("CARGO_BIN_EXE_agentic-vnext-rust"))
        .args(["migration", "draft", "--format", "json", "--project"])
        .arg(&root)
        .output()
        .unwrap();
    assert_success(&draft_output);
    let draft: Value = serde_json::from_slice(&draft_output.stdout).unwrap();
    fs::write(
        root.join("migration-draft.json"),
        serde_json::to_vec_pretty(&draft).unwrap(),
    )
    .unwrap();

    let unsafe_output = Command::new(env!("CARGO_BIN_EXE_agentic-vnext-rust"))
        .args([
            "migration",
            "generate-candidate",
            "--draft",
            "migration-draft.json",
            "--output",
            "migration-candidate",
            "--project",
        ])
        .arg(&root)
        .output()
        .unwrap();
    assert!(!unsafe_output.status.success());
    assert!(
        String::from_utf8_lossy(&unsafe_output.stderr)
            .contains("must be under .agentic/migration-candidates")
    );
    assert!(!root.join("migration-candidate").exists());

    let output_relative = ".agentic/migration-candidates/unreviewed";
    let unreviewed = Command::new(env!("CARGO_BIN_EXE_agentic-vnext-rust"))
        .args([
            "migration",
            "generate-candidate",
            "--draft",
            "migration-draft.json",
            "--output",
            output_relative,
            "--project",
        ])
        .arg(&root)
        .output()
        .unwrap();
    assert!(!unreviewed.status.success());
    assert!(
        String::from_utf8_lossy(&unreviewed.stderr).contains("requires a valid reviewed Draft")
    );
    assert!(!root.join(output_relative).exists());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn signal_domain_catalog_is_versioned_machine_readable_and_deterministic() {
    let json_output = Command::new(env!("CARGO_BIN_EXE_agentic-vnext-rust"))
        .args(["catalog", "signal-domains", "--format", "json"])
        .output()
        .unwrap();
    assert_success(&json_output);
    assert!(json_output.stderr.is_empty());
    let catalog: Value = serde_json::from_slice(&json_output.stdout).unwrap();
    validate_catalog_schema(&catalog, "signal-domain-catalog.schema.json");
    assert_eq!(catalog["catalog_version"], "3");
    assert_eq!(catalog["domains"][0]["id"], "data-persistence");
    assert_eq!(
        catalog["domains"][0]["signals"],
        json!(["object-storage-write", "persistent-data-write"])
    );
    assert_eq!(
        catalog["domains"][1]["signals"],
        json!([
            "distributed-effect",
            "external-system-call",
            "message-or-event-publish"
        ])
    );
    assert_eq!(catalog["domains"][2]["id"], "security-boundary");
    assert_eq!(
        catalog["domains"][2]["signals"],
        json!(["authorization-control-change", "sensitive-data-access"])
    );
    assert_eq!(catalog["fact_kinds"][0]["id"], "authorization_change");
    assert_eq!(
        catalog["fact_kinds"][0]["emits"],
        json!(["authorization-control-change"])
    );
    assert_eq!(catalog["fact_kinds"][1]["id"], "db_write");
    assert_eq!(
        catalog["fact_kinds"][2]["emits"],
        json!(["distributed-effect", "external-system-call"])
    );
    assert_eq!(
        catalog["fact_kinds"][3]["emits"],
        json!(["distributed-effect", "message-or-event-publish"])
    );
    assert_eq!(
        catalog["fact_kinds"][4]["emits"],
        json!(["object-storage-write", "persistent-data-write"])
    );
    assert_eq!(
        catalog["fact_kinds"][5]["emits"],
        json!(["sensitive-data-access"])
    );
    let mut body = catalog.as_object().unwrap().clone();
    let digest = body.remove("digest").unwrap();
    assert_eq!(digest, canonical_digest(&Value::Object(body)).unwrap());
    let golden_path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../golden/v1/signal-domain-catalog.json");
    let golden: Value = serde_json::from_slice(&fs::read(golden_path).unwrap()).unwrap();
    assert_eq!(catalog, golden);

    let text_output = Command::new(env!("CARGO_BIN_EXE_agentic-vnext-rust"))
        .args(["catalog", "signal-domains"])
        .output()
        .unwrap();
    assert_success(&text_output);
    let text = String::from_utf8_lossy(&text_output.stdout);
    assert!(text.contains("Signal Domain Catalog 3"));
    assert!(text.contains("data-persistence: Data persistence"));
    assert!(text.contains("external_call -> distributed-effect, external-system-call"));
    assert!(text.contains("object_write -> object-storage-write, persistent-data-write"));
    assert!(text.contains("message_publish -> distributed-effect, message-or-event-publish"));
    assert!(text.contains("authorization_change -> authorization-control-change"));
    assert!(text.contains("sensitive_data_access -> sensitive-data-access"));

    let invalid = Command::new(env!("CARGO_BIN_EXE_agentic-vnext-rust"))
        .args(["catalog", "unknown"])
        .output()
        .unwrap();
    assert!(!invalid.status.success());
    assert!(
        String::from_utf8_lossy(&invalid.stderr)
            .contains("catalog requires the signal-domains operation")
    );
}

#[test]
fn project_and_change_init_connect_an_empty_repository_to_next() {
    let release = TestProject::new();
    let candidate = release.root.join("bootstrap-candidate");
    fs::create_dir(&candidate).unwrap();
    fs::copy(
        release.root.join(".agentic/framework.lock"),
        candidate.join("candidate-framework.lock"),
    )
    .unwrap();
    fs::write(
        candidate.join("framework-release.tar"),
        tar_release(&release.release_root),
    )
    .unwrap();
    let signing_key = SigningKey::from_bytes(&[7_u8; 32]);
    let trust = json!({
        "schema_version": "1",
        "release_id": "prototype-vnext-dev",
        "keys": [{
            "id": "test.framework.release",
            "algorithm": "ed25519",
            "public_key": encode_hex(&signing_key.verifying_key().to_bytes()),
            "allowed_sources": ["offline:test-fixture"],
            "status": "active",
        }],
    });
    fs::write(
        candidate.join("distribution-trust.json"),
        serde_json::to_vec_pretty(&trust).unwrap(),
    )
    .unwrap();

    let root = temporary_test_root("project-init");
    fs::create_dir_all(&root).unwrap();
    run_git(&root, &["init", "--quiet"]);
    run_git(&root, &["config", "user.email", "cli@example.test"]);
    run_git(&root, &["config", "user.name", "CLI Fixture"]);
    run_git(
        &root,
        &["commit", "--quiet", "--allow-empty", "-m", "initial"],
    );
    let initialized = Command::new(env!("CARGO_BIN_EXE_agentic-vnext-rust"))
        .args(["project", "init", "--project"])
        .arg(&root)
        .arg("--candidate-dir")
        .arg(&candidate)
        .output()
        .unwrap();
    assert_success(&initialized);
    let initialized_stdout = String::from_utf8_lossy(&initialized.stdout);
    assert!(initialized_stdout.contains("Next: if source code already exists"));
    assert!(initialized_stdout.contains("agentic project observe"));
    assert!(initialized_stdout.contains("agentic project validate-bindings"));
    assert!(root.join(".agentic/config.yaml").is_file());
    assert!(root.join(".agentic/framework.lock").is_file());
    assert!(root.join(".agentic/repository-observation.yaml").is_file());
    assert_eq!(
        read_yaml(&root.join(".agentic/repository-observation.yaml"))["schema_version"],
        "5"
    );
    assert!(root.join(".agentic/trusted-release-keys.yaml").is_file());
    assert!(
        root.join(".agentic/cache/releases/prototype-vnext-dev/release.yaml")
            .is_file()
    );

    let repeated = Command::new(env!("CARGO_BIN_EXE_agentic-vnext-rust"))
        .args(["project", "init", "--project"])
        .arg(&root)
        .arg("--candidate-dir")
        .arg(&candidate)
        .output()
        .unwrap();
    assert!(!repeated.status.success());
    assert!(String::from_utf8_lossy(&repeated.stderr).contains("would overwrite"));

    let change = Command::new(env!("CARGO_BIN_EXE_agentic-vnext-rust"))
        .args([
            "change",
            "init",
            "change.example",
            "--title",
            "Example",
            "--intent",
            "Exercise the initialized project",
            "--project",
        ])
        .arg(&root)
        .output()
        .unwrap();
    assert_success(&change);
    run_git(
        &root,
        &[
            "add",
            ".agentic/config.yaml",
            ".agentic/framework.lock",
            ".agentic/repository-observation.yaml",
            ".agentic/trusted-release-keys.yaml",
            ".agentic/cache/.gitignore",
            ".agentic/changes/change.example/change.yaml",
        ],
    );
    run_git(&root, &["commit", "--quiet", "-m", "initialize agentic"]);

    let next = Command::new(env!("CARGO_BIN_EXE_agentic-vnext-rust"))
        .args(["next", "change.example", "--require-clean", "--project"])
        .arg(&root)
        .output()
        .unwrap();
    assert_success(&next);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn project_observe_reports_physical_identities_without_inventing_bindings() {
    let root = temporary_test_root("project-observe");
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(
        root.join("src/orders.py"),
        "def save(order):\n    orders.insert(order)\n",
    )
    .unwrap();
    fs::write(
        root.join("src/publish.ts"),
        "export function publishOrder(order: Order) {\n  events.publish(order);\n}\n",
    )
    .unwrap();
    fs::write(
        root.join("src/Service.JAVA"),
        "class Service { void save() { orders.insert(null); } }\n",
    )
    .unwrap();
    fs::write(
        root.join("src/publish.go"),
        "package service\nfunc PublishOrder(order Order) { events.Publish(order) }\n",
    )
    .unwrap();
    fs::write(
        root.join("src/publish.rs"),
        "fn publish_order(order: Order) { events.publish(order); }\n",
    )
    .unwrap();
    fs::write(
        root.join("src/PublishOrder.kt"),
        "fun publishOrder(order: Order) { events.publish(order) }\n",
    )
    .unwrap();
    fs::write(
        root.join("src/publish_order.rb"),
        "def publish_order(order)\n  events.publish(order)\nend\n",
    )
    .unwrap();
    fs::write(
        root.join("src/publish_order.php"),
        "<?php\nfunction publish_order($order) { events::publish($order); }\n",
    )
    .unwrap();
    fs::write(
        root.join("src/PublishOrder.cs"),
        "sealed class CsOrderService { void PublishOrder(Order order) { events.Publish(order); } }\n",
    )
    .unwrap();
    fs::write(
        root.join("src/PublishOrder.swift"),
        "final class SwiftOrderService { func publishOrder(_ order: Order) { events.publish(order) } }\n",
    )
    .unwrap();
    fs::write(
        root.join("src/PublishOrder.scala"),
        "final class ScalaOrderService { def publishOrder(order: Order): Unit = { events.publish(order) } }\n",
    )
    .unwrap();
    fs::write(
        root.join("src/publish_order.c"),
        "void publish_order(Order *order) { publish(events, order); }\n",
    )
    .unwrap();
    fs::write(
        root.join("src/PublishOrder.gd"),
        "class_name GdOrderService\nextends Node\nfunc publish_order(order):\n    events.emit(order)\n",
    )
    .unwrap();
    fs::write(root.join("src/native.cpp"), "void save() {}\n").unwrap();
    run_git(&root, &["init", "--quiet"]);
    let output = Command::new(env!("CARGO_BIN_EXE_agentic-vnext-rust"))
        .args([
            "project",
            "observe",
            "--analysis-root",
            "src",
            "--format",
            "json",
            "--project",
        ])
        .arg(&root)
        .output()
        .unwrap();
    assert_success(&output);
    let draft: Value = serde_json::from_slice(&output.stdout).unwrap();
    validate_output_schema(&draft, "repository-observation-draft.schema.json");
    assert_eq!(draft["schema_version"], "6");
    assert_eq!(draft["kind"], "repository-observation-draft");
    let artifacts = draft["artifacts"].as_array().unwrap();
    let python = artifacts
        .iter()
        .find(|artifact| artifact["path"] == "src/orders.py")
        .unwrap();
    assert_eq!(python["detector_status"], "supported");
    assert_eq!(python["symbols"][0], "save");
    assert_eq!(python["resources"][0], "orders");
    assert!(python.get("logical_ref").is_none());
    let typescript = artifacts
        .iter()
        .find(|artifact| artifact["path"] == "src/publish.ts")
        .unwrap();
    assert_eq!(typescript["language"], "typescript");
    assert_eq!(typescript["detector_status"], "supported");
    assert_eq!(typescript["symbols"][0], "publishOrder");
    assert_eq!(typescript["resources"][0], "events");
    let java = artifacts
        .iter()
        .find(|artifact| artifact["path"] == "src/Service.JAVA")
        .unwrap();
    assert_eq!(java["language"], "java");
    assert_eq!(java["detector_status"], "supported");
    assert_eq!(java["symbols"][0], "Service.save");
    assert_eq!(java["resources"][0], "orders");
    let go = artifacts
        .iter()
        .find(|artifact| artifact["path"] == "src/publish.go")
        .unwrap();
    assert_eq!(go["language"], "go");
    assert_eq!(go["detector_status"], "supported");
    assert_eq!(go["symbols"][0], "PublishOrder");
    assert_eq!(go["resources"][0], "events");
    let rust = artifacts
        .iter()
        .find(|artifact| artifact["path"] == "src/publish.rs")
        .unwrap();
    assert_eq!(rust["language"], "rust");
    assert_eq!(rust["detector_status"], "supported");
    assert_eq!(rust["symbols"][0], "publish_order");
    assert_eq!(rust["resources"][0], "events");
    let kotlin = artifacts
        .iter()
        .find(|artifact| artifact["path"] == "src/PublishOrder.kt")
        .unwrap();
    assert_eq!(kotlin["language"], "kotlin");
    assert_eq!(kotlin["detector_status"], "supported");
    assert_eq!(kotlin["symbols"][0], "publishOrder");
    assert_eq!(kotlin["resources"][0], "events");
    let ruby = artifacts
        .iter()
        .find(|artifact| artifact["path"] == "src/publish_order.rb")
        .unwrap();
    assert_eq!(ruby["language"], "ruby");
    assert_eq!(ruby["detector_status"], "supported");
    assert_eq!(ruby["symbols"][0], "publish_order");
    assert_eq!(ruby["resources"][0], "events");
    let php = artifacts
        .iter()
        .find(|artifact| artifact["path"] == "src/publish_order.php")
        .unwrap();
    assert_eq!(php["language"], "php");
    assert_eq!(php["detector_status"], "supported");
    assert_eq!(php["symbols"][0], "publish_order");
    assert_eq!(php["resources"][0], "events");
    let csharp = artifacts
        .iter()
        .find(|artifact| artifact["path"] == "src/PublishOrder.cs")
        .unwrap();
    assert_eq!(csharp["language"], "csharp");
    assert_eq!(csharp["detector_status"], "supported");
    assert_eq!(csharp["symbols"][0], "CsOrderService.PublishOrder");
    assert_eq!(csharp["resources"][0], "events");
    let swift = artifacts
        .iter()
        .find(|artifact| artifact["path"] == "src/PublishOrder.swift")
        .unwrap();
    assert_eq!(swift["language"], "swift");
    assert_eq!(swift["detector_status"], "supported");
    assert_eq!(swift["symbols"][0], "SwiftOrderService.publishOrder");
    assert_eq!(swift["resources"][0], "events");
    let scala = artifacts
        .iter()
        .find(|artifact| artifact["path"] == "src/PublishOrder.scala")
        .unwrap();
    assert_eq!(scala["language"], "scala");
    assert_eq!(scala["detector_status"], "supported");
    assert_eq!(scala["symbols"][0], "ScalaOrderService.publishOrder");
    assert_eq!(scala["resources"][0], "events");
    let c = artifacts
        .iter()
        .find(|artifact| artifact["path"] == "src/publish_order.c")
        .unwrap();
    assert_eq!(c["language"], "c");
    assert_eq!(c["detector_status"], "supported");
    assert_eq!(c["symbols"][0], "publish_order");
    assert_eq!(c["resources"][0], "events");
    let gdscript = artifacts
        .iter()
        .find(|artifact| artifact["path"] == "src/PublishOrder.gd")
        .unwrap();
    assert_eq!(gdscript["language"], "gdscript");
    assert_eq!(gdscript["detector_status"], "supported");
    assert_eq!(gdscript["symbols"][0], "GdOrderService.publish_order");
    assert_eq!(gdscript["resources"][0], "events");
    let cpp = artifacts
        .iter()
        .find(|artifact| artifact["path"] == "src/native.cpp")
        .unwrap();
    assert_eq!(cpp["language"], "cpp");
    assert_eq!(cpp["detector_status"], "unsupported");

    let binding_artifacts = draft["binding_artifacts"].as_array().unwrap();
    assert_eq!(binding_artifacts.len(), artifacts.len());
    let python_binding = binding_artifacts
        .iter()
        .find(|artifact| artifact["path"] == "src/orders.py")
        .unwrap();
    assert_eq!(python_binding["ref"], "code.src.orders");
    assert!(
        python_binding["bindings"]["symbols"]["save"]
            .as_object()
            .unwrap()
            .contains_key("logical_ref")
    );
    assert!(python_binding["bindings"]["symbols"]["save"]["logical_ref"].is_null());
    assert!(python_binding["bindings"]["symbols"]["save"]["owner"].is_null());
    assert!(python_binding["bindings"]["symbols"]["save"]["authority_ref"].is_null());
    assert!(python_binding["bindings"]["resources"]["orders"]["logical_refs"].is_null());
    assert!(
        python_binding["bindings"]["resources"]["orders"]
            .as_object()
            .unwrap()
            .contains_key("logical_refs")
    );
    assert_eq!(
        python_binding["bindings"]["methods"]
            .as_object()
            .unwrap()
            .len(),
        0
    );
    let cpp_binding = binding_artifacts
        .iter()
        .find(|artifact| artifact["path"] == "src/native.cpp")
        .unwrap();
    assert!(
        cpp_binding["bindings"]["symbols"]
            .as_object()
            .unwrap()
            .is_empty()
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn project_observe_writes_a_new_draft_without_applying_or_overwriting_it() {
    let root = temporary_test_root("project-observe-output");
    fs::create_dir_all(root.join("src")).unwrap();
    fs::create_dir_all(root.join(".agentic/drafts")).unwrap();
    fs::write(
        root.join("src/orders.py"),
        "def save(order):\n    orders.insert(order)\n",
    )
    .unwrap();
    run_git(&root, &["init", "--quiet"]);
    let relative = ".agentic/drafts/repository-observation.yaml";
    let output = Command::new(env!("CARGO_BIN_EXE_agentic-vnext-rust"))
        .args([
            "project",
            "observe",
            "--analysis-root",
            "src",
            "--format",
            "yaml",
            "--output",
            relative,
            "--project",
        ])
        .arg(&root)
        .output()
        .unwrap();
    assert_success(&output);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Observation draft written to:"));
    assert!(stdout.contains("Next: review binding_artifacts"));
    assert!(stdout.contains("Binding candidates were not applied automatically"));
    let draft_path = root.join(relative);
    let original = fs::read(&draft_path).unwrap();
    let draft = read_yaml(&draft_path);
    assert_eq!(draft["kind"], "repository-observation-draft");
    assert!(draft["binding_artifacts"][0]["bindings"]["symbols"]["save"]["logical_ref"].is_null());

    let repeated = Command::new(env!("CARGO_BIN_EXE_agentic-vnext-rust"))
        .args([
            "project",
            "observe",
            "--analysis-root",
            "src",
            "--output",
            relative,
            "--project",
        ])
        .arg(&root)
        .output()
        .unwrap();
    assert!(!repeated.status.success());
    assert!(String::from_utf8_lossy(&repeated.stderr).contains("refusing to overwrite"));
    assert_eq!(fs::read(&draft_path).unwrap(), original);

    let escaped = Command::new(env!("CARGO_BIN_EXE_agentic-vnext-rust"))
        .args([
            "project",
            "observe",
            "--analysis-root",
            "src",
            "--output",
            "../outside.yaml",
            "--project",
        ])
        .arg(&root)
        .output()
        .unwrap();
    assert!(!escaped.status.success());
    assert!(String::from_utf8_lossy(&escaped.stderr).contains("must stay in the repository"));

    let git_internal = Command::new(env!("CARGO_BIN_EXE_agentic-vnext-rust"))
        .args([
            "project",
            "observe",
            "--analysis-root",
            "src",
            "--output",
            ".git/observation-draft.yaml",
            "--project",
        ])
        .arg(&root)
        .output()
        .unwrap();
    assert!(!git_internal.status.success());
    assert!(String::from_utf8_lossy(&git_internal.stderr).contains("must not be inside .git"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn project_observe_uses_only_the_signed_release_framework_catalog() {
    let project = TestProject::new();
    fs::create_dir_all(project.root.join("catalog-src")).unwrap();
    fs::write(
        project.root.join("catalog-src/package.json"),
        r#"{"dependencies":{"typeorm":"latest"}}"#,
    )
    .unwrap();
    fs::write(
        project.root.join("catalog-src/orders.ts"),
        "import { Repository } from \"typeorm\";\n\
         export async function placeOrder(repository: Repository<Order>, order: Order) {\n\
         \x20 await repository.save(order);\n\
         }\n",
    )
    .unwrap();
    run_git(&project.root, &["add", "catalog-src"]);
    run_git(
        &project.root,
        &["commit", "--quiet", "-m", "add typeorm source"],
    );

    let observed = project.run(&[
        "project",
        "observe",
        "--analysis-root",
        "catalog-src",
        "--format",
        "json",
    ]);
    assert_success(&observed);
    let draft: Value = serde_json::from_slice(&observed.stdout).unwrap();
    let candidate = &draft["artifacts"][0]["framework_candidates"][0];
    assert_eq!(candidate["framework"], "dev.agentic-kit/typeorm");
    assert_eq!(candidate["method"], "save");
    assert_eq!(candidate["suggested_fact_kinds"], json!(["db_write"]));
    assert_eq!(candidate["review_status"], "required");

    fs::write(
        project.release_root.join("framework-catalog.yaml"),
        "schema_version: '1'\nnamespace: tampered\nrules: []\n",
    )
    .unwrap();
    let rejected = project.run(&[
        "project",
        "observe",
        "--analysis-root",
        "catalog-src",
        "--format",
        "json",
    ]);
    assert!(!rejected.status.success());
    assert!(
        String::from_utf8_lossy(&rejected.stderr)
            .contains("Framework Release file digest mismatch")
    );
}

#[cfg(unix)]
#[test]
fn project_observe_refuses_a_symlinked_draft_output() {
    use std::os::unix::fs::symlink;

    let root = temporary_test_root("project-observe-output-symlink");
    let outside = temporary_test_root("project-observe-output-symlink-target");
    fs::create_dir_all(root.join("src")).unwrap();
    fs::create_dir_all(root.join(".agentic")).unwrap();
    fs::write(
        root.join("src/orders.py"),
        "def save(order):\n    orders.insert(order)\n",
    )
    .unwrap();
    fs::write(&outside, "keep\n").unwrap();
    symlink(&outside, root.join(".agentic/draft.yaml")).unwrap();
    run_git(&root, &["init", "--quiet"]);

    let output = Command::new(env!("CARGO_BIN_EXE_agentic-vnext-rust"))
        .args([
            "project",
            "observe",
            "--analysis-root",
            "src",
            "--output",
            ".agentic/draft.yaml",
            "--project",
        ])
        .arg(&root)
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("refusing symlinked project path"));
    assert_eq!(fs::read_to_string(&outside).unwrap(), "keep\n");
    let _ = fs::remove_dir_all(root);
    let _ = fs::remove_file(outside);
}

#[test]
fn project_validates_and_promotes_a_reviewed_draft_safely() {
    let project = TestProject::new();
    fs::write(
        project.root.join("src/place_order.py"),
        "from django.db import models\n\
         def place_order(order):\n\
         \x20   orders.save(order)\n",
    )
    .unwrap();
    let active_path = project.root.join(".agentic/repository-observation.yaml");
    let mut base_observation = read_yaml(&active_path);
    base_observation["phase"] = json!("post-build");
    write_yaml(&active_path, &base_observation);
    let draft_relative = ".agentic/repository-observation.draft.yaml";
    let observed = project.run(&[
        "project",
        "observe",
        "--analysis-root",
        "src",
        "--output",
        draft_relative,
    ]);
    assert_success(&observed);
    let active_before = fs::read(&active_path).unwrap();

    let incomplete = project.run(&["project", "validate-bindings", "--draft", draft_relative]);
    assert!(!incomplete.status.success());
    assert!(String::from_utf8_lossy(&incomplete.stdout).contains("incomplete-binding"));
    let rejected_promotion =
        project.run(&["project", "promote-bindings", "--draft", draft_relative]);
    assert!(!rejected_promotion.status.success());
    assert!(
        String::from_utf8_lossy(&rejected_promotion.stdout).contains("Promotion was not performed")
    );
    assert_eq!(fs::read(&active_path).unwrap(), active_before);

    let draft_path = project.root.join(draft_relative);
    let mut draft = read_yaml(&draft_path);
    for artifact in draft["binding_artifacts"].as_array_mut().unwrap() {
        artifact["bindings"] = match artifact["path"].as_str().unwrap() {
            "src/place_order.py" => json!({
                "symbols": {
                    "place_order": {
                        "logical_ref": "operation.place-order",
                        "owner": "team.ordering",
                        "authority_ref": "decision.repository-bindings"
                    }
                },
                "resources": {
                    "orders": {
                        "logical_refs": {"data": "data.orders"},
                        "owner": "team.ordering",
                        "authority_ref": "decision.repository-bindings"
                    }
                },
                "methods": {
                    "orders.save": {
                        "fact_kinds": ["unknown_write"],
                        "owner": "team.ordering",
                        "authority_ref": "decision.repository-bindings"
                    }
                }
            }),
            "src/publish_order.py" => json!({
                "symbols": {
                    "publish_order": {
                        "logical_ref": "operation.place-order",
                        "owner": "team.ordering",
                        "authority_ref": "decision.repository-bindings"
                    }
                },
                "resources": {
                    "order_events": {
                        "logical_refs": {"integration": "integration.order-events"},
                        "owner": "team.ordering",
                        "authority_ref": "decision.repository-bindings"
                    }
                },
                "methods": {}
            }),
            path => panic!("unexpected Draft artifact: {path}"),
        };
    }
    write_yaml(&draft_path, &draft);

    let retained_method = draft["binding_artifacts"][0]["bindings"]["methods"]
        .as_object_mut()
        .unwrap()
        .remove("orders.save")
        .unwrap();
    write_yaml(&draft_path, &draft);
    let unclassified = project.run(&["project", "validate-bindings", "--draft", draft_relative]);
    assert!(!unclassified.status.success());
    assert!(String::from_utf8_lossy(&unclassified.stdout).contains("unsupported-observation"));
    draft["binding_artifacts"][0]["bindings"]["methods"]
        .as_object_mut()
        .unwrap()
        .insert("orders.save".to_owned(), retained_method);
    write_yaml(&draft_path, &draft);

    let unknown_kind = project.run(&["project", "validate-bindings", "--draft", draft_relative]);
    assert!(!unknown_kind.status.success());
    assert!(String::from_utf8_lossy(&unknown_kind.stdout).contains("unknown-fact-kind"));

    draft["binding_artifacts"][0]["bindings"]["methods"]["orders.save"]["fact_kinds"] =
        json!(["db_write"]);
    draft["binding_artifacts"][0]["bindings"]["methods"]["orders.save"]["authority_ref"] =
        json!("decision.not-accepted");
    write_yaml(&draft_path, &draft);
    let unaccepted = project.run(&["project", "validate-bindings", "--draft", draft_relative]);
    assert!(!unaccepted.status.success());
    assert!(String::from_utf8_lossy(&unaccepted.stdout).contains("unaccepted-binding-authority"));

    draft["binding_artifacts"][0]["bindings"]["methods"]["orders.save"]["authority_ref"] =
        json!("decision.repository-bindings");
    write_yaml(&draft_path, &draft);
    let valid = project.run(&["project", "validate-bindings", "--draft", draft_relative]);
    assert_success(&valid);
    let valid_stdout = String::from_utf8_lossy(&valid.stdout);
    assert!(valid_stdout.contains("Binding Draft validation: valid"));
    assert!(valid_stdout.contains("project promote-bindings --draft <path>"));
    assert!(valid_stdout.contains("No authoritative project file was changed"));
    assert_eq!(fs::read(&active_path).unwrap(), active_before);

    let mut changed_observation = read_yaml(&active_path);
    changed_observation["phase"] = json!("pre-build");
    write_yaml(&active_path, &changed_observation);
    let changed_bytes = fs::read(&active_path).unwrap();
    let stale_observation =
        project.run(&["project", "promote-bindings", "--draft", draft_relative]);
    assert!(!stale_observation.status.success());
    assert!(String::from_utf8_lossy(&stale_observation.stdout).contains("stale-observation"));
    assert_eq!(fs::read(&active_path).unwrap(), changed_bytes);
    fs::write(&active_path, &active_before).unwrap();

    let promoted = project.run(&["project", "promote-bindings", "--draft", draft_relative]);
    assert_success(&promoted);
    let promoted_stdout = String::from_utf8_lossy(&promoted.stdout);
    assert!(promoted_stdout.contains("Binding Draft promoted successfully"));
    assert!(promoted_stdout.contains("Draft retained"));
    assert!(draft_path.is_file());
    let active = read_yaml(&active_path);
    assert_eq!(active["schema_version"], "5");
    assert_eq!(active["phase"], "post-build");
    assert_eq!(active["analysis"], json!({"roots": ["src"]}));
    assert_eq!(active["artifacts"], draft["binding_artifacts"]);
    let active_validation = project.run(&["project", "validate-bindings"]);
    assert_success(&active_validation);

    let promoted_bytes = fs::read(&active_path).unwrap();
    let repeated = project.run(&["project", "promote-bindings", "--draft", draft_relative]);
    assert!(!repeated.status.success());
    assert!(String::from_utf8_lossy(&repeated.stdout).contains("stale-observation"));
    assert_eq!(fs::read(&active_path).unwrap(), promoted_bytes);

    fs::write(
        project.root.join("src/place_order.py"),
        "from django.db import models\n\
         def place_order(order):\n\
         \x20   orders.save(order)\n\
         # changed after review\n",
    )
    .unwrap();
    let stale = project.run(&["project", "validate-bindings", "--draft", draft_relative]);
    assert!(!stale.status.success());
    assert!(String::from_utf8_lossy(&stale.stdout).contains("stale-source"));
    assert_eq!(fs::read(&active_path).unwrap(), promoted_bytes);

    let json = project.run(&[
        "project",
        "validate-bindings",
        "--draft",
        draft_relative,
        "--format",
        "json",
    ]);
    assert!(!json.status.success());
    assert!(String::from_utf8_lossy(&json.stderr).contains("supports only text output"));

    let missing_draft = project.run(&["project", "promote-bindings"]);
    assert!(!missing_draft.status.success());
    assert!(String::from_utf8_lossy(&missing_draft.stderr).contains("requires --draft"));
    assert!(
        !fs::read_dir(project.root.join(".agentic"))
            .unwrap()
            .any(|entry| entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".repository-observation.tmp-"))
    );
}

#[test]
fn major_frameworks_run_from_observation_draft_through_signal_generation() {
    let cases: [(&str, &[&str]); 8] = [
        ("python-django-sqs", &["amazon-sqs", "django-orm"]),
        ("python-sqlalchemy-celery", &["celery", "sqlalchemy"]),
        ("typescript-prisma-kafka", &["apache-kafka", "prisma"]),
        ("java-spring-rabbit", &["rabbitmq", "spring-data-jpa"]),
        (
            "csharp-ef-servicebus",
            &["azure-service-bus", "entity-framework-core"],
        ),
        (
            "ruby-rails-redis",
            &["rails-active-record", "redis-streams"],
        ),
        (
            "php-laravel-pubsub",
            &["google-cloud-pubsub", "laravel-eloquent"],
        ),
        ("go-gorm-nats", &["gorm", "nats"]),
    ];

    for (case_id, expected_frameworks) in cases {
        let next = run_framework_e2e(case_id, expected_frameworks);
        assert_eq!(
            signals_for_binding(&next, "data", &format!("data.persistence-e2e-{case_id}")),
            ["persistent-data-write"],
            "missing persistence signal for {case_id}"
        );
        assert_eq!(
            signals_for_binding(
                &next,
                "integration",
                &format!("integration.messaging-e2e-{case_id}")
            ),
            ["distributed-effect", "message-or-event-publish"],
            "missing messaging signals for {case_id}"
        );
    }
}

#[test]
fn major_http_clients_run_from_observation_draft_through_signal_generation() {
    let case_id = "external-api-clients";
    let next = run_framework_e2e(
        case_id,
        &[
            "axios",
            "dotnet-http-client",
            "go-net-http",
            "java-http-client",
            "python-httpx",
            "python-requests",
            "spring-webclient",
            "web-fetch",
        ],
    );
    assert_eq!(
        signals_for_binding(
            &next,
            "integration",
            &format!("integration.external-e2e-{case_id}")
        ),
        ["distributed-effect", "external-system-call"]
    );
}

#[test]
fn object_storage_sdks_run_from_observation_draft_through_signal_generation() {
    let case_id = "object-storage-sdks";
    let next = run_framework_e2e(
        case_id,
        &["amazon-s3", "azure-blob-storage", "google-cloud-storage"],
    );
    assert_eq!(
        signals_for_binding(
            &next,
            "integration",
            &format!("integration.external-e2e-{case_id}")
        ),
        ["distributed-effect", "external-system-call"]
    );
    assert_eq!(
        signals_for_binding(&next, "data", &format!("data.object-e2e-{case_id}")),
        ["object-storage-write", "persistent-data-write"]
    );
}

#[test]
fn project_observe_suggests_eight_major_orms_without_approving_them() {
    let root = temporary_test_root("project-observe-frameworks");
    fs::create_dir_all(root.join("src")).unwrap();
    let files = [
        (
            "src/django_service.py",
            "from django.db import models\n\
             def place_order(order):\n\
             \x20   order.save()\n",
        ),
        (
            "src/sqlalchemy_service.py",
            "from sqlalchemy.orm import Session\n\
             def place_order(session: Session, order, statement):\n\
             \x20   session.add(order)\n\
             \x20   session.execute(statement)\n",
        ),
        (
            "src/prisma_service.ts",
            "import { PrismaClient } from \"@prisma/client\";\n\
             const prisma = new PrismaClient();\n\
             export function placeOrder(order: Order) {\n\
             \x20 return prisma.order.create({ data: order });\n\
             }\n",
        ),
        (
            "src/SpringOrderService.java",
            "import org.springframework.data.repository.CrudRepository;\n\
             class SpringOrderService {\n\
             \x20 void placeOrder(OrderRepository repository, Order order) {\n\
             \x20   repository.save(order);\n\
             \x20 }\n\
             }\n",
        ),
        (
            "src/EfOrderService.cs",
            "using Microsoft.EntityFrameworkCore;\n\
             class EfOrderService {\n\
             \x20 async Task PlaceOrder(DbContext context) {\n\
             \x20   await context.SaveChangesAsync();\n\
             \x20 }\n\
             }\n",
        ),
        (
            "src/rails_order_service.rb",
            "class RailsOrderService\n\
             \x20 def place_order(order)\n\
             \x20   order.save!\n\
             \x20 end\n\
             end\n",
        ),
        (
            "src/laravel_order_service.php",
            "<?php\n\
             use Illuminate\\Database\\Eloquent\\Model;\n\
             function placeOrder($order) {\n\
             \x20   $order->save();\n\
             }\n",
        ),
        (
            "src/gorm_order_service.go",
            "package service\n\
             import \"gorm.io/gorm\"\n\
             func PlaceOrder(db *gorm.DB, order *Order) {\n\
             \x20   db.Create(order)\n\
             }\n",
        ),
    ];
    for (path, source) in files {
        fs::write(root.join(path), source).unwrap();
    }
    fs::write(
        root.join("Gemfile"),
        "source \"https://rubygems.org\"\ngem \"rails\"\n",
    )
    .unwrap();
    run_git(&root, &["init", "--quiet"]);

    let output = Command::new(env!("CARGO_BIN_EXE_agentic-vnext-rust"))
        .args([
            "project",
            "observe",
            "--analysis-root",
            "src",
            "--format",
            "json",
            "--project",
        ])
        .arg(&root)
        .output()
        .unwrap();
    assert_success(&output);
    let draft: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(draft["schema_version"], "6");
    let artifacts = draft["artifacts"].as_array().unwrap();
    let expected = [
        ("src/django_service.py", "django-orm", "save"),
        ("src/sqlalchemy_service.py", "sqlalchemy", "add"),
        ("src/prisma_service.ts", "prisma", "create"),
        ("src/SpringOrderService.java", "spring-data-jpa", "save"),
        (
            "src/EfOrderService.cs",
            "entity-framework-core",
            "SaveChangesAsync",
        ),
        ("src/rails_order_service.rb", "rails-active-record", "save!"),
        ("src/laravel_order_service.php", "laravel-eloquent", "save"),
        ("src/gorm_order_service.go", "gorm", "Create"),
    ];
    for (path, framework, method) in expected {
        let artifact = artifacts
            .iter()
            .find(|artifact| artifact["path"] == path)
            .unwrap_or_else(|| panic!("missing {path}"));
        let candidate = artifact["framework_candidates"]
            .as_array()
            .unwrap()
            .iter()
            .find(|candidate| candidate["framework"] == framework && candidate["method"] == method)
            .unwrap_or_else(|| panic!("missing {framework}.{method} for {path}"));
        assert_eq!(candidate["suggested_fact_kinds"], json!(["db_write"]));
        assert_eq!(candidate["review_status"], "required");
        assert_eq!(candidate["method_binding_required"], true);
        assert!(
            candidate["binding_key"]
                .as_str()
                .is_some_and(|key| key.ends_with(&format!(".{method}")))
        );
        let binding_artifact = draft["binding_artifacts"]
            .as_array()
            .unwrap()
            .iter()
            .find(|artifact| artifact["path"] == path)
            .unwrap();
        assert!(
            binding_artifact["bindings"]["methods"][candidate["binding_key"].as_str().unwrap()]
                ["fact_kinds"]
                .is_null()
        );
        assert!(
            binding_artifact["bindings"]["methods"]
                [candidate["binding_key"].as_str().unwrap()]["owner"]
                .is_null()
        );
    }

    let sqlalchemy = artifacts
        .iter()
        .find(|artifact| artifact["path"] == "src/sqlalchemy_service.py")
        .unwrap();
    let execute = sqlalchemy["framework_candidates"]
        .as_array()
        .unwrap()
        .iter()
        .find(|candidate| candidate["method"] == "execute")
        .unwrap();
    assert_eq!(execute["suggested_fact_kinds"], json!([]));
    let sqlalchemy_binding = draft["binding_artifacts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|artifact| artifact["path"] == "src/sqlalchemy_service.py")
        .unwrap();
    assert!(sqlalchemy_binding["bindings"]["methods"]["session.execute"]["fact_kinds"].is_null());
    assert!(
        draft["next"]
            .as_str()
            .unwrap()
            .contains("non-authoritative")
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn project_observe_suggests_eight_major_messaging_apis_without_approving_them() {
    let root = temporary_test_root("project-observe-messaging");
    fs::create_dir_all(root.join("src")).unwrap();
    let files = [
        (
            "src/sqs_service.py",
            "import boto3\n\
             sqs = boto3.client('sqs')\n\
             def enqueue(queue_url, body):\n\
             \x20   sqs.send_message(QueueUrl=queue_url, MessageBody=body)\n",
        ),
        (
            "src/KafkaService.java",
            "import org.apache.kafka.clients.producer.KafkaProducer;\n\
             class KafkaService {\n\
             \x20 void publish(KafkaProducer producer, ProducerRecord record) {\n\
             \x20   producer.send(record);\n\
             \x20 }\n\
             }\n",
        ),
        (
            "src/rabbit_service.ts",
            "import amqp from \"amqplib\";\n\
             export function enqueue(channel: amqp.Channel, payload: Buffer) {\n\
             \x20 channel.sendToQueue(\"orders\", payload);\n\
             }\n",
        ),
        (
            "src/celery_service.py",
            "from celery import shared_task\n\
             def enqueue(task):\n\
             \x20   task.delay()\n",
        ),
        (
            "src/pubsub_service.rb",
            "require \"google/cloud/pubsub\"\n\
             def publish_event(publisher, payload)\n\
             \x20 publisher.publish_async(payload)\n\
             end\n",
        ),
        (
            "src/ServiceBusService.cs",
            "using Azure.Messaging.ServiceBus;\n\
             class ServiceBusService {\n\
             \x20 async Task Send(ServiceBusSender sender, IEnumerable<ServiceBusMessage> messages) {\n\
             \x20   await sender.SendMessagesAsync(messages);\n\
             \x20 }\n\
             }\n",
        ),
        (
            "src/nats_service.go",
            "package service\n\
             import \"github.com/nats-io/nats.go\"\n\
             func PublishEvent(connection *nats.Conn, payload []byte) {\n\
             \x20   connection.Publish(\"orders\", payload)\n\
             }\n",
        ),
        (
            "src/redis_stream_service.ts",
            "import { createClient } from \"redis\";\n\
             export function append(redis: ReturnType<typeof createClient>) {\n\
             \x20 redis.xAdd(\"orders\", \"*\", { status: \"created\" });\n\
             }\n",
        ),
    ];
    for (path, source) in files {
        fs::write(root.join(path), source).unwrap();
    }
    run_git(&root, &["init", "--quiet"]);

    let output = Command::new(env!("CARGO_BIN_EXE_agentic-vnext-rust"))
        .args([
            "project",
            "observe",
            "--analysis-root",
            "src",
            "--format",
            "json",
            "--project",
        ])
        .arg(&root)
        .output()
        .unwrap();
    assert_success(&output);
    let draft: Value = serde_json::from_slice(&output.stdout).unwrap();
    let artifacts = draft["artifacts"].as_array().unwrap();
    let expected = [
        ("src/sqs_service.py", "amazon-sqs", "send_message"),
        ("src/KafkaService.java", "apache-kafka", "send"),
        ("src/rabbit_service.ts", "rabbitmq", "sendToQueue"),
        ("src/celery_service.py", "celery", "delay"),
        (
            "src/pubsub_service.rb",
            "google-cloud-pubsub",
            "publish_async",
        ),
        (
            "src/ServiceBusService.cs",
            "azure-service-bus",
            "SendMessagesAsync",
        ),
        ("src/nats_service.go", "nats", "Publish"),
        ("src/redis_stream_service.ts", "redis-streams", "xAdd"),
    ];
    for (path, framework, method) in expected {
        let artifact = artifacts
            .iter()
            .find(|artifact| artifact["path"] == path)
            .unwrap_or_else(|| panic!("missing {path}"));
        let candidate = artifact["framework_candidates"]
            .as_array()
            .unwrap()
            .iter()
            .find(|candidate| candidate["framework"] == framework && candidate["method"] == method)
            .unwrap_or_else(|| panic!("missing {framework}.{method} for {path}"));
        assert_eq!(
            candidate["suggested_fact_kinds"],
            json!(["message_publish"])
        );
        assert_eq!(candidate["review_status"], "required");
        assert!(
            candidate["binding_key"]
                .as_str()
                .is_some_and(|key| key.ends_with(&format!(".{method}")))
        );
    }
    let kafka_binding = draft["binding_artifacts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|artifact| artifact["path"] == "src/KafkaService.java")
        .unwrap();
    assert!(kafka_binding["bindings"]["methods"]["producer.send"]["fact_kinds"].is_null());
    assert!(kafka_binding["bindings"]["methods"]["producer.send"]["authority_ref"].is_null());
    let sqs_binding = draft["binding_artifacts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|artifact| artifact["path"] == "src/sqs_service.py")
        .unwrap();
    assert!(
        sqs_binding["bindings"]["methods"]
            .as_object()
            .unwrap()
            .is_empty(),
        "built-in send_message classification does not need a method binding"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn project_observe_suggests_eight_major_http_clients_without_approving_them() {
    let root = temporary_test_root("project-observe-http-clients");
    fs::create_dir_all(root.join("src")).unwrap();
    let files = [
        (
            "src/requests_service.py",
            "import requests\n\
             def notify(payload):\n\
             \x20   requests.post('https://example.test/orders', json=payload)\n",
        ),
        (
            "src/httpx_service.py",
            "import httpx\n\
             def lookup():\n\
             \x20   httpx.get('https://example.test/orders')\n",
        ),
        (
            "src/fetch_service.ts",
            "export function notify(payload: unknown) {\n\
             \x20 window.fetch('https://example.test/orders', { method: 'POST' });\n\
             }\n",
        ),
        (
            "src/axios_service.ts",
            "import axios from 'axios';\n\
             export function notify(payload: unknown) {\n\
             \x20 axios.post('https://example.test/orders', payload);\n\
             }\n",
        ),
        (
            "src/JavaHttpService.java",
            "import java.net.http.HttpClient;\n\
             class JavaHttpService {\n\
             \x20 void notify(HttpClient httpClient, HttpRequest request, BodyHandler handler) {\n\
             \x20   httpClient.sendAsync(request, handler);\n\
             \x20 }\n\
             }\n",
        ),
        (
            "src/SpringHttpService.kt",
            "import org.springframework.web.reactive.function.client.WebClient\n\
             fun notify(webClient: WebClient) {\n\
             \x20 webClient.post().uri(\"/orders\").retrieve()\n\
             }\n",
        ),
        (
            "src/go_http_service.go",
            "package service\n\
             import \"net/http\"\n\
             func Notify(client *http.Client, request *http.Request) {\n\
             \x20 client.Do(request)\n\
             }\n",
        ),
        (
            "src/DotnetHttpService.cs",
            "using System.Net.Http;\n\
             class DotnetHttpService {\n\
             \x20 async Task Notify(HttpClient httpClient, HttpRequestMessage request) {\n\
             \x20   await httpClient.SendAsync(request);\n\
             \x20 }\n\
             }\n",
        ),
    ];
    for (path, source) in files {
        fs::write(root.join(path), source).unwrap();
    }
    run_git(&root, &["init", "--quiet"]);

    let output = Command::new(env!("CARGO_BIN_EXE_agentic-vnext-rust"))
        .args([
            "project",
            "observe",
            "--analysis-root",
            "src",
            "--format",
            "json",
            "--project",
        ])
        .arg(&root)
        .output()
        .unwrap();
    assert_success(&output);
    let draft: Value = serde_json::from_slice(&output.stdout).unwrap();
    let expected = [
        ("src/requests_service.py", "python-requests", "post"),
        ("src/httpx_service.py", "python-httpx", "get"),
        ("src/fetch_service.ts", "web-fetch", "fetch"),
        ("src/axios_service.ts", "axios", "post"),
        ("src/JavaHttpService.java", "java-http-client", "sendAsync"),
        ("src/SpringHttpService.kt", "spring-webclient", "retrieve"),
        ("src/go_http_service.go", "go-net-http", "Do"),
        (
            "src/DotnetHttpService.cs",
            "dotnet-http-client",
            "SendAsync",
        ),
    ];
    assert_review_candidates(&draft, &expected, &["external_call"]);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn project_observe_suggests_three_object_storage_families_without_approving_them() {
    let root = temporary_test_root("project-observe-object-storage");
    fs::create_dir_all(root.join("src")).unwrap();
    let files = [
        (
            "src/s3_service.py",
            "import boto3\n\
             s3 = boto3.client('s3')\n\
             def archive(payload):\n\
             \x20   s3.put_object(Bucket='orders', Key='latest', Body=payload)\n",
        ),
        (
            "src/gcs_service.ts",
            "import { Storage } from '@google-cloud/storage';\n\
             export function archive(file: File, payload: string) {\n\
             \x20 file.save(payload);\n\
             }\n",
        ),
        (
            "src/AzureBlobService.cs",
            "using Azure.Storage.Blobs;\n\
             class AzureBlobService {\n\
             \x20 async Task Archive(BlobClient blobClient, Stream payload) {\n\
             \x20   await blobClient.UploadAsync(payload);\n\
             \x20 }\n\
             }\n",
        ),
    ];
    for (path, source) in files {
        fs::write(root.join(path), source).unwrap();
    }
    run_git(&root, &["init", "--quiet"]);

    let output = Command::new(env!("CARGO_BIN_EXE_agentic-vnext-rust"))
        .args([
            "project",
            "observe",
            "--analysis-root",
            "src",
            "--format",
            "json",
            "--project",
        ])
        .arg(&root)
        .output()
        .unwrap();
    assert_success(&output);
    let draft: Value = serde_json::from_slice(&output.stdout).unwrap();
    let expected = [
        ("src/s3_service.py", "amazon-s3", "put_object"),
        ("src/gcs_service.ts", "google-cloud-storage", "save"),
        (
            "src/AzureBlobService.cs",
            "azure-blob-storage",
            "UploadAsync",
        ),
    ];
    assert_review_candidates(&draft, &expected, &["external_call", "object_write"]);
    let _ = fs::remove_dir_all(root);
}

fn assert_review_candidates(draft: &Value, expected: &[(&str, &str, &str)], kinds: &[&str]) {
    let artifacts = draft["artifacts"].as_array().unwrap();
    for (path, framework, method) in expected {
        let artifact = artifacts
            .iter()
            .find(|artifact| artifact["path"] == *path)
            .unwrap_or_else(|| panic!("missing {path}"));
        let candidate = artifact["framework_candidates"]
            .as_array()
            .unwrap()
            .iter()
            .find(|candidate| {
                candidate["framework"] == *framework && candidate["method"] == *method
            })
            .unwrap_or_else(|| panic!("missing {framework}.{method} for {path}"));
        assert_eq!(candidate["suggested_fact_kinds"], json!(kinds));
        assert_eq!(candidate["review_status"], "required");
        assert_eq!(candidate["method_binding_required"], true);
        let binding = draft["binding_artifacts"]
            .as_array()
            .unwrap()
            .iter()
            .find(|artifact| artifact["path"] == *path)
            .unwrap();
        assert!(
            binding["bindings"]["methods"][candidate["binding_key"].as_str().unwrap()]["fact_kinds"]
                .is_null()
        );
    }
}

#[test]
fn binding_artifacts_are_not_authoritative_until_placeholders_are_reviewed() {
    let project = TestProject::new();
    let observe = Command::new(env!("CARGO_BIN_EXE_agentic-vnext-rust"))
        .args([
            "project",
            "observe",
            "--analysis-root",
            "src",
            "--format",
            "json",
            "--project",
        ])
        .arg(&project.root)
        .output()
        .unwrap();
    assert_success(&observe);
    let draft: Value = serde_json::from_slice(&observe.stdout).unwrap();
    write_yaml(
        &project.root.join(".agentic/repository-observation.yaml"),
        &json!({
            "schema_version": "5",
            "phase": "pre-build",
            "analysis": {"roots": draft["analysis_roots"].clone()},
            "artifacts": draft["binding_artifacts"].clone(),
        }),
    );

    let output = project.run(&["next", "change.place-order", "--format", "json"]);
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("logical_ref"),
        "unexpected stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn project_validate_bindings_reports_a_valid_reviewed_repository() {
    let project = TestProject::new();

    let output = project.run(&["project", "validate-bindings", "--format", "json"]);
    assert_success(&output);
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    validate_output_schema(&report, "binding-validation-report.schema.json");
    assert_eq!(report["status"], "valid");
    assert_eq!(report["summary"]["binding_issues"], 0);
    assert_eq!(report["summary"]["coverage_issues"], 0);

    let text = project.run(&["project", "validate-bindings"]);
    assert_success(&text);
    assert!(String::from_utf8_lossy(&text.stdout).contains("Binding validation: valid"));

    fs::write(
        project.root.join("src/place_order.py"),
        "def place_order(order):\n    orders.insert(order)\n# dirty\n",
    )
    .unwrap();
    let clean = project.run(&[
        "project",
        "validate-bindings",
        "--format",
        "json",
        "--require-clean",
    ]);
    assert!(!clean.status.success());
    assert!(String::from_utf8_lossy(&clean.stderr).contains("working tree is not clean"));
}

#[test]
fn project_validate_bindings_reports_missing_and_ambiguous_bindings() {
    let project = TestProject::new();
    let observation_path = project.root.join(".agentic/repository-observation.yaml");
    let mut observation = read_yaml(&observation_path);
    observation["artifacts"][0]["bindings"]["resources"] = json!({});
    write_yaml(&observation_path, &observation);

    let missing = project.run(&["project", "validate-bindings", "--format", "json"]);
    assert!(!missing.status.success());
    let report: Value = serde_json::from_slice(&missing.stdout).unwrap();
    validate_output_schema(&report, "binding-validation-report.schema.json");
    assert_eq!(report["status"], "invalid");
    assert!(report["issues"].as_array().unwrap().iter().any(|issue| {
        issue["kind"] == "unmapped-observation"
            && issue["artifact_ref"] == "code.place-order-handler"
    }));
    let missing_text = project.run(&["project", "validate-bindings"]);
    assert!(!missing_text.status.success());
    let missing_stdout = String::from_utf8_lossy(&missing_text.stdout);
    assert!(missing_stdout.contains("Next: run agentic project observe"));
    assert!(missing_stdout.contains("Binding candidates are never applied automatically"));

    fs::write(
        project.root.join("src/duplicate_service.java"),
        "class A { void save() { orders.insert(null); } }\n\
         class B { void save() { orders.insert(null); } }\n",
    )
    .unwrap();
    let mut observation = read_yaml(&observation_path);
    observation["artifacts"]
        .as_array_mut()
        .unwrap()
        .push(json!({
            "ref": "code.duplicate-service-java",
            "path": "src/duplicate_service.java",
            "language": "java",
            "bindings": {
                "symbols": {
                    "save": {
                        "logical_ref": "operation.place-order",
                        "owner": "team.ordering",
                        "authority_ref": "decision.repository-bindings",
                    },
                },
                "resources": {
                    "orders": {
                        "logical_ref": "data.orders",
                        "owner": "team.ordering",
                        "authority_ref": "decision.repository-bindings",
                    },
                },
                "methods": {},
            },
        }));
    write_yaml(&observation_path, &observation);
    run_git(&project.root, &["add", "-A"]);
    run_git(
        &project.root,
        &["commit", "--quiet", "-m", "add invalid bindings"],
    );

    let ambiguous = project.run(&["project", "validate-bindings", "--format", "json"]);
    assert!(!ambiguous.status.success());
    let report: Value = serde_json::from_slice(&ambiguous.stdout).unwrap();
    assert!(report["issues"].as_array().unwrap().iter().any(|issue| {
        issue["kind"] == "ambiguous-symbol-binding"
            && issue["artifact_ref"] == "code.duplicate-service-java"
    }));
}

#[test]
fn project_validate_bindings_reports_unaccepted_authority_and_coverage_blocks() {
    let project = TestProject::new();
    let observation_path = project.root.join(".agentic/repository-observation.yaml");
    let mut observation = read_yaml(&observation_path);
    observation["artifacts"][0]["bindings"]["methods"]["unused.save"] = json!({
        "kind": "db_write",
        "owner": "team.ordering",
        "authority_ref": "decision.unused-binding",
    });
    write_yaml(&observation_path, &observation);
    let unused = project.run(&["project", "validate-bindings", "--format", "json"]);
    assert!(!unused.status.success());
    let report: Value = serde_json::from_slice(&unused.stdout).unwrap();
    assert!(report["issues"].as_array().unwrap().iter().any(|issue| {
        issue["kind"] == "unaccepted-binding-authority"
            && issue["reason"]
                .as_str()
                .is_some_and(|reason| reason.contains("decision.unused-binding"))
    }));
    observation["artifacts"][0]["bindings"]["methods"]
        .as_object_mut()
        .unwrap()
        .remove("unused.save");
    write_yaml(&observation_path, &observation);

    let decision_path = project
        .root
        .join("decisions/decision.repository-bindings.yaml");
    let mut decision = read_yaml(&decision_path);
    decision["status"] = json!("proposed");
    write_yaml(&decision_path, &decision);

    let unaccepted = project.run(&["project", "validate-bindings", "--format", "json"]);
    assert!(!unaccepted.status.success());
    let report: Value = serde_json::from_slice(&unaccepted.stdout).unwrap();
    assert_eq!(report["status"], "invalid");
    assert!(report["issues"].as_array().unwrap().iter().any(|issue| {
        issue["kind"] == "unaccepted-binding-authority"
            && issue["reason"]
                .as_str()
                .is_some_and(|reason| reason.contains("decision.repository-bindings"))
    }));

    decision["status"] = json!("accepted");
    write_yaml(&decision_path, &decision);
    fs::write(
        project.root.join("src/place_order.py"),
        "def place_order(:\n",
    )
    .unwrap();
    let blocked = project.run(&["project", "validate-bindings", "--format", "json"]);
    assert!(!blocked.status.success());
    let report: Value = serde_json::from_slice(&blocked.stdout).unwrap();
    assert_eq!(report["status"], "blocked");
    assert!(
        report["issues"]
            .as_array()
            .unwrap()
            .iter()
            .any(|issue| { issue["category"] == "coverage" && issue["kind"] == "parse-error" })
    );
}

#[test]
fn typescript_artifact_runs_through_the_real_project_loader() {
    assert_language_artifact_runs_through_real_loader(
        "src/place_order.ts",
        "typescript",
        "export function place_order(order: Order) {\n  orders.insert(order);\n}\n",
        "place_order",
    );
}

fn assert_language_artifact_runs_through_real_loader(
    path: &str,
    language: &str,
    source: &str,
    symbol: &str,
) {
    assert_language_artifact_with_optional_method_binding(path, language, source, symbol, None);
}

fn assert_language_artifact_with_optional_method_binding(
    path: &str,
    language: &str,
    source: &str,
    symbol: &str,
    method_binding: Option<&str>,
) {
    let project = TestProject::new();
    fs::remove_file(project.root.join("src/place_order.py")).unwrap();
    fs::write(project.root.join(path), source).unwrap();
    let observation_path = project.root.join(".agentic/repository-observation.yaml");
    let mut observation = read_yaml(&observation_path);
    observation["artifacts"][0]["path"] = Value::String(path.to_owned());
    observation["artifacts"][0]["language"] = Value::String(language.to_owned());
    observation["artifacts"][0]["bindings"]["symbols"] = json!({
        (symbol): {
            "logical_ref": "operation.place-order",
            "owner": "team.ordering",
            "authority_ref": "decision.repository-bindings",
        },
    });
    if let Some(method_key) = method_binding {
        observation["artifacts"][0]["bindings"]["methods"][method_key] = json!({
            "kind": "db_write",
            "owner": "team.ordering",
            "authority_ref": "decision.repository-bindings",
        });
    }
    write_yaml(&observation_path, &observation);
    run_git(&project.root, &["add", "-A"]);
    run_git(
        &project.root,
        &["commit", "--quiet", "-m", "use language fixture"],
    );

    let output = project.run(&["next", "change.place-order", "--format", "json"]);
    assert_success(&output);
    let output: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(output["state"], "needs-analysis");
    assert!(output["diagnostics"].as_array().is_some_and(Vec::is_empty));
}

#[cfg(unix)]
#[test]
fn project_commands_refuse_symlinked_project_paths() {
    use std::os::unix::fs::symlink;

    let root = temporary_test_root("project-symlink");
    let outside = temporary_test_root("project-symlink-outside");
    fs::create_dir_all(&root).unwrap();
    fs::create_dir_all(&outside).unwrap();
    fs::write(outside.join("config.yaml"), "schema_version: \"1\"\n").unwrap();
    symlink(&outside, root.join(".agentic")).unwrap();
    run_git(&root, &["init", "--quiet"]);

    let change = Command::new(env!("CARGO_BIN_EXE_agentic-vnext-rust"))
        .args([
            "change",
            "init",
            "change.example",
            "--title",
            "Example",
            "--intent",
            "Must remain inside the repository",
            "--project",
        ])
        .arg(&root)
        .output()
        .unwrap();
    assert!(!change.status.success());
    assert!(String::from_utf8_lossy(&change.stderr).contains("symlinked project path"));
    assert!(!outside.join("changes/change.example/change.yaml").exists());

    let _ = fs::remove_file(root.join(".agentic"));
    let _ = fs::remove_dir_all(root);
    let _ = fs::remove_dir_all(outside);
}

#[test]
fn project_next_explain_and_contract_health_share_the_real_project_loader() {
    let project = TestProject::new();
    let next = project.run(&["next", "change.place-order", "--format", "json"]);
    assert_success(&next);
    let next: Value = serde_json::from_slice(&next.stdout).unwrap();
    validate_output_schema(&next, "next-response.schema.json");
    assert_eq!(next["schema_version"], "1");
    assert_eq!(next["state"], "needs-analysis");
    assert_eq!(next["next_action"]["action"], "review-risk-signals");
    assert_eq!(next["context"]["source_refs"].as_array().unwrap().len(), 4);

    let explain = project.run(&["explain", "change.place-order", "--format", "json"]);
    assert_success(&explain);
    let explain: Value = serde_json::from_slice(&explain.stdout).unwrap();
    validate_output_schema(&explain, "explain-report.schema.json");
    assert_eq!(explain["schema_version"], "1");
    assert_eq!(explain["state"], next["state"]);
    assert_eq!(explain["candidates"].as_array().unwrap().len(), 3);
    assert_eq!(
        explain["next_action"]["id"], next["next_action"]["id"],
        "next and explain must use the same Kernel decision"
    );

    let text = project.run(&["explain", "change.place-order"]);
    assert_success(&text);
    let text = String::from_utf8(text.stdout).unwrap();
    assert!(text.contains("state: needs-analysis"));
    assert!(text.contains("next: Analyst/review-risk-signals"));

    let health = project.run(&["contract-health", "--format", "json", "--require-clean"]);
    assert_success(&health);
    let health: Value = serde_json::from_slice(&health.stdout).unwrap();
    validate_output_schema(&health, "contract-health-report.schema.json");
    assert_eq!(health["schema_version"], "1");
    assert_eq!(health["summary"]["verified"], 0);
    assert_eq!(
        health["summary"]["unverified"], health["summary"]["total"],
        "a new project must not imply Contract verification"
    );

    let text = project.run(&["contract-health"]);
    assert_success(&text);
    let text = String::from_utf8(text.stdout).unwrap();
    assert!(text.contains("unverified"));
}

#[test]
fn contract_health_policy_turns_the_report_into_an_explicit_ci_gate() {
    let project = TestProject::new();
    let policy_path = project.root.join(".agentic/contract-health-policy.yaml");
    let blocking_policy = json!({
        "schema_version": "1",
        "fail_on": ["failed", "unverified"]
    });
    validate_ci_schema(&blocking_policy, "contract-health-policy.schema.json");
    write_yaml(&policy_path, &blocking_policy);

    let failed = project.run(&[
        "contract-health",
        "--policy",
        ".agentic/contract-health-policy.yaml",
        "--format",
        "json",
    ]);
    assert!(!failed.status.success());
    assert!(failed.stderr.is_empty());
    let failed: Value = serde_json::from_slice(&failed.stdout).unwrap();
    validate_output_schema(&failed, "contract-health-gate-report.schema.json");
    assert_eq!(failed["status"], "failed");
    assert_eq!(
        failed["blocking_clause_refs"].as_array().unwrap().len(),
        failed["contract_health"]["summary"]["unverified"]
            .as_u64()
            .unwrap() as usize
    );

    write_yaml(
        &policy_path,
        &json!({
            "schema_version": "1",
            "fail_on": ["stale"]
        }),
    );
    let passed = project.run(&[
        "contract-health",
        "--policy",
        ".agentic/contract-health-policy.yaml",
        "--format",
        "json",
    ]);
    assert_success(&passed);
    let passed: Value = serde_json::from_slice(&passed.stdout).unwrap();
    validate_output_schema(&passed, "contract-health-gate-report.schema.json");
    assert_eq!(passed["status"], "passed");
    assert_eq!(passed["blocking_clause_refs"], json!([]));
}

#[test]
fn contract_health_gate_rejects_invalid_or_untracked_policy() {
    let project = TestProject::new();
    let policy_path = project.root.join(".agentic/contract-health-policy.yaml");
    write_yaml(
        &policy_path,
        &json!({
            "schema_version": "1",
            "fail_on": ["verified"]
        }),
    );
    let invalid = project.run(&[
        "contract-health",
        "--policy",
        ".agentic/contract-health-policy.yaml",
    ]);
    assert!(!invalid.status.success());
    assert!(
        String::from_utf8_lossy(&invalid.stderr)
            .contains("unsupported Contract Health failure state: verified")
    );

    write_yaml(
        &policy_path,
        &json!({
            "schema_version": "1",
            "fail_on": ["failed"]
        }),
    );
    fs::write(
        project.root.join(".git/info/exclude"),
        ".agentic/contract-health-policy.yaml\n",
    )
    .unwrap();
    let untracked = project.run(&[
        "contract-health",
        "--policy",
        ".agentic/contract-health-policy.yaml",
        "--require-clean",
    ]);
    assert!(!untracked.status.success());
    assert!(
        String::from_utf8_lossy(&untracked.stderr)
            .contains("required project input is not tracked by Git")
    );
}

#[test]
fn stdio_mcp_lists_typed_tools_and_persists_an_issued_result() {
    let project = TestProject::new();
    let mut child = Command::new(env!("CARGO_BIN_EXE_agentic-vnext-rust"))
        .args(["mcp", "--project"])
        .arg(&project.root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    let mut output = BufReader::new(child.stdout.take().unwrap());

    mcp_send(
        &mut input,
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-11-25",
                "capabilities": {},
                "clientInfo": {"name": "integration-test", "version": "1"}
            }
        }),
    );
    let initialized = mcp_receive(&mut output);
    assert_eq!(initialized["id"], 1);
    assert_eq!(initialized["result"]["protocolVersion"], "2025-11-25");
    mcp_send(
        &mut input,
        json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized"
        }),
    );

    mcp_send(
        &mut input,
        json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}}),
    );
    let listed = mcp_receive(&mut output);
    let tools = listed["result"]["tools"].as_array().unwrap();
    let names = tools
        .iter()
        .filter_map(|tool| tool["name"].as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        names,
        [
            "agentic_abandon_action",
            "agentic_add_evidence",
            "agentic_apply_contract",
            "agentic_apply_decision",
            "agentic_contract_health",
            "agentic_explain",
            "agentic_next",
            "agentic_submit",
        ]
    );
    for name in ["agentic_next", "agentic_submit"] {
        let tool = tools
            .iter()
            .find(|tool| tool["name"].as_str() == Some(name))
            .unwrap();
        assert!(tool["inputSchema"].is_object());
        assert!(tool["outputSchema"].is_object());
    }
    let contract_tool = tools
        .iter()
        .find(|tool| tool["name"].as_str() == Some("agentic_apply_contract"))
        .unwrap();
    assert!(
        contract_tool["inputSchema"]["properties"]["expected_clause_digests"].is_object(),
        "Contract tool must expose clause-scoped optimistic locking"
    );

    let next_call = json!({
        "jsonrpc": "2.0",
        "id": 3,
        "method": "tools/call",
        "params": {
            "name": "agentic_next",
            "arguments": {"change_id": "change.place-order"}
        }
    });
    mcp_send(&mut input, next_call);
    let next = mcp_receive(&mut output);
    assert_eq!(next["result"]["isError"], false);
    let next = &next["result"]["structuredContent"];
    validate_mcp_schema(next, "next-output.schema.json");
    assert_eq!(next["next_response"]["state"], "needs-analysis");
    assert_eq!(
        next["next_response"]["next_action"]["action"],
        "review-risk-signals"
    );

    let key = next["issued_action"].clone();
    let context = &next["next_response"]["context"]["payload"];
    let reviewed_candidates = context["signal_candidates"]
        .as_array()
        .unwrap()
        .iter()
        .map(|candidate| {
            json!({
                "fingerprint": candidate["fingerprint"],
                "status": "confirmed",
                "reason": "MCP integration testで確認した",
                "basis_refs": candidate["evidence_refs"],
            })
        })
        .collect::<Vec<_>>();
    let outcomes = context["requirement_instances"]
        .as_array()
        .unwrap()
        .iter()
        .map(|instance| {
            let basis_refs = instance["sources"]
                .as_object()
                .unwrap()
                .keys()
                .cloned()
                .collect::<Vec<_>>();
            json!({
                "instance_key": instance["instance_key"],
                "definition_digest": instance["definition_digest"],
                "status": "satisfied",
                "summary": "MCP integration testで確認した",
                "basis_refs": basis_refs,
            })
        })
        .collect::<Vec<_>>();
    let arguments = json!({
        "change_id": key["change_id"],
        "action_id": key["action_id"],
        "context_digest": key["context_digest"],
        "payload": {
            "reviewed_candidates": reviewed_candidates,
            "outcomes": outcomes,
        },
        "output_refs": [],
    });

    let mut wrong_arguments = arguments.clone();
    wrong_arguments["action_id"] = Value::String("action.not-issued".to_owned());
    mcp_send(
        &mut input,
        json!({
            "jsonrpc": "2.0",
            "id": 4,
            "method": "tools/call",
            "params": {"name": "agentic_submit", "arguments": wrong_arguments}
        }),
    );
    let rejected = mcp_receive(&mut output);
    assert_eq!(rejected["result"]["isError"], true);
    assert_eq!(
        rejected["result"]["structuredContent"]["code"],
        "ACTION_NOT_ISSUED"
    );

    mcp_send(
        &mut input,
        json!({
            "jsonrpc": "2.0",
            "id": 5,
            "method": "tools/call",
            "params": {"name": "agentic_submit", "arguments": arguments}
        }),
    );
    let submitted = mcp_receive(&mut output);
    assert_eq!(submitted["result"]["isError"], false);
    let submitted = &submitted["result"]["structuredContent"];
    validate_mcp_schema(submitted, "submit-output.schema.json");
    assert_eq!(submitted["already_completed"], false);
    assert!(
        submitted["result_id"]
            .as_str()
            .unwrap()
            .starts_with("result.")
    );
    assert_eq!(
        fs::read_dir(
            project
                .root
                .join(".agentic/changes/change.place-order/results")
        )
        .unwrap()
        .count(),
        1
    );

    mcp_send(
        &mut input,
        json!({
            "jsonrpc": "2.0",
            "id": 6,
            "method": "tools/call",
            "params": {"name": "agentic_submit", "arguments": arguments}
        }),
    );
    let retried = mcp_receive(&mut output);
    assert_eq!(retried["result"]["isError"], false);
    assert_eq!(
        retried["result"]["structuredContent"]["already_completed"],
        true
    );

    drop(input);
    assert!(child.wait().unwrap().success());
}

fn mcp_send(input: &mut ChildStdin, message: Value) {
    serde_json::to_writer(&mut *input, &message).unwrap();
    input.write_all(b"\n").unwrap();
    input.flush().unwrap();
}

fn mcp_receive(output: &mut BufReader<ChildStdout>) -> Value {
    let mut line = String::new();
    assert_ne!(output.read_line(&mut line).unwrap(), 0, "MCP server closed");
    serde_json::from_str(&line).unwrap()
}

#[test]
fn source_artifact_without_a_binding_record_blocks_the_project() {
    let project = TestProject::new();
    fs::write(
        project.root.join("src/unbound.py"),
        "def delete_order(order):\n    orders.delete(order)\n",
    )
    .unwrap();

    let output = project.run(&["next", "change.place-order", "--format", "json"]);
    assert_success(&output);
    let output: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(output["state"], "blocked-detection");
    assert!(
        output["diagnostics"][0]
            .as_str()
            .unwrap()
            .contains("unbound-source-artifact")
    );

    let text = project.run(&["next", "change.place-order"]);
    assert_success(&text);
    let stdout = String::from_utf8_lossy(&text.stdout);
    assert!(stdout.contains("Next: run agentic project observe"));
    assert!(stdout.contains("Then: run agentic project validate-bindings"));
    assert!(stdout.contains("Binding candidates are never applied automatically"));
}

#[test]
fn java_artifact_runs_through_the_real_project_loader() {
    assert_language_artifact_runs_through_real_loader(
        "src/OrderService.java",
        "java",
        "class OrderService {\n  void place_order(Order order) {\n    orders.insert(order);\n  }\n}\n",
        "place_order",
    );
}

#[test]
fn ambiguous_short_symbol_binding_blocks_same_named_methods() {
    let project = TestProject::new();
    fs::write(
        project.root.join("src/duplicate_service.java"),
        "class A { void save() { orders.insert(null); } }\n\
         class B { void save() { orders.insert(null); } }\n",
    )
    .unwrap();
    let observation_path = project.root.join(".agentic/repository-observation.yaml");
    let mut observation = read_yaml(&observation_path);
    observation["artifacts"]
        .as_array_mut()
        .unwrap()
        .push(json!({
            "ref": "code.duplicate-service-java",
            "path": "src/duplicate_service.java",
            "language": "java",
            "bindings": {
                "symbols": {
                    "save": {
                        "logical_ref": "operation.place-order",
                        "owner": "team.ordering",
                        "authority_ref": "decision.repository-bindings",
                    },
                },
                "resources": {
                    "orders": {
                        "logical_ref": "data.orders",
                        "owner": "team.ordering",
                        "authority_ref": "decision.repository-bindings",
                    },
                },
                "methods": {},
            },
        }));
    write_yaml(&observation_path, &observation);
    run_git(&project.root, &["add", "-A"]);
    run_git(
        &project.root,
        &["commit", "--quiet", "-m", "add duplicate methods"],
    );

    let output = project.run(&["next", "change.place-order", "--format", "json"]);
    assert_success(&output);
    let output: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(output["state"], "blocked-detection");
    assert!(
        output["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| {
                item.as_str()
                    .is_some_and(|message| message.contains("ambiguous-symbol-binding"))
            })
    );
}

#[test]
fn declared_inventory_only_language_reports_unsupported_language() {
    let project = TestProject::new();
    fs::write(
        project.root.join("src/order_service.cpp"),
        "int main() { return 0; }\n",
    )
    .unwrap();
    let observation_path = project.root.join(".agentic/repository-observation.yaml");
    let mut observation = read_yaml(&observation_path);
    observation["artifacts"]
        .as_array_mut()
        .unwrap()
        .push(json!({
            "ref": "code.order-service-cpp",
            "path": "src/order_service.cpp",
            "language": "cpp",
            "bindings": {
                "symbols": {},
                "resources": {},
                "methods": {},
            },
        }));
    write_yaml(&observation_path, &observation);
    run_git(&project.root, &["add", "-A"]);
    run_git(
        &project.root,
        &["commit", "--quiet", "-m", "declare cpp source"],
    );

    let output = project.run(&["next", "change.place-order", "--format", "json"]);
    assert_success(&output);
    let output: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(output["state"], "blocked-detection");
    assert!(
        output["diagnostics"][0]
            .as_str()
            .unwrap()
            .contains("unsupported-language")
    );
}

#[test]
fn go_artifact_runs_through_the_real_project_loader() {
    assert_language_artifact_runs_through_real_loader(
        "src/order_service.go",
        "go",
        "package service\n\nfunc PlaceOrder(order Order) {\n    orders.Insert(order)\n}\n",
        "PlaceOrder",
    );
}

#[test]
fn rust_artifact_runs_through_the_real_project_loader() {
    assert_language_artifact_runs_through_real_loader(
        "src/order_service.rs",
        "rust",
        "fn place_order(order: Order) {\n    orders.insert(order);\n}\n",
        "place_order",
    );
}

#[test]
fn kotlin_artifact_runs_through_the_real_project_loader() {
    assert_language_artifact_runs_through_real_loader(
        "src/OrderService.kt",
        "kotlin",
        "fun placeOrder(order: Order) {\n    orders.insert(order)\n}\n",
        "placeOrder",
    );
}

#[test]
fn ruby_artifact_runs_through_the_real_project_loader() {
    assert_language_artifact_runs_through_real_loader(
        "src/order_service.rb",
        "ruby",
        "def place_order(order)\n  orders.insert(order)\nend\n",
        "place_order",
    );
}

#[test]
fn php_artifact_runs_through_the_real_project_loader() {
    assert_language_artifact_runs_through_real_loader(
        "src/order_service.php",
        "php",
        "<?php\nfunction place_order($order) {\n    orders::insert($order);\n}\n",
        "place_order",
    );
}

#[test]
fn csharp_artifact_runs_through_the_real_project_loader() {
    assert_language_artifact_runs_through_real_loader(
        "src/OrderService.cs",
        "csharp",
        "sealed class OrderService {\n    void PlaceOrder(Order order) {\n        orders.Insert(order);\n    }\n}\n",
        "PlaceOrder",
    );
}

#[test]
fn swift_artifact_runs_through_the_real_project_loader() {
    assert_language_artifact_runs_through_real_loader(
        "src/OrderService.swift",
        "swift",
        "final class OrderService {\n    func placeOrder(_ order: Order) {\n        orders.insert(order)\n    }\n}\n",
        "placeOrder",
    );
}

#[test]
fn scala_artifact_runs_through_the_real_project_loader() {
    assert_language_artifact_runs_through_real_loader(
        "src/OrderService.scala",
        "scala",
        "final class OrderService {\n  def placeOrder(order: Order): Unit = {\n    orders.insert(order)\n  }\n}\n",
        "placeOrder",
    );
}

#[test]
fn c_artifact_runs_through_the_real_project_loader() {
    assert_language_artifact_runs_through_real_loader(
        "src/place_order.c",
        "c",
        "void place_order(Order *order) {\n    memcpy(buffer, order, sizeof(*order));\n    insert(orders, order);\n}\n",
        "place_order",
    );
}

#[test]
fn gdscript_artifact_runs_through_the_real_project_loader() {
    assert_language_artifact_runs_through_real_loader(
        "src/place_order.gd",
        "gdscript",
        "extends Node\nfunc place_order(order):\n    orders.insert(order)\n",
        "place_order",
    );
}

#[test]
fn gdscript_framework_method_binding_runs_through_the_real_project_loader() {
    assert_language_artifact_with_optional_method_binding(
        "src/place_order.gd",
        "gdscript",
        "extends Node\nfunc place_order(order):\n    orders.save(order)\n",
        "place_order",
        Some("orders.save"),
    );
}

#[test]
fn c_framework_function_binding_runs_through_the_real_project_loader() {
    assert_language_artifact_with_optional_method_binding(
        "src/place_order.c",
        "c",
        "void place_order(Order *order) {\n    sqlite3_exec(orders, \"INSERT\", 0, 0, 0);\n}\n",
        "place_order",
        Some("orders.sqlite3_exec"),
    );
}

#[test]
fn syntax_error_blocks_detection() {
    let project = TestProject::new();
    fs::write(
        project.root.join("src/place_order.py"),
        "def place_order(:\n",
    )
    .unwrap();

    let output = project.run(&["next", "change.place-order", "--format", "json"]);
    assert_success(&output);
    let output: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(output["state"], "blocked-detection");
    assert!(
        output["diagnostics"][0]
            .as_str()
            .unwrap()
            .contains("parse-error")
    );
}

#[test]
fn observed_resource_without_a_binding_blocks_detection() {
    let project = TestProject::new();
    let observation_path = project.root.join(".agentic/repository-observation.yaml");
    let mut observation = read_yaml(&observation_path);
    observation["artifacts"][0]["bindings"]["resources"] = json!({});
    write_yaml(&observation_path, &observation);

    let output = project.run(&["next", "change.place-order", "--format", "json"]);
    assert_success(&output);
    let output: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(output["state"], "blocked-detection");
    assert!(
        output["diagnostics"][0]
            .as_str()
            .unwrap()
            .contains("unmapped-observation")
    );
}

#[test]
fn framework_candidate_does_not_bypass_reviewed_method_binding() {
    let project = TestProject::new();
    fs::write(
        project.root.join("src/place_order.py"),
        "from django.db import models\n\
         def place_order(order):\n\
         \x20   orders.save(order)\n",
    )
    .unwrap();

    let output = project.run(&["next", "change.place-order", "--format", "json"]);
    assert_success(&output);
    let output: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(output["state"], "blocked-detection");
    assert!(
        output["diagnostics"][0]
            .as_str()
            .unwrap()
            .contains("unsupported-observation")
    );
}

#[test]
fn messaging_candidate_does_not_bypass_reviewed_method_binding() {
    let project = TestProject::new();
    fs::write(
        project.root.join("src/place_order.py"),
        "from celery import shared_task\n\
         def place_order(order):\n\
         \x20   orders.delay(order)\n",
    )
    .unwrap();

    let output = project.run(&["next", "change.place-order", "--format", "json"]);
    assert_success(&output);
    let output: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(output["state"], "blocked-detection");
    assert!(
        output["diagnostics"][0]
            .as_str()
            .unwrap()
            .contains("unsupported-observation")
    );
}

#[test]
fn reviewed_framework_method_binding_classifies_a_non_builtin_method() {
    let project = TestProject::new();
    fs::write(
        project.root.join("src/place_order.py"),
        "from django.db import models\n\
         def place_order(order):\n\
         \x20   orders.save(order)\n",
    )
    .unwrap();
    let observation_path = project.root.join(".agentic/repository-observation.yaml");
    let mut observation = read_yaml(&observation_path);
    observation["artifacts"][0]["bindings"]["methods"]["orders.save"] = json!({
        "kind": "db_write",
        "owner": "team.ordering",
        "authority_ref": "decision.repository-bindings",
    });
    write_yaml(&observation_path, &observation);

    let output = project.run(&["next", "change.place-order", "--format", "json"]);
    assert_success(&output);
    let output: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(output["state"], "needs-analysis");
    assert!(output["diagnostics"].as_array().is_some_and(Vec::is_empty));
}

#[test]
fn reviewed_messaging_method_binding_classifies_a_non_builtin_method() {
    let project = TestProject::new();
    fs::write(
        project.root.join("src/place_order.py"),
        "from celery import shared_task\n\
         def place_order(order):\n\
         \x20   orders.delay(order)\n",
    )
    .unwrap();
    let observation_path = project.root.join(".agentic/repository-observation.yaml");
    let mut observation = read_yaml(&observation_path);
    observation["artifacts"][0]["bindings"]["resources"]["orders"]["logical_ref"] =
        json!("integration.order-tasks");
    observation["artifacts"][0]["bindings"]["methods"]["orders.delay"] = json!({
        "kind": "message_publish",
        "owner": "team.ordering",
        "authority_ref": "decision.repository-bindings",
    });
    write_yaml(&observation_path, &observation);

    let output = project.run(&["next", "change.place-order", "--format", "json"]);
    assert_success(&output);
    let output: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(output["state"], "needs-analysis");
    assert!(output["diagnostics"].as_array().is_some_and(Vec::is_empty));
}

#[test]
fn reviewed_external_call_binding_emits_generic_and_specific_signals() {
    let project = TestProject::new();
    fs::write(
        project.root.join("src/place_order.py"),
        "def place_order(order):\n\
         \x20   payment_client.request(order)\n",
    )
    .unwrap();
    let observation_path = project.root.join(".agentic/repository-observation.yaml");
    let mut observation = read_yaml(&observation_path);
    observation["artifacts"][0]["bindings"]["resources"]["payment_client"] = json!({
        "logical_ref": "integration.payment-provider",
        "owner": "team.ordering",
        "authority_ref": "decision.repository-bindings",
    });
    observation["artifacts"][0]["bindings"]["methods"]["payment_client.request"] = json!({
        "kind": "external_call",
        "owner": "team.ordering",
        "authority_ref": "decision.repository-bindings",
    });
    write_yaml(&observation_path, &observation);

    let output = project.run(&["next", "change.place-order", "--format", "json"]);
    assert_success(&output);
    let output: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(output["state"], "needs-analysis");
    assert!(output["diagnostics"].as_array().is_some_and(Vec::is_empty));
    assert_eq!(
        signals_for_binding(&output, "integration", "integration.payment-provider"),
        ["distributed-effect", "external-system-call"]
    );
}

#[test]
fn reviewed_object_write_binding_emits_generic_and_specific_signals() {
    let project = TestProject::new();
    fs::write(
        project.root.join("src/place_order.py"),
        "def place_order(order):\n\
         \x20   archive_bucket.put_object(order)\n",
    )
    .unwrap();
    let observation_path = project.root.join(".agentic/repository-observation.yaml");
    let mut observation = read_yaml(&observation_path);
    observation["artifacts"][0]["bindings"]["resources"]["archive_bucket"] = json!({
        "logical_ref": "data.order-archive",
        "owner": "team.ordering",
        "authority_ref": "decision.repository-bindings",
    });
    observation["artifacts"][0]["bindings"]["methods"]["archive_bucket.put_object"] = json!({
        "kind": "object_write",
        "owner": "team.ordering",
        "authority_ref": "decision.repository-bindings",
    });
    write_yaml(&observation_path, &observation);

    let output = project.run(&["next", "change.place-order", "--format", "json"]);
    assert_success(&output);
    let output: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(output["state"], "needs-analysis");
    assert!(output["diagnostics"].as_array().is_some_and(Vec::is_empty));
    assert_eq!(
        signals_for_binding(&output, "data", "data.order-archive"),
        ["object-storage-write", "persistent-data-write"]
    );
}

#[test]
fn reviewed_multi_fact_binding_emits_external_and_object_write_signals() {
    let project = TestProject::new();
    fs::write(
        project.root.join("src/place_order.py"),
        "def place_order(order):\n\
         \x20   archive_bucket.put_object(order)\n",
    )
    .unwrap();
    let observation_path = project.root.join(".agentic/repository-observation.yaml");
    let mut observation = read_yaml(&observation_path);
    observation["schema_version"] = json!("5");
    observation["artifacts"][0]["bindings"]["resources"]["archive_bucket"] = json!({
        "logical_refs": {
            "data": "data.order-archive",
            "integration": "integration.amazon-s3",
        },
        "owner": "team.ordering",
        "authority_ref": "decision.repository-bindings",
    });
    observation["artifacts"][0]["bindings"]["methods"]["archive_bucket.put_object"] = json!({
        "fact_kinds": ["external_call", "object_write"],
        "owner": "team.ordering",
        "authority_ref": "decision.repository-bindings",
    });
    write_yaml(&observation_path, &observation);

    let output = project.run(&["next", "change.place-order", "--format", "json"]);
    assert_success(&output);
    let output: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(output["state"], "needs-analysis");
    assert!(output["diagnostics"].as_array().is_some_and(Vec::is_empty));
    assert_eq!(
        signals_for_binding(&output, "integration", "integration.amazon-s3"),
        ["distributed-effect", "external-system-call"]
    );
    assert_eq!(
        signals_for_binding(&output, "data", "data.order-archive"),
        ["object-storage-write", "persistent-data-write"]
    );
}

#[test]
fn multi_fact_binding_fails_closed_when_one_logical_ref_is_missing() {
    let project = TestProject::new();
    fs::write(
        project.root.join("src/place_order.py"),
        "def place_order(order):\n\
         \x20   archive_bucket.put_object(order)\n",
    )
    .unwrap();
    let observation_path = project.root.join(".agentic/repository-observation.yaml");
    let mut observation = read_yaml(&observation_path);
    observation["schema_version"] = json!("5");
    observation["artifacts"][0]["bindings"]["resources"]["archive_bucket"] = json!({
        "logical_refs": {"data": "data.order-archive"},
        "owner": "team.ordering",
        "authority_ref": "decision.repository-bindings",
    });
    observation["artifacts"][0]["bindings"]["methods"]["archive_bucket.put_object"] = json!({
        "fact_kinds": ["external_call", "object_write"],
        "owner": "team.ordering",
        "authority_ref": "decision.repository-bindings",
    });
    write_yaml(&observation_path, &observation);

    let output = project.run(&["next", "change.place-order", "--format", "json"]);
    assert_success(&output);
    let output: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(output["state"], "blocked-detection");
    assert!(
        output["diagnostics"][0]
            .as_str()
            .unwrap()
            .contains("has no integration logical ref required by external_call")
    );
    assert!(output["context"].is_null());
}

#[test]
fn reviewed_security_bindings_emit_authorization_and_sensitive_data_signals() {
    let project = TestProject::new();
    fs::write(
        project.root.join("src/place_order.py"),
        "def place_order(order):\n\
         \x20   permissions.grant(order)\n\
         \x20   customers.find(order)\n",
    )
    .unwrap();
    let observation_path = project.root.join(".agentic/repository-observation.yaml");
    let mut observation = read_yaml(&observation_path);
    observation["schema_version"] = json!("5");
    observation["artifacts"][0]["bindings"]["resources"]["permissions"] = json!({
        "logical_refs": {
            "authorization": "authorization.order-administration",
        },
        "owner": "team.security",
        "authority_ref": "decision.repository-bindings",
    });
    observation["artifacts"][0]["bindings"]["resources"]["customers"] = json!({
        "logical_refs": {
            "data": "data.customer-pii",
        },
        "owner": "team.security",
        "authority_ref": "decision.repository-bindings",
    });
    observation["artifacts"][0]["bindings"]["methods"]["permissions.grant"] = json!({
        "fact_kinds": ["authorization_change"],
        "owner": "team.security",
        "authority_ref": "decision.repository-bindings",
    });
    observation["artifacts"][0]["bindings"]["methods"]["customers.find"] = json!({
        "fact_kinds": ["sensitive_data_access"],
        "owner": "team.security",
        "authority_ref": "decision.repository-bindings",
    });
    write_yaml(&observation_path, &observation);

    let output = project.run(&["next", "change.place-order", "--format", "json"]);
    assert_success(&output);
    let output: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(output["state"], "needs-analysis");
    assert!(output["diagnostics"].as_array().is_some_and(Vec::is_empty));
    assert_eq!(
        signals_for_binding(
            &output,
            "authorization",
            "authorization.order-administration"
        ),
        ["authorization-control-change"]
    );
    assert_eq!(
        signals_for_binding(&output, "data", "data.customer-pii"),
        ["sensitive-data-access"]
    );
}

#[test]
fn unknown_method_binding_kind_is_rejected_before_detection() {
    let project = TestProject::new();
    let observation_path = project.root.join(".agentic/repository-observation.yaml");
    let mut observation = read_yaml(&observation_path);
    observation["artifacts"][0]["bindings"]["methods"]["orders.save"] = json!({
        "kind": "unknown_write",
        "owner": "team.ordering",
        "authority_ref": "decision.repository-bindings",
    });
    write_yaml(&observation_path, &observation);

    let output = project.run(&["next", "change.place-order", "--format", "json"]);
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("artifact method binding kind is not supported: unknown_write")
    );
}

fn signals_for_binding(output: &Value, binding: &str, logical_ref: &str) -> Vec<String> {
    output["context"]["payload"]["signal_candidates"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|candidate| candidate["bindings"][binding].as_str() == Some(logical_ref))
        .map(|candidate| candidate["signal"].as_str().unwrap().to_owned())
        .collect()
}

fn run_framework_e2e(case_id: &str, expected_frameworks: &[&str]) -> Value {
    let manifest_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let corpus_root = manifest_root.join("../benchmarks/major-frameworks-v1/projects");
    let project = TestProject::new();
    copy_tree(&corpus_root.join(case_id), &project.root);
    run_git(&project.root, &["add", "-A"]);
    run_git(
        &project.root,
        &["commit", "--quiet", "-m", "add framework fixture"],
    );
    let draft_relative = format!(".agentic/{case_id}.draft.yaml");
    let observed = project.run(&[
        "project",
        "observe",
        "--analysis-root",
        ".",
        "--output",
        &draft_relative,
    ]);
    assert_success(&observed);

    let draft_path = project.root.join(&draft_relative);
    let mut draft = read_yaml(&draft_path);
    let frameworks = complete_framework_draft(&mut draft, case_id);
    assert_eq!(
        frameworks,
        expected_frameworks
            .iter()
            .map(|framework| (*framework).to_owned())
            .collect::<BTreeSet<_>>(),
        "unexpected framework candidates for {case_id}"
    );
    write_yaml(&draft_path, &draft);

    let reviewed = project.run(&["project", "validate-bindings", "--draft", &draft_relative]);
    assert_success(&reviewed);
    let promoted = project.run(&["project", "promote-bindings", "--draft", &draft_relative]);
    assert_success(&promoted);
    let authoritative = project.run(&["project", "validate-bindings"]);
    assert_success(&authoritative);

    let next = project.run(&["next", "change.place-order", "--format", "json"]);
    assert_success(&next);
    let next: Value = serde_json::from_slice(&next.stdout).unwrap();
    assert_eq!(next["state"], "needs-analysis", "failed case: {case_id}");
    assert!(
        next["diagnostics"].as_array().is_some_and(Vec::is_empty),
        "unexpected diagnostics for {case_id}: {}",
        next["diagnostics"]
    );
    next
}

fn complete_framework_draft(draft: &mut Value, case_id: &str) -> BTreeSet<String> {
    let inventory = draft["artifacts"].as_array().unwrap().clone();
    let mut frameworks = BTreeSet::new();
    for artifact in &inventory {
        frameworks.extend(
            artifact["framework_candidates"]
                .as_array()
                .unwrap()
                .iter()
                .map(|candidate| candidate["framework"].as_str().unwrap().to_owned()),
        );
    }

    for binding_artifact in draft["binding_artifacts"].as_array_mut().unwrap() {
        let path = binding_artifact["path"].as_str().unwrap();
        let artifact = inventory
            .iter()
            .find(|artifact| artifact["path"].as_str() == Some(path))
            .unwrap();
        let candidates = artifact["framework_candidates"].as_array().unwrap();
        let mut symbols = serde_json::Map::new();
        let mut resources = BTreeMap::<String, BTreeMap<String, String>>::new();
        let mut methods = serde_json::Map::new();

        for observation in artifact["observations"].as_array().unwrap() {
            let candidate = candidates.iter().find(|candidate| {
                candidate["symbol"] == observation["symbol"]
                    && candidate["resource"] == observation["resource"]
                    && candidate["method"] == observation["method"]
                    && candidate["line"] == observation["line"]
            });
            let fact_kinds = match observation["kind"].as_str().unwrap() {
                "db_write" => vec!["db_write".to_owned()],
                "message_publish" => vec!["message_publish".to_owned()],
                "other_method_call" => {
                    let Some(candidate) = candidate else {
                        continue;
                    };
                    let suggested = candidate["suggested_fact_kinds"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|kind| kind.as_str().unwrap().to_owned())
                        .collect::<Vec<_>>();
                    if suggested.is_empty() {
                        explicitly_reviewed_fact_kinds(candidate, case_id)
                    } else {
                        suggested
                    }
                }
                kind => panic!("unexpected observation kind in {case_id}: {kind}"),
            };
            let symbol = observation["symbol"].as_str().unwrap();
            symbols.entry(symbol.to_owned()).or_insert_with(|| {
                json!({
                    "logical_ref": "operation.place-order",
                    "owner": "team.ordering",
                    "authority_ref": "decision.repository-bindings",
                })
            });
            let resource = observation["resource"].as_str().unwrap();
            let resource_bindings = resources.entry(resource.to_owned()).or_default();
            for fact_kind in &fact_kinds {
                let (binding, logical_ref) = logical_ref_for_fact(fact_kind, case_id);
                if let Some(existing) =
                    resource_bindings.insert(binding.to_owned(), logical_ref.clone())
                {
                    assert_eq!(
                        existing, logical_ref,
                        "{case_id} maps one physical resource binding to conflicting logical refs"
                    );
                }
            }
            if observation["kind"] == "other_method_call" {
                let candidate = candidate.unwrap();
                methods.insert(
                    candidate["binding_key"].as_str().unwrap().to_owned(),
                    json!({
                        "fact_kinds": fact_kinds,
                        "owner": "team.ordering",
                        "authority_ref": "decision.repository-bindings",
                    }),
                );
            }
        }

        let resources = resources
            .into_iter()
            .map(|(physical, bindings)| {
                (
                    physical,
                    json!({
                        "logical_refs": bindings,
                        "owner": "team.ordering",
                        "authority_ref": "decision.repository-bindings",
                    }),
                )
            })
            .collect::<serde_json::Map<_, _>>();
        binding_artifact["bindings"] = json!({
            "symbols": symbols,
            "resources": resources,
            "methods": methods,
        });
    }
    frameworks
}

fn explicitly_reviewed_fact_kinds(candidate: &Value, case_id: &str) -> Vec<String> {
    match (
        candidate["framework"].as_str(),
        candidate["method"].as_str(),
    ) {
        (Some("sqlalchemy"), Some("execute")) => vec!["db_write".to_owned()],
        (Some("amazon-s3"), Some("send")) => {
            vec!["external_call".to_owned(), "object_write".to_owned()]
        }
        _ => panic!("{case_id} requires an explicit reviewed classification: {candidate}"),
    }
}

fn logical_ref_for_fact(fact_kind: &str, case_id: &str) -> (&'static str, String) {
    match fact_kind {
        "db_write" => ("data", format!("data.persistence-e2e-{case_id}")),
        "object_write" => ("data", format!("data.object-e2e-{case_id}")),
        "sensitive_data_access" => ("data", format!("data.sensitive-e2e-{case_id}")),
        "external_call" => ("integration", format!("integration.external-e2e-{case_id}")),
        "message_publish" => (
            "integration",
            format!("integration.messaging-e2e-{case_id}"),
        ),
        "authorization_change" => (
            "authorization",
            format!("authorization.control-e2e-{case_id}"),
        ),
        other => panic!("unsupported reviewed fact kind: {other}"),
    }
}

#[test]
fn local_cli_observes_dirty_artifacts_but_clean_mode_rejects_them() {
    let project = TestProject::new();
    fs::write(
        project.root.join("src/place_order.py"),
        "def place_order():\n    return 'dirty'\n",
    )
    .unwrap();

    let local = project.run(&["next", "change.place-order", "--format", "json"]);
    assert_success(&local);

    let clean = project.run(&[
        "next",
        "change.place-order",
        "--format",
        "json",
        "--require-clean",
    ]);
    assert!(!clean.status.success());
    assert!(
        String::from_utf8_lossy(&clean.stderr).contains("working tree is not clean"),
        "unexpected stderr: {}",
        String::from_utf8_lossy(&clean.stderr)
    );
}

#[test]
fn release_resolver_rejects_tampered_rule_assets() {
    let project = TestProject::new();
    fs::write(
        project.release_root.join("rules.yaml"),
        "requirements: []\nrules: []\n",
    )
    .unwrap();

    let output = project.run(&["next", "change.place-order", "--format", "json"]);
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("file digest mismatch"),
        "unexpected stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn release_resolver_rejects_tampered_schema_assets() {
    let project = TestProject::new();
    fs::write(
        project.release_root.join("schemas/v1/change.schema.json"),
        "{}\n",
    )
    .unwrap();

    let output = project.run(&["next", "change.place-order"]);
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("file digest mismatch"),
        "unexpected stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn release_resolver_rejects_asset_path_escape() {
    let project = TestProject::new();
    let mut lock = read_yaml(&project.root.join(".agentic/framework.lock"));
    lock["schema_version"] = Value::String("1".to_owned());
    lock.as_object_mut().unwrap().remove("release_artifact");
    write_yaml(&project.root.join(".agentic/framework.lock"), &lock);
    fs::write(
        project.release_root.join("release.yaml"),
        "schema_version: \"1\"\n\
         release_id: prototype-vnext-dev\n\
         assets:\n\
         \x20 rules: ../rules.yaml\n\
         \x20 schemas: schemas/v1\n",
    )
    .unwrap();

    let output = project.run(&["explain", "change.place-order"]);
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("escapes Release root"),
        "unexpected stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn release_resolver_rejects_invalid_signatures() {
    let project = TestProject::new();
    let manifest_path = project.release_root.join("release.yaml");
    let mut manifest = read_yaml(&manifest_path);
    let signature = manifest["signature"].as_str().unwrap();
    let replacement = if signature.ends_with('0') { "1" } else { "0" };
    manifest["signature"] = Value::String(format!(
        "{}{replacement}",
        &signature[..signature.len() - 1]
    ));
    write_yaml(&manifest_path, &manifest);

    let output = project.run(&["next", "change.place-order"]);
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("signature verification failed"),
        "unexpected stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn signed_offline_bundle_is_installed_atomically() {
    let project = TestProject::new();
    let bundle = project.root.join("offline-bundle");
    copy_tree(&project.release_root, &bundle);
    fs::remove_dir_all(&project.release_root).unwrap();

    let output = project.run(&[
        "release",
        "install",
        "offline-bundle",
        "--lock",
        ".agentic/framework.lock",
    ]);
    assert_success(&output);
    assert!(project.release_root.join("release.yaml").is_file());

    let next = project.run(&["next", "change.place-order", "--format", "json"]);
    assert_success(&next);
}

#[test]
fn remote_archive_reuses_verified_install_and_switch_boundaries() {
    let project = TestProject::new();
    let (candidate, bundle) = prepare_remote_candidate(&project);
    let archive = tar_release(&bundle);
    let (base_url, server) = serve_once(200, archive, None);
    write_release_source(&project.root, &base_url);
    fs::remove_dir_all(&project.release_root).unwrap();

    let fetched = project.run(&["release", "fetch", candidate.to_str().unwrap()]);
    assert_success(&fetched);
    server.join().unwrap();
    assert!(project.release_root.join("release.yaml").is_file());

    let switched = project.run(&["release", "switch", candidate.to_str().unwrap()]);
    assert_success(&switched);
    assert_success(&project.run(&["next", "change.place-order"]));
}

#[test]
fn remote_archive_rejects_links_without_partial_install() {
    let project = TestProject::new();
    let (candidate, _bundle) = prepare_remote_candidate(&project);
    let (base_url, server) = serve_once(200, symlink_archive(), None);
    write_release_source(&project.root, &base_url);
    fs::remove_dir_all(&project.release_root).unwrap();

    let output = project.run(&["release", "fetch", candidate.to_str().unwrap()]);
    assert!(!output.status.success());
    server.join().unwrap();
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("only regular files and directories"),
        "unexpected stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!project.release_root.exists());
}

#[test]
fn remote_fetch_does_not_follow_unreviewed_redirects() {
    let project = TestProject::new();
    let (candidate, _bundle) = prepare_remote_candidate(&project);
    let (base_url, server) = serve_once(302, Vec::new(), Some("http://127.0.0.1:9/unreviewed.tar"));
    write_release_source(&project.root, &base_url);
    fs::remove_dir_all(&project.release_root).unwrap();

    let output = project.run(&["release", "fetch", candidate.to_str().unwrap()]);
    assert!(!output.status.success());
    server.join().unwrap();
    assert!(!project.release_root.exists());
}

#[test]
fn publisher_is_reproducible_non_overwriting_and_consumable() {
    let project = TestProject::new();
    let source = project.root.join("publisher-source");
    copy_tree(&project.release_root, &source);
    let seed = "07".repeat(32);
    let first = project.run_with_env(
        &publisher_arguments(
            "publisher-source",
            "published/release-a.tar",
            "published/framework-a.lock",
        ),
        "AGENTIC_RELEASE_SIGNING_KEY_HEX",
        &seed,
    );
    assert_success(&first);
    let receipt: Value = serde_json::from_slice(&first.stdout).unwrap();
    validate_delivery_schema(&receipt, "publish-receipt.schema.json");
    assert_eq!(receipt["release_id"], "prototype-vnext-dev");
    let second = project.run_with_env(
        &publisher_arguments(
            "publisher-source",
            "published/release-b.tar",
            "published/framework-b.lock",
        ),
        "AGENTIC_RELEASE_SIGNING_KEY_HEX",
        &seed,
    );
    assert_success(&second);
    assert_eq!(
        fs::read(project.root.join("published/release-a.tar")).unwrap(),
        fs::read(project.root.join("published/release-b.tar")).unwrap()
    );
    assert_eq!(
        fs::read(project.root.join("published/framework-a.lock")).unwrap(),
        fs::read(project.root.join("published/framework-b.lock")).unwrap()
    );

    let archive_before = fs::read(project.root.join("published/release-a.tar")).unwrap();
    let duplicate = project.run_with_env(
        &publisher_arguments(
            "publisher-source",
            "published/release-a.tar",
            "published/framework-a.lock",
        ),
        "AGENTIC_RELEASE_SIGNING_KEY_HEX",
        &seed,
    );
    assert!(!duplicate.status.success());
    assert_eq!(
        archive_before,
        fs::read(project.root.join("published/release-a.tar")).unwrap()
    );

    let trust_path = project.root.join(".agentic/trusted-release-keys.yaml");
    let mut trust = read_yaml(&trust_path);
    trust["keys"][0]["allowed_sources"]
        .as_array_mut()
        .unwrap()
        .push(Value::String("remote:test-fixture".to_owned()));
    write_yaml(&trust_path, &trust);
    let archive = fs::read(project.root.join("published/release-a.tar")).unwrap();
    fs::remove_dir_all(&project.release_root).unwrap();

    let candidate = project.root.join("published/framework-a.lock");
    let marker = b"requirements:";
    let position = archive
        .windows(marker.len())
        .position(|window| window == marker)
        .expect("published archive contains Rule source");
    let mut tampered = archive;
    tampered[position] = b'X';
    fs::write(project.root.join("published/tampered.tar"), tampered).unwrap();
    let rejected = project.run(&[
        "release",
        "install-archive",
        "published/tampered.tar",
        "--lock",
        candidate.to_str().unwrap(),
    ]);
    assert!(!rejected.status.success());
    assert!(!project.release_root.exists());

    assert_success(&project.run(&[
        "release",
        "install-archive",
        "published/release-a.tar",
        "--lock",
        candidate.to_str().unwrap(),
    ]));
    assert_success(&project.run(&["release", "switch", candidate.to_str().unwrap()]));
    assert_success(&project.run(&["next", "change.place-order"]));
}

#[test]
fn publisher_rejects_missing_or_invalid_signing_secrets() {
    let project = TestProject::new();
    let source = project.root.join("publisher-source");
    copy_tree(&project.release_root, &source);
    let arguments = publisher_arguments(
        "publisher-source",
        "published/release.tar",
        "published/framework.lock",
    );

    let missing = project.run(&arguments);
    assert!(!missing.status.success());
    assert!(String::from_utf8_lossy(&missing.stderr).contains("AGENTIC_RELEASE_SIGNING_KEY_HEX"));
    let invalid = project.run_with_env(
        &arguments,
        "AGENTIC_RELEASE_SIGNING_KEY_HEX",
        "not-a-secret-key",
    );
    assert!(!invalid.status.success());
    assert!(!project.root.join("published/release.tar").exists());
    assert!(!project.root.join("published/framework.lock").exists());

    let expected_public_key = "00".repeat(32);
    let mut wrong_key_arguments = publisher_arguments(
        "publisher-source",
        "published/wrong-key.tar",
        "published/wrong-key.lock",
    );
    wrong_key_arguments.extend(["--expected-public-key", &expected_public_key]);
    let wrong_key = project.run_with_env(
        &wrong_key_arguments,
        "AGENTIC_RELEASE_SIGNING_KEY_HEX",
        &"07".repeat(32),
    );
    assert!(!wrong_key.status.success());
    assert!(String::from_utf8_lossy(&wrong_key.stderr).contains("expected signer public key"));
    assert!(!project.root.join("published/wrong-key.tar").exists());
    assert!(!project.root.join("published/wrong-key.lock").exists());

    let mut incompatible = read_yaml(&project.root.join(".agentic/framework.lock"));
    incompatible["protocols"]["kernel"] = Value::String("unexpected".to_owned());
    write_yaml(
        &project.root.join(".agentic/incompatible-publisher.lock"),
        &incompatible,
    );
    let mut incompatible_arguments = arguments;
    incompatible_arguments[4] = ".agentic/incompatible-publisher.lock";
    let rejected = project.run_with_env(
        &incompatible_arguments,
        "AGENTIC_RELEASE_SIGNING_KEY_HEX",
        &"07".repeat(32),
    );
    assert!(!rejected.status.success());
    assert!(String::from_utf8_lossy(&rejected.stderr).contains("protocols.kernel"));
    assert!(!project.root.join("published/release.tar").exists());

    fs::write(
        source.join("framework-catalog.yaml"),
        "schema_version: '1'\nnamespace: agentic\nrules: []\n",
    )
    .unwrap();
    let invalid_catalog = project.run_with_env(
        &publisher_arguments(
            "publisher-source",
            "published/invalid-catalog.tar",
            "published/invalid-catalog.lock",
        ),
        "AGENTIC_RELEASE_SIGNING_KEY_HEX",
        &"07".repeat(32),
    );
    assert!(!invalid_catalog.status.success());
    assert!(String::from_utf8_lossy(&invalid_catalog.stderr).contains("namespace agentic"));
    assert!(!project.root.join("published/invalid-catalog.tar").exists());
    assert!(!project.root.join("published/invalid-catalog.lock").exists());
}

#[test]
fn publication_record_schema_pins_candidate_provenance_and_asset_digests() {
    let record = json!({
        "schema_version": "1",
        "release_id": "prototype-vnext-dev",
        "release_tag": "framework-prototype-vnext-dev",
        "source_revision": "1".repeat(40),
        "candidate_workflow_run_id": "12345",
        "source_id": "remote:test-fixture",
        "signer_key_id": "test.framework.release",
        "artifact_digest": format!("sha256:{}", "2".repeat(64)),
        "archive_digest": format!("sha256:{}", "3".repeat(64)),
        "signer_public_key": "4".repeat(64),
        "asset_digests": {
            "candidate-framework.lock": format!("sha256:{}", "5".repeat(64)),
            "distribution-trust.json": format!("sha256:{}", "6".repeat(64)),
            "framework-release.tar": format!("sha256:{}", "6".repeat(64)),
            "publish-receipt.json": format!("sha256:{}", "7".repeat(64)),
        },
        "binary_asset_digests": {
            "SHA256SUMS": format!("sha256:{}", "8".repeat(64)),
            "agentic-vnext-rust-aarch64-apple-darwin":
                format!("sha256:{}", "9".repeat(64)),
            "agentic-vnext-rust-aarch64-apple-darwin.build.json":
                format!("sha256:{}", "a".repeat(64)),
            "agentic-vnext-rust-aarch64-unknown-linux-gnu":
                format!("sha256:{}", "b".repeat(64)),
            "agentic-vnext-rust-aarch64-unknown-linux-gnu.build.json":
                format!("sha256:{}", "c".repeat(64)),
            "agentic-vnext-rust-x86_64-apple-darwin":
                format!("sha256:{}", "d".repeat(64)),
            "agentic-vnext-rust-x86_64-apple-darwin.build.json":
                format!("sha256:{}", "e".repeat(64)),
            "agentic-vnext-rust-x86_64-pc-windows-msvc.exe":
                format!("sha256:{}", "f".repeat(64)),
            "agentic-vnext-rust-x86_64-pc-windows-msvc.exe.build.json":
                format!("sha256:{}", "1".repeat(64)),
            "agentic-vnext-rust-x86_64-unknown-linux-gnu":
                format!("sha256:{}", "2".repeat(64)),
            "agentic-vnext-rust-x86_64-unknown-linux-gnu.build.json":
                format!("sha256:{}", "3".repeat(64)),
        },
    });
    validate_delivery_schema(&record, "publication-record.schema.json");
}

#[test]
fn distribution_trust_schema_pins_the_release_key_and_source_policy() {
    let trust = json!({
        "schema_version": "1",
        "release_id": "prototype-vnext-dev",
        "keys": [{
            "id": "test.framework.release",
            "algorithm": "ed25519",
            "public_key": "4".repeat(64),
            "allowed_sources": ["remote:test-fixture"],
            "status": "active",
        }],
    });
    validate_delivery_schema(&trust, "distribution-trust.schema.json");
}

#[test]
fn binary_build_record_schema_pins_target_revision_and_digest() {
    let record = json!({
        "schema_version": "1",
        "binary_name": "agentic-vnext-rust-x86_64-unknown-linux-gnu",
        "target": "x86_64-unknown-linux-gnu",
        "source_revision": "1".repeat(40),
        "sha256": format!("sha256:{}", "2".repeat(64)),
        "size": 123456,
        "rustc_version": "rustc 1.89.0 (29483883e 2025-08-04)",
    });
    validate_delivery_schema(&record, "binary-build-record.schema.json");
}

#[test]
fn failed_install_does_not_leave_a_partial_release() {
    let project = TestProject::new();
    let bundle = project.root.join("tampered-bundle");
    copy_tree(&project.release_root, &bundle);
    fs::write(bundle.join("rules.yaml"), "rules: []\nrequirements: []\n").unwrap();
    fs::remove_dir_all(&project.release_root).unwrap();

    let output = project.run(&[
        "release",
        "install",
        "tampered-bundle",
        "--lock",
        ".agentic/framework.lock",
    ]);
    assert!(!output.status.success());
    assert!(!project.release_root.exists());
    let releases = project.root.join(".agentic/cache/releases");
    assert!(fs::read_dir(releases).unwrap().all(|entry| {
        !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .contains(".install-")
    }));
}

#[test]
fn framework_lock_switch_creates_a_validated_rollback_point() {
    let project = TestProject::new();
    let candidate = project.root.join(".agentic/candidate-framework.lock");
    fs::copy(project.root.join(".agentic/framework.lock"), &candidate).unwrap();
    let mut legacy = read_yaml(&project.root.join(".agentic/framework.lock"));
    legacy["schema_version"] = Value::String("1".to_owned());
    legacy.as_object_mut().unwrap().remove("release_artifact");
    write_yaml(&project.root.join(".agentic/framework.lock"), &legacy);

    let switched = project.run(&["release", "switch", ".agentic/candidate-framework.lock"]);
    assert_success(&switched);
    assert_eq!(
        read_yaml(&project.root.join(".agentic/framework.lock"))["schema_version"],
        "2"
    );
    let backup = fs::read_dir(project.root.join(".agentic/cache/framework-lock-backups"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let backup = backup.to_string_lossy().into_owned();
    set_test_key_status(&project.root, "retired");
    let rolled_back = project.run(&["release", "rollback", &backup]);
    assert_success(&rolled_back);
    assert_eq!(
        read_yaml(&project.root.join(".agentic/framework.lock"))["schema_version"],
        "1"
    );
}

#[test]
fn retired_keys_allow_runtime_and_rollback_but_not_new_install() {
    let project = TestProject::new();
    set_test_key_status(&project.root, "retired");

    let runtime = project.run(&["next", "change.place-order"]);
    assert_success(&runtime);

    fs::copy(
        project.root.join(".agentic/framework.lock"),
        project.root.join(".agentic/retired-candidate.lock"),
    )
    .unwrap();
    let switch = project.run(&["release", "switch", ".agentic/retired-candidate.lock"]);
    assert!(!switch.status.success());
    assert!(String::from_utf8_lossy(&switch.stderr).contains("retired"));

    let bundle = project.root.join("retired-key-bundle");
    copy_tree(&project.release_root, &bundle);
    fs::remove_dir_all(&project.release_root).unwrap();
    let install = project.run(&[
        "release",
        "install",
        "retired-key-bundle",
        "--lock",
        ".agentic/framework.lock",
    ]);
    assert!(!install.status.success());
    assert!(
        String::from_utf8_lossy(&install.stderr).contains("retired"),
        "unexpected stderr: {}",
        String::from_utf8_lossy(&install.stderr)
    );
    assert!(!project.release_root.exists());
}

#[test]
fn revoked_keys_stop_even_an_already_installed_release() {
    let project = TestProject::new();
    set_test_key_status(&project.root, "revoked");

    let output = project.run(&["next", "change.place-order"]);
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("has been revoked"),
        "unexpected stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn overlapping_keys_allow_a_reviewable_signer_rotation() {
    let project = TestProject::new();
    let rotated_bundle = project.root.join("rotated-bundle");
    copy_tree(&project.release_root, &rotated_bundle);
    let next_key = SigningKey::from_bytes(&[8_u8; 32]);
    let artifact_digest = write_signed_release(
        &rotated_bundle,
        &next_key,
        "test.framework.release.next",
        "offline:test-fixture",
    );

    let trust_path = project.root.join(".agentic/trusted-release-keys.yaml");
    let mut trust = read_yaml(&trust_path);
    trust["keys"][0]["status"] = Value::String("retired".to_owned());
    trust["keys"].as_array_mut().unwrap().push(json!({
        "id": "test.framework.release.next",
        "algorithm": "ed25519",
        "public_key": encode_hex(&next_key.verifying_key().to_bytes()),
        "allowed_sources": ["offline:test-fixture"],
        "status": "active",
    }));
    write_yaml(&trust_path, &trust);

    let lock_path = project.root.join(".agentic/framework.lock");
    let mut lock = read_yaml(&lock_path);
    lock["release_artifact"]["artifact_digest"] = Value::String(artifact_digest);
    lock["release_artifact"]["signer_key_id"] =
        Value::String("test.framework.release.next".to_owned());
    write_yaml(&lock_path, &lock);

    let output = project.run(&["next", "change.place-order", "--release", "rotated-bundle"]);
    assert_success(&output);
}

#[test]
fn framework_lock_pins_the_logical_release_source() {
    let project = TestProject::new();
    let lock_path = project.root.join(".agentic/framework.lock");
    let mut lock = read_yaml(&lock_path);
    lock["release_artifact"]["source_id"] = Value::String("offline:other".to_owned());
    write_yaml(&lock_path, &lock);

    let output = project.run(&["next", "change.place-order"]);
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("source mismatch"),
        "unexpected stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn switch_rejects_a_candidate_with_an_incompatible_runtime_protocol() {
    let project = TestProject::new();
    let active_path = project.root.join(".agentic/framework.lock");
    let active_before = fs::read_to_string(&active_path).unwrap();
    let mut candidate = read_yaml(&active_path);
    candidate["protocols"]["kernel"] = Value::String("unexpected".to_owned());
    write_yaml(
        &project.root.join(".agentic/incompatible-framework.lock"),
        &candidate,
    );

    let output = project.run(&["release", "switch", ".agentic/incompatible-framework.lock"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("protocols.kernel"));
    assert_eq!(active_before, fs::read_to_string(active_path).unwrap());
}

#[test]
fn explicit_offline_release_uses_the_same_lock_verification() {
    let project = TestProject::new();
    let offline = project.root.join("offline-release");
    copy_tree(&project.release_root, &offline);
    let offline = offline.to_string_lossy().into_owned();

    let output = project.run(&[
        "next",
        "change.place-order",
        "--release",
        &offline,
        "--format",
        "json",
    ]);
    assert_success(&output);
    let output: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(output["state"], "needs-analysis");
}

#[test]
fn clean_mode_rejects_ignored_untracked_authoritative_records() {
    let project = TestProject::new();
    fs::write(
        project.root.join("contracts/ignored-contract.yaml"),
        "schema_version: \"1\"\n\
         id: contract.ignored\n\
         change_id: change.place-order\n\
         applies_to:\n\
         \x20 - operation.place-order\n\
         clauses:\n\
         \x20 - id: ignored\n\
         \x20   text: This ignored Contract must not enter a clean evaluation.\n",
    )
    .unwrap();

    let output = project.run(&[
        "next",
        "change.place-order",
        "--require-clean",
        "--format",
        "json",
    ]);
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("required project input is not tracked by Git"),
        "unexpected stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "command failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn temporary_test_root(label: &str) -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    std::env::temp_dir().join(format!(
        "agentic-vnext-{label}-{}-{}",
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ))
}

fn legacy_migration_project(label: &str) -> PathBuf {
    let root = temporary_test_root(label);
    fs::create_dir_all(root.join(".agentic/changes/change.checkout")).unwrap();
    fs::create_dir_all(root.join("contracts/project")).unwrap();
    fs::create_dir_all(root.join("decisions")).unwrap();
    fs::create_dir_all(root.join("evidence")).unwrap();
    fs::write(
        root.join(".agentic/installation.yaml"),
        "schema_version: 2\nkit_version: '3.0.0'\nproject: shop\nmode: adopt\nlevel: standard\ninstalled_at: '2026-08-02'\nskill_path: .agents/skills/agentic-development\ncli_path: .agentic/bin/agentic\n",
    )
    .unwrap();
    fs::write(
        root.join(".agentic/config.yaml"),
        "schema_version: 1\ncontract_roots: [contracts]\ndecision_root: decisions\nevidence_root: evidence\npolicies: {}\nruntime: {python: '>=3.10', dependencies: .agentic/runtime/requirements.txt}\n",
    )
    .unwrap();
    fs::write(
        root.join(".agentic/active-changes.yaml"),
        "schema_version: 1\nchanges: [{id: checkout, status: assessing}]\n",
    )
    .unwrap();
    fs::write(
        root.join(".agentic/changes/change.checkout/change.yaml"),
        "schema_version: 1\nid: checkout\ntitle: Checkout\nstatus: assessing\nrisk: R1\naffected: {}\nfeature_contract: features/checkout\n",
    )
    .unwrap();
    fs::write(
        root.join("contracts/project/constitution.yaml"),
        "schema_version: 2\nid: project/constitution\nkind: project\nstatus: accepted\nversion: 1\n",
    )
    .unwrap();
    fs::write(
        root.join("decisions/DEC-001.md"),
        "# DEC-001\n\nUse the reviewed checkout boundary.\n",
    )
    .unwrap();
    fs::write(
        root.join("evidence/checkout.yaml"),
        "schema_version: 1\nchange: checkout\nrequirements: []\nresidual_risks: []\n",
    )
    .unwrap();
    run_git(&root, &["init", "--quiet"]);
    run_git(&root, &["config", "user.email", "migration@example.test"]);
    run_git(&root, &["config", "user.name", "Migration Fixture"]);
    run_git(&root, &["add", "."]);
    run_git(&root, &["commit", "--quiet", "-m", "legacy project"]);
    root
}

fn complete_migration_reviews(draft: &mut Value) {
    for action in draft["actions"].as_array_mut().unwrap() {
        if action["requires_human_review"] == true {
            action["review"] = json!({
                "decision": "proceed",
                "reviewer": "migration-reviewer",
                "rationale": format!(
                    "Reviewed migration action {} against the current Project.",
                    action["id"].as_str().unwrap()
                ),
                "evidence_refs": ["decision:migration-review"]
            });
        }
    }
}

struct TestProject {
    root: PathBuf,
    release_root: PathBuf,
}

impl TestProject {
    fn new() -> Self {
        use std::sync::atomic::{AtomicU64, Ordering};
        static SEQUENCE: AtomicU64 = AtomicU64::new(0);
        let manifest_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let root = std::env::temp_dir().join(format!(
            "agentic-vnext-cli-{}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        if root.exists() {
            fs::remove_dir_all(&root).unwrap();
        }
        copy_tree(&manifest_root.join("../fixtures/cli-project"), &root);
        let release_root = root
            .join(".agentic/cache/releases")
            .join("prototype-vnext-dev");
        fs::create_dir_all(&release_root).unwrap();
        fs::copy(
            manifest_root.join("../fixtures/db-sqs/rules.yaml"),
            release_root.join("rules.yaml"),
        )
        .unwrap();
        fs::copy(
            manifest_root.join("../fixtures/framework-catalog/framework-catalog.yaml"),
            release_root.join("framework-catalog.yaml"),
        )
        .unwrap();
        validate_catalog_schema(
            &read_yaml(&release_root.join("framework-catalog.yaml")),
            "framework-catalog.schema.json",
        );
        copy_tree(
            &manifest_root.join("../schemas/v1"),
            &release_root.join("schemas/v1"),
        );
        let signing_key = SigningKey::from_bytes(&[7_u8; 32]);
        let artifact_digest = write_signed_release(
            &release_root,
            &signing_key,
            "test.framework.release",
            "offline:test-fixture",
        );
        validate_delivery_schema(
            &read_yaml(&release_root.join("release.yaml")),
            "release-manifest.schema.json",
        );
        let trust_store = json!({
            "schema_version": "2",
            "keys": [{
                "id": "test.framework.release",
                "algorithm": "ed25519",
                "public_key": encode_hex(&signing_key.verifying_key().to_bytes()),
                "allowed_sources": ["offline:test-fixture"],
                "status": "active",
            }],
        });
        validate_delivery_schema(&trust_store, "trusted-release-keys-v2.schema.json");
        write_yaml(
            &root.join(".agentic/trusted-release-keys.yaml"),
            &trust_store,
        );
        let framework_lock_path = root.join(".agentic/framework.lock");
        let mut framework_lock = read_yaml(&framework_lock_path);
        framework_lock["schema_version"] = Value::String("2".to_owned());
        framework_lock["release_artifact"] = json!({
            "artifact_digest": artifact_digest,
            "source_id": "offline:test-fixture",
            "signer_key_id": "test.framework.release",
        });
        validate_delivery_schema(&framework_lock, "framework-lock.schema.json");
        write_yaml(&framework_lock_path, &framework_lock);
        run_git(&root, &["init", "--quiet"]);
        run_git(&root, &["config", "user.email", "cli@example.test"]);
        run_git(&root, &["config", "user.name", "CLI Fixture"]);
        run_git(&root, &["add", "."]);
        run_git(&root, &["commit", "--quiet", "-m", "initial"]);
        Self { root, release_root }
    }

    fn run(&self, arguments: &[&str]) -> Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_agentic-vnext-rust"));
        command.args(arguments);
        command
            .arg("--project")
            .arg(&self.root)
            .env_remove("AGENTIC_RELEASE_SIGNING_KEY_HEX")
            .output()
            .unwrap()
    }

    fn run_with_env(&self, arguments: &[&str], variable: &str, value: &str) -> Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_agentic-vnext-rust"));
        command.args(arguments);
        command
            .arg("--project")
            .arg(&self.root)
            .env(variable, value)
            .output()
            .unwrap()
    }
}

impl Drop for TestProject {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn run_git(root: &Path, arguments: &[&str]) {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(arguments)
        .output()
        .unwrap();
    assert_success(&output);
}

fn git_output(root: &Path, arguments: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(arguments)
        .output()
        .unwrap();
    assert_success(&output);
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

fn copy_tree(source: &Path, target: &Path) {
    fs::create_dir_all(target).unwrap();
    for entry in fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let source_path = entry.path();
        let target_path = target.join(entry.file_name());
        if source_path.is_dir() {
            copy_tree(&source_path, &target_path);
        } else {
            fs::copy(source_path, target_path).unwrap();
        }
    }
}

fn publisher_arguments<'a>(source: &'a str, archive: &'a str, lock: &'a str) -> Vec<&'a str> {
    vec![
        "release",
        "build",
        source,
        "--lock",
        ".agentic/framework.lock",
        "--source-id",
        "remote:test-fixture",
        "--key-id",
        "test.framework.release",
        "--framework-catalog",
        "framework-catalog.yaml",
        "--output",
        archive,
        "--lock-output",
        lock,
        "--format",
        "json",
    ]
}

fn prepare_remote_candidate(project: &TestProject) -> (PathBuf, PathBuf) {
    let bundle = project.root.join("remote-release-bundle");
    copy_tree(&project.release_root, &bundle);
    let signing_key = SigningKey::from_bytes(&[7_u8; 32]);
    let artifact_digest = write_signed_release(
        &bundle,
        &signing_key,
        "test.framework.release",
        "remote:test-fixture",
    );
    let trust_path = project.root.join(".agentic/trusted-release-keys.yaml");
    let mut trust = read_yaml(&trust_path);
    trust["keys"][0]["allowed_sources"]
        .as_array_mut()
        .unwrap()
        .push(Value::String("remote:test-fixture".to_owned()));
    write_yaml(&trust_path, &trust);

    let mut candidate = read_yaml(&project.root.join(".agentic/framework.lock"));
    candidate["release_artifact"]["artifact_digest"] = Value::String(artifact_digest);
    candidate["release_artifact"]["source_id"] = Value::String("remote:test-fixture".to_owned());
    let path = project.root.join(".agentic/remote-framework.lock");
    write_yaml(&path, &candidate);
    (path, bundle)
}

fn write_release_source(project_root: &Path, base_url: &str) {
    let source = json!({
        "schema_version": "1",
        "sources": [{
            "id": "remote:test-fixture",
            "base_url": base_url,
        }],
    });
    validate_delivery_schema(&source, "release-sources.schema.json");
    write_yaml(&project_root.join(".agentic/release-sources.yaml"), &source);
}

fn tar_release(root: &Path) -> Vec<u8> {
    let mut output = Vec::new();
    {
        let mut archive = tar::Builder::new(&mut output);
        let mut files = release_files(root);
        files.insert("release.yaml".to_owned(), root.join("release.yaml"));
        for (relative, path) in files {
            archive.append_path_with_name(path, relative).unwrap();
        }
        archive.finish().unwrap();
    }
    output
}

fn symlink_archive() -> Vec<u8> {
    let mut output = Vec::new();
    {
        let mut archive = tar::Builder::new(&mut output);
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(tar::EntryType::Symlink);
        header.set_size(0);
        header.set_mode(0o777);
        header.set_link_name("../outside").unwrap();
        header.set_cksum();
        archive
            .append_data(&mut header, "linked-release", std::io::empty())
            .unwrap();
        archive.finish().unwrap();
    }
    output
}

fn serve_once(
    status: u16,
    body: Vec<u8>,
    location: Option<&str>,
) -> (String, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let location = location.map(str::to_owned);
    let handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = [0_u8; 4096];
        let read = stream.read(&mut request).unwrap();
        let request = String::from_utf8_lossy(&request[..read]);
        assert!(request.starts_with("GET /releases/prototype-vnext-dev.tar "));
        let reason = if status == 200 { "OK" } else { "Found" };
        let mut response = format!(
            "HTTP/1.1 {status} {reason}\r\nContent-Length: {}\r\nConnection: close\r\n",
            body.len()
        );
        if let Some(location) = location {
            response.push_str(&format!("Location: {location}\r\n"));
        }
        response.push_str("\r\n");
        stream.write_all(response.as_bytes()).unwrap();
        stream.write_all(&body).unwrap();
        stream.flush().unwrap();
    });
    (format!("http://{address}/releases"), handle)
}

fn write_signed_release(
    root: &Path,
    signing_key: &SigningKey,
    key_id: &str,
    source_id: &str,
) -> String {
    let files = release_files(root)
        .into_iter()
        .map(|(path, file)| {
            let digest = Sha256::digest(fs::read(file).unwrap());
            json!({
                "path": path,
                "digest": format!("sha256:{digest:x}"),
            })
        })
        .collect::<Vec<_>>();
    let mut assets = json!({
        "rules": "rules.yaml",
        "schemas": "schemas/v1",
    });
    if root.join("framework-catalog.yaml").is_file() {
        assets["framework_catalog"] = Value::String("framework-catalog.yaml".to_owned());
    }
    let payload = json!({
        "schema_version": "2",
        "release_id": "prototype-vnext-dev",
        "source_id": source_id,
        "assets": assets,
        "files": files,
        "signer": {
            "algorithm": "ed25519",
            "key_id": key_id,
        },
    });
    let signature = signing_key.sign(canonical_json(&payload).unwrap().as_bytes());
    let mut manifest = payload.clone();
    manifest["signature"] = Value::String(format!("ed25519:{}", encode_hex(&signature.to_bytes())));
    write_yaml(&root.join("release.yaml"), &manifest);
    canonical_digest(&payload).unwrap()
}

fn set_test_key_status(project_root: &Path, status: &str) {
    let path = project_root.join(".agentic/trusted-release-keys.yaml");
    let mut trust = read_yaml(&path);
    trust["keys"][0]["status"] = Value::String(status.to_owned());
    write_yaml(&path, &trust);
}

fn release_files(root: &Path) -> BTreeMap<String, PathBuf> {
    fn visit(root: &Path, directory: &Path, output: &mut BTreeMap<String, PathBuf>) {
        let mut entries = fs::read_dir(directory)
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            if entry.path().is_dir() {
                visit(root, &entry.path(), output);
            } else {
                let relative = entry
                    .path()
                    .strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                if relative != "release.yaml" {
                    output.insert(relative, entry.path());
                }
            }
        }
    }
    let mut output = BTreeMap::new();
    visit(root, root, &mut output);
    output
}

fn read_yaml(path: &Path) -> Value {
    serde_yaml::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

fn write_yaml(path: &Path, value: &Value) {
    fs::write(path, serde_yaml::to_string(value).unwrap()).unwrap();
}

fn encode_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn validate_output_schema(value: &Value, filename: &str) {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../schemas/outputs/v1")
        .join(filename);
    let schema: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    validate_json_document(value, &schema).unwrap();
}

fn validate_ci_schema(value: &Value, filename: &str) {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../schemas/ci/v1")
        .join(filename);
    let schema: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    validate_json_document(value, &schema).unwrap();
}

fn validate_catalog_schema(value: &Value, filename: &str) {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../schemas/catalog/v1")
        .join(filename);
    let schema: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    validate_json_document(value, &schema).unwrap();
}

fn validate_delivery_schema(value: &Value, filename: &str) {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../schemas/delivery/v2")
        .join(filename);
    let schema: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    validate_json_document(value, &schema).unwrap();
}

fn validate_mcp_schema(value: &Value, filename: &str) {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../schemas/mcp/v1")
        .join(filename);
    let schema: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    validate_json_document(value, &schema).unwrap();
}
