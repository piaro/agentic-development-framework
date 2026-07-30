use agentic_vnext_rust::schema::validate_json_document;
use agentic_vnext_rust::{canonical_digest, canonical_json};
use ed25519_dalek::{Signer, SigningKey};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
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
    assert!(root.join(".agentic/config.yaml").is_file());
    assert!(root.join(".agentic/framework.lock").is_file());
    assert!(root.join(".agentic/repository-observation.yaml").is_file());
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
    let _ = fs::remove_dir_all(root);
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
        project.root.join("src/OrderService.scala"),
        "final class OrderService\n",
    )
    .unwrap();
    let observation_path = project.root.join(".agentic/repository-observation.yaml");
    let mut observation = read_yaml(&observation_path);
    observation["artifacts"]
        .as_array_mut()
        .unwrap()
        .push(json!({
            "ref": "code.order-service-scala",
            "path": "src/OrderService.scala",
            "language": "scala",
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
        &["commit", "--quiet", "-m", "declare scala source"],
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
fn unsupported_method_on_a_bound_resource_blocks_detection() {
    let project = TestProject::new();
    fs::write(
        project.root.join("src/place_order.py"),
        "def place_order(order):\n    orders.save(order)\n",
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
        "def place_order(order):\n    orders.save(order)\n",
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
    let payload = json!({
        "schema_version": "2",
        "release_id": "prototype-vnext-dev",
        "source_id": source_id,
        "assets": {
            "rules": "rules.yaml",
            "schemas": "schemas/v1",
        },
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
