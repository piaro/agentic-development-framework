use adf::application::{ApplicationResponse, InMemoryApplication};
use adf::git_repository::GitRepositoryAdapter;
use adf::schema::SchemaRegistry;
use adf::submission::ResultSubmission;
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

#[test]
fn security_bindings_drive_the_reviewed_lifecycle() {
    let manifest_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let fixture_root = manifest_root.join("testdata/fixtures/security-lifecycle");
    let scenario = read_yaml(fixture_root.join("scenario.yaml"));
    let mut project = read_yaml(fixture_root.join("project.yaml"));
    let rules = read_yaml(manifest_root.join("testdata/fixtures/db-sqs/rules.yaml"));
    let framework_lock =
        read_yaml(manifest_root.join("testdata/fixtures/db-sqs/framework-lock.yaml"));
    let schemas = SchemaRegistry::load(manifest_root.join("schemas/v1")).unwrap();
    let repository = TestRepository::new(&fixture_root);

    let pre_build_observation = repository.observe();
    assert_eq!(pre_build_observation["coverage"]["status"], "complete");
    assert_eq!(
        string_set(&pre_build_observation["facts"], "kind"),
        BTreeSet::from(["authorization_change", "sensitive_data_access"])
    );
    for fact in pre_build_observation["facts"].as_array().unwrap() {
        assert_eq!(
            fact["binding_authority_refs"],
            json!(["decision.customer-security-boundary"])
        );
    }
    project["repository"] = pre_build_observation;

    let change_id = scenario["change_id"].as_str().unwrap();
    let mut application =
        InMemoryApplication::new(project, &rules, &framework_lock, &schemas).unwrap();
    let mut response = application.next(change_id).unwrap();

    assert_eq!(
        string_set(
            &response.context.as_ref().unwrap().payload["signal_candidates"],
            "signal"
        ),
        scenario["expected_signals"]
            .as_array()
            .unwrap()
            .iter()
            .map(|value| value.as_str().unwrap())
            .collect()
    );

    for expected in scenario["before_build"].as_array().unwrap() {
        assert_action(&response, expected);
        let payload = satisfied_payload(&response);
        response = application
            .submit(&submission(&response, change_id, payload, Vec::new()))
            .unwrap();
    }

    assert_eq!(response.decision.state, scenario["build"]["state"]);
    assert_eq!(
        response
            .decision
            .action
            .as_ref()
            .unwrap()
            .expected_result_schema,
        scenario["build"]["result_schema"]
    );
    let build_submission = submission(
        &response,
        change_id,
        json!({"summary": "顧客管理の認可と個人情報アクセスを実装した"}),
        vec!["code.customer-access-policy".to_owned()],
    );
    let post_build_observation = repository.complete_build();
    assert_eq!(post_build_observation["phase"], "post-build");
    let post_build_revision = post_build_observation["revision"]
        .as_str()
        .unwrap()
        .to_owned();
    application
        .update_repository(post_build_observation)
        .unwrap();
    response = application.submit(&build_submission).unwrap();

    let evidence_expected = &scenario["after_build"][0];
    assert_action(&response, evidence_expected);
    let shallow = application
        .submit(&submission(
            &response,
            change_id,
            satisfied_payload(&response),
            Vec::new(),
        ))
        .unwrap_err();
    assert!(
        shallow
            .to_string()
            .contains(scenario["shallow_evidence_error"].as_str().unwrap()),
        "unexpected shallow Evidence error: {shallow}"
    );

    let evidence_instance = response
        .decision
        .action
        .as_ref()
        .unwrap()
        .requirement_instances[0]
        .instance_key
        .clone();
    let evidence_id = "evidence.customer-security-probes";
    application
        .add_evidence(json!({
            "schema_version": "1",
            "id": evidence_id,
            "change_id": change_id,
            "requirement_instances": [evidence_instance],
            "contract_clause_refs": [
                "contract.customer-security#least-privilege",
                "contract.customer-security#pii-access-restricted"
            ],
            "git_revision": post_build_revision,
            "method": "security policy integration test",
            "condition": "権限外操作を拒否し、許可された個人情報項目だけを返す",
            "outcome": "passed",
            "summary": "認可拒否と個人情報の最小開示を再現可能な検査で確認した",
            "artifact": {
                "uri": "artifact://security/customer-access-policy-test",
                "digest": "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "exit_code": 0
            }
        }))
        .unwrap();
    let mut evidence_payload = satisfied_payload(&response);
    evidence_payload["outcomes"][0]["basis_refs"]
        .as_array_mut()
        .unwrap()
        .push(Value::String(evidence_id.to_owned()));
    response = application
        .submit(&submission(
            &response,
            change_id,
            evidence_payload,
            vec![evidence_id.to_owned()],
        ))
        .unwrap();

    assert_action(&response, &scenario["after_build"][1]);
    let revalidation_instances = response
        .decision
        .action
        .as_ref()
        .unwrap()
        .requirement_instances
        .iter()
        .map(|instance| instance.instance_key.clone())
        .collect::<Vec<_>>();
    let revalidation_evidence_id = "evidence.customer-security-contract-revalidation";
    application
        .add_evidence(json!({
            "schema_version": "1",
            "id": revalidation_evidence_id,
            "change_id": change_id,
            "requirement_instances": revalidation_instances,
            "contract_clause_refs": [
                "contract.customer-security#least-privilege",
                "contract.customer-security#pii-access-restricted"
            ],
            "git_revision": post_build_revision,
            "method": "security contract revalidation",
            "condition": "post-buildの実装に対してSecurity Contractを再検証する",
            "outcome": "passed",
            "summary": "認可境界と個人情報制約が実装後も成立することを確認した",
            "artifact": {
                "uri": "artifact://security/customer-security-contract-revalidation",
                "digest": "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                "exit_code": 0
            }
        }))
        .unwrap();
    let mut revalidation_payload = satisfied_payload(&response);
    for outcome in revalidation_payload["outcomes"].as_array_mut().unwrap() {
        outcome["basis_refs"]
            .as_array_mut()
            .unwrap()
            .push(Value::String(revalidation_evidence_id.to_owned()));
    }
    response = application
        .submit(&submission(
            &response,
            change_id,
            revalidation_payload,
            vec![revalidation_evidence_id.to_owned()],
        ))
        .unwrap();

    assert_action(&response, &scenario["after_build"][2]);
    response = application
        .submit(&submission(
            &response,
            change_id,
            satisfied_payload(&response),
            Vec::new(),
        ))
        .unwrap();
    assert_eq!(response.decision.state, scenario["terminal_state"]);
    assert!(response.decision.action.is_none());
    assert!(response.context.is_none());
}

