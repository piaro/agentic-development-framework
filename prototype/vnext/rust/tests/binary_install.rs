use agentic_vnext_rust::binary_install::{
    binary_install_status, current_platform_target, install_binary_candidate,
    published_binary_name, rollback_binary_install,
};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

#[test]
fn install_update_and_rollback_switch_one_activation_record() {
    let fixture = BinaryFixture::new();
    let revision_one = "1".repeat(40);
    let revision_two = "2".repeat(40);
    let candidate_one = fixture.candidate("framework-test-v1", &revision_one);
    let candidate_two = fixture.candidate("framework-test-v2", &revision_two);

    let first = install_binary_candidate(
        &candidate_one,
        "framework-test-v1",
        &revision_one,
        &fixture.install_root,
    )
    .unwrap();
    assert_eq!(first.current, "framework-test-v1");
    assert_eq!(first.previous, None);
    assert!(!first.already_installed);

    let repeated = install_binary_candidate(
        &candidate_one,
        "framework-test-v1",
        &revision_one,
        &fixture.install_root,
    )
    .unwrap();
    assert!(repeated.already_installed);
    assert_eq!(repeated.previous, None);

    let second = install_binary_candidate(
        &candidate_two,
        "framework-test-v2",
        &revision_two,
        &fixture.install_root,
    )
    .unwrap();
    assert_eq!(second.current, "framework-test-v2");
    assert_eq!(second.previous.as_deref(), Some("framework-test-v1"));
    assert_eq!(
        fs::read_to_string(fixture.install_root.join("active")).unwrap(),
        "framework-test-v2\nframework-test-v1\n"
    );

    let status = binary_install_status(&fixture.install_root).unwrap();
    assert_eq!(status.current.as_deref(), Some("framework-test-v2"));
    assert_eq!(status.previous.as_deref(), Some("framework-test-v1"));

    let rollback = rollback_binary_install(&fixture.install_root).unwrap();
    assert_eq!(rollback.current, "framework-test-v1");
    assert_eq!(rollback.previous.as_deref(), Some("framework-test-v2"));
    assert_eq!(
        fs::read_to_string(fixture.install_root.join("active")).unwrap(),
        "framework-test-v1\nframework-test-v2\n"
    );
}

#[test]
fn failed_update_preserves_the_active_release() {
    let fixture = BinaryFixture::new();
    let revision_one = "1".repeat(40);
    let revision_two = "2".repeat(40);
    let candidate_one = fixture.candidate("framework-test-v1", &revision_one);
    let candidate_two = fixture.candidate("framework-test-v2", &revision_two);
    install_binary_candidate(
        &candidate_one,
        "framework-test-v1",
        &revision_one,
        &fixture.install_root,
    )
    .unwrap();

    let binary = candidate_two.join(published_binary_name(current_platform_target().unwrap()));
    let mut bytes = fs::read(&binary).unwrap();
    bytes.extend_from_slice(b"tampered");
    fs::write(&binary, bytes).unwrap();
    let failed = install_binary_candidate(
        &candidate_two,
        "framework-test-v2",
        &revision_two,
        &fixture.install_root,
    );
    assert!(failed.is_err());

    let status = binary_install_status(&fixture.install_root).unwrap();
    assert_eq!(status.current.as_deref(), Some("framework-test-v1"));
    assert_eq!(status.previous, None);
    assert!(
        !fixture
            .install_root
            .join("releases/framework-test-v2")
            .exists()
    );
}

#[test]
fn cli_and_managed_launcher_observe_the_same_installation() {
    let fixture = BinaryFixture::new();
    let revision = "3".repeat(40);
    let candidate = fixture.candidate("framework-test-v3", &revision);
    let install = run_cli(&[
        "binary",
        "install",
        candidate.to_str().unwrap(),
        "--tag",
        "framework-test-v3",
        "--source-revision",
        &revision,
        "--install-root",
        fixture.install_root.to_str().unwrap(),
        "--format",
        "json",
    ]);
    assert_success(&install);
    let receipt: Value = serde_json::from_slice(&install.stdout).unwrap();
    assert_eq!(receipt["current"], "framework-test-v3");

    let status = run_cli(&[
        "binary",
        "status",
        "--install-root",
        fixture.install_root.to_str().unwrap(),
        "--format",
        "json",
    ]);
    assert_success(&status);
    let status: Value = serde_json::from_slice(&status.stdout).unwrap();
    assert_eq!(status["current"], "framework-test-v3");

    #[cfg(unix)]
    {
        let launched = Command::new(fixture.install_root.join("bin/agentic"))
            .args([
                "binary",
                "status",
                "--install-root",
                fixture.install_root.to_str().unwrap(),
                "--format",
                "json",
            ])
            .output()
            .unwrap();
        assert_success(&launched);
        let launched: Value = serde_json::from_slice(&launched.stdout).unwrap();
        assert_eq!(launched["current"], "framework-test-v3");
    }
}