fn assert_action(response: &ApplicationResponse, expected: &Value) {
    assert_eq!(
        response.decision.state, expected["state"],
        "unexpected action: {:?}; diagnostics: {:?}",
        response.decision.action, response.decision.diagnostics
    );
    let action = response.decision.action.as_ref().unwrap();
    assert_eq!(
        action.requirement_instances.len(),
        expected["instances"].as_u64().unwrap() as usize
    );
    assert!(
        action.requirement_instances.iter().all(|instance| {
            instance.requirement_id == expected["requirement"].as_str().unwrap()
        })
    );
}

fn satisfied_payload(response: &ApplicationResponse) -> Value {
    let action = response.decision.action.as_ref().unwrap();
    let context = response.context.as_ref().unwrap();
    let outcomes = action
        .requirement_instances
        .iter()
        .map(|instance| {
            let basis_refs = context.instance_source_digests[&instance.instance_key]
                .keys()
                .cloned()
                .collect::<Vec<_>>();
            json!({
                "instance_key": instance.instance_key,
                "definition_digest": instance.definition_digest,
                "status": "satisfied",
                "summary": format!("{}をfixtureで確認した", instance.requirement_id),
                "basis_refs": basis_refs,
            })
        })
        .collect::<Vec<_>>();
    if action.expected_result_schema == "result.risk-signal-review" {
        let reviews = context.payload["signal_candidates"]
            .as_array()
            .unwrap()
            .iter()
            .map(|candidate| {
                json!({
                    "fingerprint": candidate["fingerprint"],
                    "status": "confirmed",
                    "reason": "review済みBindingと変更意図が一致する",
                    "basis_refs": candidate["evidence_refs"],
                })
            })
            .collect::<Vec<_>>();
        json!({"reviewed_candidates": reviews, "outcomes": outcomes})
    } else {
        json!({"outcomes": outcomes})
    }
}

fn submission(
    response: &ApplicationResponse,
    change_id: &str,
    payload: Value,
    output_refs: Vec<String>,
) -> ResultSubmission {
    let action = response.decision.action.as_ref().unwrap();
    let context = response.context.as_ref().unwrap();
    ResultSubmission {
        change_id: change_id.to_owned(),
        action_id: action.id.clone(),
        context_digest: context.digest.clone(),
        role: action.role.clone(),
        result_schema: action.expected_result_schema.clone(),
        payload,
        output_refs,
    }
}

fn string_set<'a>(values: &'a Value, field: &str) -> BTreeSet<&'a str> {
    values
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|value| value[field].as_str())
        .collect()
}

fn read_yaml(path: PathBuf) -> Value {
    serde_yaml::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

struct TestRepository {
    root: PathBuf,
}

impl TestRepository {
    fn new(fixture_root: &Path) -> Self {
        use std::sync::atomic::{AtomicU64, Ordering};
        static SEQUENCE: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "adf-security-lifecycle-{}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        if root.exists() {
            fs::remove_dir_all(&root).unwrap();
        }
        fs::create_dir_all(root.join("src")).unwrap();
        fs::copy(
            fixture_root.join("repository-observation.yaml"),
            root.join("repository-observation.yaml"),
        )
        .unwrap();
        fs::copy(
            fixture_root.join("src/customer_access.py"),
            root.join("src/customer_access.py"),
        )
        .unwrap();
        run_git(&root, &["init", "--quiet"]);
        run_git(&root, &["config", "user.email", "security@example.test"]);
        run_git(&root, &["config", "user.name", "Security Fixture"]);
        run_git(&root, &["add", "."]);
        run_git(&root, &["commit", "--quiet", "-m", "pre-build"]);
        Self { root }
    }

    fn observe(&self) -> Value {
        GitRepositoryAdapter::new(&self.root, "repository-observation.yaml", false)
            .unwrap()
            .observe()
            .unwrap()
    }

    fn complete_build(&self) -> Value {
        let manifest_path = self.root.join("repository-observation.yaml");
        let mut manifest = read_yaml(manifest_path.clone());
        manifest["phase"] = Value::String("post-build".to_owned());
        fs::write(&manifest_path, serde_yaml::to_string(&manifest).unwrap()).unwrap();
        let source_path = self.root.join("src/customer_access.py");
        let mut source = fs::read_to_string(&source_path).unwrap();
        source.push_str("\n# implementation completed\n");
        fs::write(source_path, source).unwrap();
        run_git(&self.root, &["add", "."]);
        run_git(&self.root, &["commit", "--quiet", "-m", "post-build"]);
        self.observe()
    }
}

impl Drop for TestRepository {
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

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "command failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