#[test]
fn install_never_overwrites_an_unmanaged_launcher() {
    let fixture = BinaryFixture::new();
    let revision = "4".repeat(40);
    let candidate = fixture.candidate("framework-test-v4", &revision);
    let launcher = fixture.install_root.join(if cfg!(windows) {
        "bin/agentic.cmd"
    } else {
        "bin/agentic"
    });
    fs::create_dir_all(launcher.parent().unwrap()).unwrap();
    fs::write(&launcher, "user-owned launcher\n").unwrap();

    let result = install_binary_candidate(
        &candidate,
        "framework-test-v4",
        &revision,
        &fixture.install_root,
    );
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("unmanaged launcher")
    );
    assert_eq!(
        fs::read_to_string(&launcher).unwrap(),
        "user-owned launcher\n"
    );
    assert!(!fixture.install_root.join("active").exists());
}

#[cfg(unix)]
#[test]
fn install_rejects_a_symlinked_release_directory() {
    use std::os::unix::fs::symlink;

    let fixture = BinaryFixture::new();
    let revision = "5".repeat(40);
    let candidate = fixture.candidate("framework-test-v5", &revision);
    let outside = fixture.root.join("outside");
    fs::create_dir_all(&fixture.install_root).unwrap();
    fs::create_dir(&outside).unwrap();
    symlink(&outside, fixture.install_root.join("releases")).unwrap();

    let result = install_binary_candidate(
        &candidate,
        "framework-test-v5",
        &revision,
        &fixture.install_root,
    );
    assert!(result.unwrap_err().to_string().contains("real directory"));
    assert_eq!(fs::read_dir(outside).unwrap().count(), 0);
    assert!(!fixture.install_root.join("active").exists());
}

struct BinaryFixture {
    root: PathBuf,
    install_root: PathBuf,
}

impl BinaryFixture {
    fn new() -> Self {
        static SEQUENCE: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "agentic-vnext-binary-install-{}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        if root.exists() {
            fs::remove_dir_all(&root).unwrap();
        }
        fs::create_dir_all(&root).unwrap();
        Self {
            install_root: root.join("installed"),
            root,
        }
    }

    fn candidate(&self, tag: &str, revision: &str) -> PathBuf {
        let candidate = self.root.join(format!("candidate-{tag}"));
        fs::create_dir(&candidate).unwrap();
        let target = current_platform_target().unwrap();
        let binary_name = published_binary_name(target);
        let binary = candidate.join(&binary_name);
        fs::copy(env!("CARGO_BIN_EXE_agentic-vnext-rust"), &binary).unwrap();
        let binary_digest = digest(&binary);
        let build_name = format!("{binary_name}.build.json");
        let build = json!({
            "schema_version": "1",
            "binary_name": binary_name,
            "target": target,
            "source_revision": revision,
            "sha256": binary_digest,
            "size": fs::metadata(&binary).unwrap().len(),
            "rustc_version": "rustc 1.89.0 (test fixture)",
        });
        fs::write(
            candidate.join(&build_name),
            serde_json::to_vec_pretty(&build).unwrap(),
        )
        .unwrap();
        let hexadecimal_digest = digest(&binary).strip_prefix("sha256:").unwrap().to_owned();
        fs::write(
            candidate.join("SHA256SUMS"),
            format!("{hexadecimal_digest}  {binary_name}\n"),
        )
        .unwrap();

        let mut binary_digests = published_binary_digest_fixture();
        binary_digests.insert(binary_name, Value::String(digest(&binary)));
        binary_digests.insert(
            build_name.clone(),
            Value::String(digest(&candidate.join(build_name))),
        );
        binary_digests.insert(
            "SHA256SUMS".to_owned(),
            Value::String(digest(&candidate.join("SHA256SUMS"))),
        );
        let publication = json!({
            "schema_version": "1",
            "release_id": tag.strip_prefix("framework-").unwrap(),
            "release_tag": tag,
            "source_revision": revision,
            "candidate_workflow_run_id": "12345",
            "source_id": "remote:test-fixture",
            "signer_key_id": "test.framework.release",
            "artifact_digest": fake_digest('a'),
            "archive_digest": fake_digest('b'),
            "signer_public_key": "c".repeat(64),
            "asset_digests": {
                "candidate-framework.lock": fake_digest('d'),
                "framework-release.tar": fake_digest('e'),
                "publish-receipt.json": fake_digest('f'),
            },
            "binary_asset_digests": binary_digests,
        });
        fs::write(
            candidate.join("publication-record.json"),
            serde_json::to_vec_pretty(&publication).unwrap(),
        )
        .unwrap();
        candidate
    }
}

impl Drop for BinaryFixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn published_binary_digest_fixture() -> Map<String, Value> {
    let mut values = Map::new();
    for target in [
        "aarch64-apple-darwin",
        "aarch64-unknown-linux-gnu",
        "x86_64-apple-darwin",
        "x86_64-pc-windows-msvc",
        "x86_64-unknown-linux-gnu",
    ] {
        let binary = published_binary_name(target);
        values.insert(binary.clone(), Value::String(fake_digest('0')));
        values.insert(
            format!("{binary}.build.json"),
            Value::String(fake_digest('0')),
        );
    }
    values.insert("SHA256SUMS".to_owned(), Value::String(fake_digest('0')));
    values
}

fn digest(path: &Path) -> String {
    format!("sha256:{:x}", Sha256::digest(fs::read(path).unwrap()))
}

fn fake_digest(character: char) -> String {
    format!("sha256:{}", character.to_string().repeat(64))
}

fn run_cli(arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_agentic-vnext-rust"))
        .args(arguments)
        .output()
        .unwrap()
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "command failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
