use agentic_vnext_rust::binary_install::{
    BinaryInstallReceipt, binary_install_status, install_binary_candidate, rollback_binary_install,
};
use agentic_vnext_rust::cli_output::{next_response_value, render_next_text};
use agentic_vnext_rust::contract_health_gate::{ContractHealthGateReport, ContractHealthPolicy};
use agentic_vnext_rust::delivery::{
    install_release, read_framework_lock, rollback_framework_lock, switch_framework_lock,
};
use agentic_vnext_rust::detector_benchmark::run_detector_benchmark;
use agentic_vnext_rust::mcp_server::run_stdio_server;
use agentic_vnext_rust::project_runtime::LoadedProject;
use agentic_vnext_rust::project_setup::{
    ProjectInitOptions, default_candidate_root, initialize_change, initialize_project,
    observation_draft, write_observation_draft,
};
use agentic_vnext_rust::release_publisher::{
    PublishOptions, publish_release, signing_seed_from_environment,
};
use agentic_vnext_rust::remote_delivery::{fetch_release, install_release_archive};
use agentic_vnext_rust::signal_catalog::SignalCatalogRegistry;
use agentic_vnext_rust::{
    verify_application_suite, verify_canonicalization_suite, verify_context_suite,
    verify_detection_suite, verify_explain_suite, verify_filesystem_project_suite,
    verify_framework_lock_suite, verify_kernel_suite, verify_persistent_application_suite,
    verify_project_snapshot_suite, verify_result_submission_suite, verify_rule_compilation_suite,
    verify_schema_suite,
};
use std::env;
use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    let arguments: Vec<String> = env::args().collect();
    let Some(command) = arguments.get(1).map(String::as_str) else {
        usage();
        return ExitCode::from(2);
    };
    if matches!(command, "--help" | "-h" | "help") {
        print!("{}", usage_text());
        return ExitCode::SUCCESS;
    }
    if matches!(command, "--version" | "-V" | "version") {
        println!(
            "agentic {} (revision {})",
            env!("CARGO_PKG_VERSION"),
            option_env!("AGENTIC_BUILD_SOURCE_REVISION").unwrap_or("unknown")
        );
        return ExitCode::SUCCESS;
    }
    if command == "mcp" {
        let options = match parse_mcp_command(&arguments[2..]) {
            Ok(options) => options,
            Err(error) => {
                eprintln!("{error}");
                mcp_usage();
                return ExitCode::from(2);
            }
        };
        if let Err(error) = ensure_project_initialized(&options.project_root) {
            eprintln!("{error}");
            return ExitCode::FAILURE;
        }
        return match run_stdio_server(options.project_root, options.release) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("{error}");
                ExitCode::FAILURE
            }
        };
    }
    if command == "release" {
        let options = match parse_release_command(&arguments[2..]) {
            Ok(options) => options,
            Err(error) => {
                eprintln!("{error}");
                release_usage();
                return ExitCode::from(2);
            }
        };
        return match run_release_command(&options) {
            Ok(output) => {
                println!("{output}");
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("{error}");
                ExitCode::FAILURE
            }
        };
    }
    if command == "binary" {
        let options = match parse_binary_command(&arguments[2..]) {
            Ok(options) => options,
            Err(error) => {
                eprintln!("{error}");
                binary_usage();
                return ExitCode::from(2);
            }
        };
        return match run_binary_command(&options) {
            Ok(output) => {
                println!("{output}");
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("{error}");
                ExitCode::FAILURE
            }
        };
    }
    if command == "project" {
        let options = match parse_project_management_command(&arguments[2..]) {
            Ok(options) => options,
            Err(error) => {
                eprintln!("{error}");
                project_management_usage();
                return ExitCode::from(2);
            }
        };
        return match run_project_management_command(&options) {
            Ok(response) => {
                print!("{}", response.output);
                if response.success {
                    ExitCode::SUCCESS
                } else {
                    ExitCode::FAILURE
                }
            }
            Err(error) => {
                eprintln!("{error}");
                ExitCode::FAILURE
            }
        };
    }
    if command == "change" {
        let options = match parse_change_management_command(&arguments[2..]) {
            Ok(options) => options,
            Err(error) => {
                eprintln!("{error}");
                change_management_usage();
                return ExitCode::from(2);
            }
        };
        return match run_change_management_command(&options) {
            Ok(output) => {
                println!("{output}");
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("{error}");
                ExitCode::FAILURE
            }
        };
    }
    if matches!(command, "next" | "explain") {
        let options = match parse_project_command(command, &arguments[2..]) {
            Ok(options) => options,
            Err(error) => {
                eprintln!("{error}");
                project_usage(command);
                return ExitCode::from(2);
            }
        };
        return match run_project_command(command, &options) {
            Ok(output) => {
                print!("{output}");
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("{error}");
                ExitCode::FAILURE
            }
        };
    }
    if command == "contract-health" {
        let options = match parse_repository_command(command, &arguments[2..]) {
            Ok(options) => options,
            Err(error) => {
                eprintln!("{error}");
                repository_usage(command);
                return ExitCode::from(2);
            }
        };
        return match run_contract_health(&options) {
            Ok((output, passed)) => {
                print!("{output}");
                if passed {
                    ExitCode::SUCCESS
                } else {
                    ExitCode::FAILURE
                }
            }
            Err(error) => {
                eprintln!("{error}");
                ExitCode::FAILURE
            }
        };
    }
    if command == "benchmark" {
        let options = match parse_benchmark_command(&arguments[2..]) {
            Ok(options) => options,
            Err(error) => {
                eprintln!("{error}");
                benchmark_usage();
                return ExitCode::from(2);
            }
        };
        return match run_benchmark(&options) {
            Ok((output, passed)) => {
                print!("{output}");
                if passed {
                    ExitCode::SUCCESS
                } else {
                    ExitCode::FAILURE
                }
            }
            Err(error) => {
                eprintln!("{error}");
                ExitCode::FAILURE
            }
        };
    }
    if command == "catalog" {
        let options = match parse_catalog_command(&arguments[2..]) {
            Ok(options) => options,
            Err(error) => {
                eprintln!("{error}");
                catalog_usage();
                return ExitCode::from(2);
            }
        };
        return match run_catalog(&options) {
            Ok(output) => {
                print!("{output}");
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("{error}");
                ExitCode::FAILURE
            }
        };
    }

    if arguments.len() != 3 {
        usage();
        return ExitCode::from(2);
    }
    match run_verification(command, &arguments[2]) {
        Some(Ok(message)) => {
            println!("{message}");
            ExitCode::SUCCESS
        }
        Some(Err(error)) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
        None => {
            usage();
            ExitCode::from(2)
        }
    }
}

enum BinaryCommand {
    Install {
        candidate_root: PathBuf,
        release_tag: String,
        source_revision: String,
    },
    Status,
    Rollback,
}

struct BinaryOptions {
    install_root: PathBuf,
    format: OutputFormat,
    command: BinaryCommand,
}

fn parse_binary_command(arguments: &[String]) -> Result<BinaryOptions, String> {
    let operation = arguments
        .first()
        .ok_or_else(|| "binary requires an operation".to_owned())?;
    let mut index = 1;
    let candidate_root = if matches!(operation.as_str(), "install" | "update") {
        let value = arguments
            .get(index)
            .filter(|value| !value.starts_with('-'))
            .ok_or_else(|| format!("binary {operation} requires a candidate directory"))?;
        index += 1;
        Some(PathBuf::from(value))
    } else {
        None
    };
    let mut install_root = None;
    let mut release_tag = None;
    let mut source_revision = None;
    let mut format = OutputFormat::Text;
    while index < arguments.len() {
        match arguments[index].as_str() {
            "--install-root" => {
                install_root = Some(PathBuf::from(flag_value(
                    arguments,
                    &mut index,
                    "--install-root",
                )?));
            }
            "--tag" => {
                release_tag = Some(flag_value(arguments, &mut index, "--tag")?.to_owned());
            }
            "--source-revision" => {
                source_revision =
                    Some(flag_value(arguments, &mut index, "--source-revision")?.to_owned());
            }
            "--format" => {
                format = match flag_value(arguments, &mut index, "--format")? {
                    "text" => OutputFormat::Text,
                    "json" => OutputFormat::Json,
                    other => return Err(format!("unsupported output format: {other}")),
                };
            }
            other => return Err(format!("unknown binary argument: {other}")),
        }
        index += 1;
    }
    let install_root =
        install_root.ok_or_else(|| "binary requires --install-root <path>".to_owned())?;
    let command = match operation.as_str() {
        "install" | "update" => BinaryCommand::Install {
            candidate_root: candidate_root.expect("candidate was required above"),
            release_tag: release_tag
                .ok_or_else(|| format!("binary {operation} requires --tag <tag>"))?,
            source_revision: source_revision.ok_or_else(|| {
                format!("binary {operation} requires --source-revision <git-sha>")
            })?,
        },
        "status" => {
            reject_binary_install_options(&release_tag, &source_revision)?;
            BinaryCommand::Status
        }
        "rollback" => {
            reject_binary_install_options(&release_tag, &source_revision)?;
            BinaryCommand::Rollback
        }
        other => return Err(format!("unsupported binary operation: {other}")),
    };
    Ok(BinaryOptions {
        install_root,
        format,
        command,
    })
}

fn reject_binary_install_options(
    release_tag: &Option<String>,
    source_revision: &Option<String>,
) -> Result<(), String> {
    if release_tag.is_some() || source_revision.is_some() {
        Err("--tag and --source-revision apply only to binary install/update".to_owned())
    } else {
        Ok(())
    }
}

fn run_binary_command(options: &BinaryOptions) -> Result<String, String> {
    match &options.command {
        BinaryCommand::Install {
            candidate_root,
            release_tag,
            source_revision,
        } => {
            let receipt = install_binary_candidate(
                candidate_root,
                release_tag,
                source_revision,
                &options.install_root,
            )
            .map_err(|error| error.to_string())?;
            render_binary_receipt(&receipt, options.format)
        }
        BinaryCommand::Status => {
            let status =
                binary_install_status(&options.install_root).map_err(|error| error.to_string())?;
            match options.format {
                OutputFormat::Text => Ok(format!(
                    "target: {}\ncurrent: {}\nprevious: {}\nlauncher: {}",
                    status.target,
                    status.current.as_deref().unwrap_or("-"),
                    status.previous.as_deref().unwrap_or("-"),
                    status.launcher.display()
                )),
                OutputFormat::Json => {
                    serde_json::to_string_pretty(&status).map_err(|error| error.to_string())
                }
            }
        }
        BinaryCommand::Rollback => {
            let receipt = rollback_binary_install(&options.install_root)
                .map_err(|error| error.to_string())?;
            render_binary_receipt(&receipt, options.format)
        }
    }
}

fn render_binary_receipt(
    receipt: &BinaryInstallReceipt,
    format: OutputFormat,
) -> Result<String, String> {
    match format {
        OutputFormat::Text => Ok(format!(
            "binary Release {} active\ncurrent: {}\nprevious: {}\nlauncher: {}\nalready installed: {}",
            receipt.release_tag,
            receipt.current,
            receipt.previous.as_deref().unwrap_or("-"),
            receipt.launcher.display(),
            receipt.already_installed
        )),
        OutputFormat::Json => serde_json::to_string_pretty(&serde_json::json!({
            "schema_version": "1",
            "release_tag": receipt.release_tag,
            "current": receipt.current,
            "previous": receipt.previous,
            "already_installed": receipt.already_installed,
            "launcher": receipt.launcher,
        }))
        .map_err(|error| error.to_string()),
    }
}

enum ReleaseCommand {
    Build {
        source_root: PathBuf,
        base_lock: PathBuf,
        source_id: String,
        signer_key_id: String,
        expected_signer_public_key: Option<String>,
        rules_path: String,
        schemas_path: String,
        archive_output: PathBuf,
        lock_output: PathBuf,
        format: OutputFormat,
    },
    Fetch {
        candidate_lock: PathBuf,
    },
    Install {
        bundle: PathBuf,
        candidate_lock: PathBuf,
    },
    InstallArchive {
        archive: PathBuf,
        candidate_lock: PathBuf,
    },
    Switch {
        candidate_lock: PathBuf,
    },
    Rollback {
        backup_lock: PathBuf,
    },
}

struct ReleaseOptions {
    project_root: PathBuf,
    command: ReleaseCommand,
}

fn parse_release_command(arguments: &[String]) -> Result<ReleaseOptions, String> {
    let operation = arguments
        .first()
        .ok_or_else(|| "release requires an operation".to_owned())?;
    let target = arguments
        .get(1)
        .filter(|value| !value.starts_with('-'))
        .ok_or_else(|| format!("release {operation} requires a path"))?;
    let mut project_root = PathBuf::from(".");
    let mut lock = None;
    let mut source_id = None;
    let mut signer_key_id = None;
    let mut expected_signer_public_key = None;
    let mut archive_output = None;
    let mut lock_output = None;
    let mut rules_path = "rules.yaml".to_owned();
    let mut schemas_path = "schemas/v1".to_owned();
    let mut build_format = OutputFormat::Text;
    let mut index = 2;
    while index < arguments.len() {
        match arguments[index].as_str() {
            "--project" => {
                project_root = PathBuf::from(flag_value(arguments, &mut index, "--project")?);
            }
            "--lock" => {
                lock = Some(PathBuf::from(flag_value(arguments, &mut index, "--lock")?));
            }
            "--source-id" => {
                source_id = Some(flag_value(arguments, &mut index, "--source-id")?.to_owned());
            }
            "--key-id" => {
                signer_key_id = Some(flag_value(arguments, &mut index, "--key-id")?.to_owned());
            }
            "--expected-public-key" => {
                expected_signer_public_key =
                    Some(flag_value(arguments, &mut index, "--expected-public-key")?.to_owned());
            }
            "--output" => {
                archive_output = Some(PathBuf::from(flag_value(
                    arguments, &mut index, "--output",
                )?));
            }
            "--lock-output" => {
                lock_output = Some(PathBuf::from(flag_value(
                    arguments,
                    &mut index,
                    "--lock-output",
                )?));
            }
            "--rules" => {
                rules_path = flag_value(arguments, &mut index, "--rules")?.to_owned();
            }
            "--schemas" => {
                schemas_path = flag_value(arguments, &mut index, "--schemas")?.to_owned();
            }
            "--format" => {
                build_format = match flag_value(arguments, &mut index, "--format")? {
                    "text" => OutputFormat::Text,
                    "json" => OutputFormat::Json,
                    other => return Err(format!("unsupported output format: {other}")),
                };
            }
            other => return Err(format!("unknown release argument: {other}")),
        }
        index += 1;
    }
    let command = match operation.as_str() {
        "build" => ReleaseCommand::Build {
            source_root: PathBuf::from(target),
            base_lock: lock
                .ok_or_else(|| "release build requires --lock <base-lock>".to_owned())?,
            source_id: source_id
                .ok_or_else(|| "release build requires --source-id <id>".to_owned())?,
            signer_key_id: signer_key_id
                .ok_or_else(|| "release build requires --key-id <id>".to_owned())?,
            expected_signer_public_key,
            rules_path,
            schemas_path,
            archive_output: archive_output
                .ok_or_else(|| "release build requires --output <archive>".to_owned())?,
            lock_output: lock_output
                .ok_or_else(|| "release build requires --lock-output <lock>".to_owned())?,
            format: build_format,
        },
        "fetch" => {
            reject_build_only_options(
                (&source_id, &signer_key_id, &expected_signer_public_key),
                (&archive_output, &lock_output),
                &rules_path,
                &schemas_path,
                build_format,
            )?;
            if lock.is_some() {
                return Err("release fetch does not accept --lock".to_owned());
            }
            ReleaseCommand::Fetch {
                candidate_lock: PathBuf::from(target),
            }
        }
        "install" => {
            reject_build_only_options(
                (&source_id, &signer_key_id, &expected_signer_public_key),
                (&archive_output, &lock_output),
                &rules_path,
                &schemas_path,
                build_format,
            )?;
            ReleaseCommand::Install {
                bundle: PathBuf::from(target),
                candidate_lock: lock
                    .ok_or_else(|| "release install requires --lock <candidate-lock>".to_owned())?,
            }
        }
        "install-archive" => {
            reject_build_only_options(
                (&source_id, &signer_key_id, &expected_signer_public_key),
                (&archive_output, &lock_output),
                &rules_path,
                &schemas_path,
                build_format,
            )?;
            ReleaseCommand::InstallArchive {
                archive: PathBuf::from(target),
                candidate_lock: lock.ok_or_else(|| {
                    "release install-archive requires --lock <candidate-lock>".to_owned()
                })?,
            }
        }
        "switch" => {
            reject_build_only_options(
                (&source_id, &signer_key_id, &expected_signer_public_key),
                (&archive_output, &lock_output),
                &rules_path,
                &schemas_path,
                build_format,
            )?;
            if lock.is_some() {
                return Err("release switch does not accept --lock".to_owned());
            }
            ReleaseCommand::Switch {
                candidate_lock: PathBuf::from(target),
            }
        }
        "rollback" => {
            reject_build_only_options(
                (&source_id, &signer_key_id, &expected_signer_public_key),
                (&archive_output, &lock_output),
                &rules_path,
                &schemas_path,
                build_format,
            )?;
            if lock.is_some() {
                return Err("release rollback does not accept --lock".to_owned());
            }
            ReleaseCommand::Rollback {
                backup_lock: PathBuf::from(target),
            }
        }
        other => return Err(format!("unsupported release operation: {other}")),
    };
    Ok(ReleaseOptions {
        project_root,
        command,
    })
}

fn reject_build_only_options(
    identity: (&Option<String>, &Option<String>, &Option<String>),
    outputs: (&Option<PathBuf>, &Option<PathBuf>),
    rules_path: &str,
    schemas_path: &str,
    format: OutputFormat,
) -> Result<(), String> {
    let (source_id, signer_key_id, expected_signer_public_key) = identity;
    let (archive_output, lock_output) = outputs;
    if source_id.is_some()
        || signer_key_id.is_some()
        || expected_signer_public_key.is_some()
        || archive_output.is_some()
        || lock_output.is_some()
        || rules_path != "rules.yaml"
        || schemas_path != "schemas/v1"
        || format != OutputFormat::Text
    {
        Err("publisher options are accepted only by release build".to_owned())
    } else {
        Ok(())
    }
}

fn run_release_command(options: &ReleaseOptions) -> Result<String, String> {
    match &options.command {
        ReleaseCommand::Build {
            source_root,
            base_lock,
            source_id,
            signer_key_id,
            expected_signer_public_key,
            rules_path,
            schemas_path,
            archive_output,
            lock_output,
            format,
        } => {
            let source_root = project_path(&options.project_root, source_root);
            let base_lock = project_path(&options.project_root, base_lock);
            let archive_output = project_path(&options.project_root, archive_output);
            let lock_output = project_path(&options.project_root, lock_output);
            let mut signing_seed =
                signing_seed_from_environment().map_err(|error| error.to_string())?;
            let published = publish_release(
                &PublishOptions {
                    source_root: &source_root,
                    base_framework_lock: &base_lock,
                    source_id,
                    signer_key_id,
                    expected_signer_public_key: expected_signer_public_key.as_deref(),
                    rules_path,
                    schemas_path,
                    archive_output: &archive_output,
                    lock_output: &lock_output,
                },
                signing_seed,
            );
            signing_seed.fill(0);
            let receipt = published.map_err(|error| error.to_string())?;
            match format {
                OutputFormat::Text => Ok(format!(
                    "Framework Release {} published\narchive: {}\nlock: {}\nartifact digest: {}\narchive digest: {}\nsigner public key: {}",
                    receipt.release_id,
                    receipt.archive_output.display(),
                    receipt.lock_output.display(),
                    receipt.artifact_digest,
                    receipt.archive_digest,
                    receipt.signer_public_key,
                )),
                OutputFormat::Json => serde_json::to_string_pretty(&serde_json::json!({
                    "schema_version": "1",
                    "release_id": receipt.release_id,
                    "artifact_digest": receipt.artifact_digest,
                    "archive_digest": receipt.archive_digest,
                    "signer_public_key": receipt.signer_public_key,
                    "outputs": {
                        "archive": receipt.archive_output.to_string_lossy(),
                        "framework_lock": receipt.lock_output.to_string_lossy(),
                    },
                }))
                .map_err(|error| error.to_string()),
            }
        }
        ReleaseCommand::Fetch { candidate_lock } => {
            let candidate_path = project_path(&options.project_root, candidate_lock);
            let lock = read_framework_lock(&candidate_path).map_err(|error| error.to_string())?;
            let receipt =
                fetch_release(&options.project_root, &lock).map_err(|error| error.to_string())?;
            let status = if receipt.install.already_installed {
                "already installed"
            } else {
                "downloaded and installed"
            };
            Ok(format!(
                "Framework Release {} {status} from {}",
                receipt.install.release_id, receipt.artifact_url
            ))
        }
        ReleaseCommand::Install {
            bundle,
            candidate_lock,
        } => {
            let candidate_path = project_path(&options.project_root, candidate_lock);
            let lock = read_framework_lock(&candidate_path).map_err(|error| error.to_string())?;
            let receipt = install_release(&options.project_root, &lock, bundle)
                .map_err(|error| error.to_string())?;
            let status = if receipt.already_installed {
                "already installed"
            } else {
                "installed"
            };
            Ok(format!(
                "Framework Release {} {status} at {}",
                receipt.release_id,
                receipt.installed_root.display()
            ))
        }
        ReleaseCommand::InstallArchive {
            archive,
            candidate_lock,
        } => {
            let candidate_path = project_path(&options.project_root, candidate_lock);
            let archive_path = project_path(&options.project_root, archive);
            let lock = read_framework_lock(&candidate_path).map_err(|error| error.to_string())?;
            let receipt = install_release_archive(&options.project_root, &lock, &archive_path)
                .map_err(|error| error.to_string())?;
            let status = if receipt.already_installed {
                "already installed"
            } else {
                "installed"
            };
            Ok(format!(
                "Framework Release archive {} {status} at {}",
                receipt.release_id,
                receipt.installed_root.display()
            ))
        }
        ReleaseCommand::Switch { candidate_lock } => {
            let receipt = switch_framework_lock(&options.project_root, candidate_lock)
                .map_err(|error| error.to_string())?;
            Ok(format!(
                "Framework Release {} activated; rollback lock: {}",
                receipt.release_id,
                receipt.backup_path.display()
            ))
        }
        ReleaseCommand::Rollback { backup_lock } => {
            let receipt = rollback_framework_lock(&options.project_root, backup_lock)
                .map_err(|error| error.to_string())?;
            Ok(format!(
                "Framework Release {} restored; previous lock backed up at {}",
                receipt.release_id,
                receipt.backup_path.display()
            ))
        }
    }
}

fn project_path(project_root: &std::path::Path, path: &std::path::Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        project_root.join(path)
    }
}

fn run_verification(command: &str, golden_root: &str) -> Option<Result<String, String>> {
    let result = match command {
        "verify-canonicalization" => verify_canonicalization_suite(golden_root).map(|report| {
            format!(
                "canonicalization golden passed: {} valid, {} invalid",
                report.valid_cases, report.invalid_cases
            )
        }),
        "verify-schema" => verify_schema_suite(golden_root).map(|report| {
            format!(
                "Schema golden passed: {} valid, {} invalid",
                report.valid_cases, report.invalid_cases
            )
        }),
        "verify-rules" => verify_rule_compilation_suite(golden_root).map(|report| {
            format!(
                "Rule Compiler golden passed: {} valid variants, {} invalid",
                report.valid_variants, report.invalid_cases
            )
        }),
        "verify-detection" => verify_detection_suite(golden_root).map(|report| {
            format!(
                "Detector golden passed: {} valid variants, {} invalid",
                report.valid_variants, report.invalid_cases
            )
        }),
        "verify-kernel" => verify_kernel_suite(golden_root)
            .map(|report| format!("Thin Kernel golden passed: {} cases", report.cases)),
        "verify-context" => verify_context_suite(golden_root)
            .map(|report| format!("Context Compiler golden passed: {} cases", report.cases)),
        "verify-project" => verify_project_snapshot_suite(golden_root).map(|report| {
            format!(
                "Project Snapshot golden passed: {} valid, {} invalid",
                report.valid_cases, report.invalid_cases
            )
        }),
        "verify-lock" => verify_framework_lock_suite(golden_root).map(|report| {
            format!(
                "Framework lock golden passed: {} valid, {} invalid",
                report.valid_cases, report.invalid_cases
            )
        }),
        "verify-submission" => verify_result_submission_suite(golden_root).map(|report| {
            format!(
                "Result submission golden passed: {} valid, {} invalid",
                report.valid_cases, report.invalid_cases
            )
        }),
        "verify-application" => verify_application_suite(golden_root).map(|report| {
            format!(
                "Application golden passed: {} initial, {} lifecycle steps",
                report.initial_cases, report.lifecycle_steps
            )
        }),
        "verify-store" => verify_filesystem_project_suite(golden_root).map(|report| {
            format!(
                "Filesystem Project golden passed: {} formats, {} invalid roots",
                report.formats, report.invalid_source_roots
            )
        }),
        "verify-persistent" => verify_persistent_application_suite(golden_root).map(|report| {
            format!(
                "Persistent Application golden passed: {} formats, {} restart checkpoints",
                report.formats, report.restart_checkpoints
            )
        }),
        "verify-explain" => verify_explain_suite(golden_root).map(|report| {
            format!(
                "Explain Report golden passed: {} checkpoints",
                report.checkpoints
            )
        }),
        _ => return None,
    };
    Some(result.map_err(|error| error.to_string()))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OutputFormat {
    Text,
    Json,
}

struct ProjectCommand {
    change_id: String,
    project_root: PathBuf,
    release: Option<PathBuf>,
    format: OutputFormat,
    require_clean: bool,
}

struct RepositoryCommand {
    project_root: PathBuf,
    release: Option<PathBuf>,
    policy: Option<String>,
    format: OutputFormat,
    require_clean: bool,
}

struct BenchmarkCommand {
    corpus_root: PathBuf,
    format: OutputFormat,
}

struct CatalogCommand {
    format: OutputFormat,
}

struct McpCommand {
    project_root: PathBuf,
    release: Option<PathBuf>,
}

enum ProjectManagementOperation {
    Init {
        candidate_root: Option<PathBuf>,
        analysis_roots: Vec<String>,
    },
    Observe {
        analysis_roots: Vec<String>,
        format: ProjectDraftFormat,
        output: Option<String>,
    },
    ValidateBindings {
        format: ProjectDraftFormat,
        require_clean: bool,
    },
}

struct ProjectManagementCommand {
    project_root: PathBuf,
    operation: ProjectManagementOperation,
}

struct ProjectManagementResponse {
    output: String,
    success: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProjectDraftFormat {
    Text,
    Yaml,
    Json,
}

struct ChangeManagementCommand {
    project_root: PathBuf,
    change_id: String,
    title: String,
    intent: String,
}

fn parse_project_management_command(
    arguments: &[String],
) -> Result<ProjectManagementCommand, String> {
    let operation = arguments
        .first()
        .ok_or_else(|| "project requires an operation".to_owned())?;
    let mut project_root = PathBuf::from(".");
    let mut candidate_root = None;
    let mut analysis_roots = Vec::new();
    let mut format = ProjectDraftFormat::Yaml;
    let mut format_provided = false;
    let mut require_clean = false;
    let mut output = None;
    let mut index = 1;
    while index < arguments.len() {
        match arguments[index].as_str() {
            "--project" => {
                project_root = PathBuf::from(flag_value(arguments, &mut index, "--project")?);
            }
            "--candidate-dir" => {
                candidate_root = Some(PathBuf::from(flag_value(
                    arguments,
                    &mut index,
                    "--candidate-dir",
                )?));
            }
            "--analysis-root" => {
                analysis_roots
                    .push(flag_value(arguments, &mut index, "--analysis-root")?.to_owned());
            }
            "--format" => {
                format_provided = true;
                format = match flag_value(arguments, &mut index, "--format")? {
                    "text" => ProjectDraftFormat::Text,
                    "yaml" => ProjectDraftFormat::Yaml,
                    "json" => ProjectDraftFormat::Json,
                    other => return Err(format!("unsupported project output format: {other}")),
                };
            }
            "--require-clean" => {
                require_clean = true;
            }
            "--output" => {
                output = Some(flag_value(arguments, &mut index, "--output")?.to_owned());
            }
            other => return Err(format!("unknown project argument: {other}")),
        }
        index += 1;
    }
    let operation = match operation.as_str() {
        "init" => {
            if format_provided {
                return Err("project init does not accept --format".to_owned());
            }
            if require_clean {
                return Err("project init does not accept --require-clean".to_owned());
            }
            if output.is_some() {
                return Err("project init does not accept --output".to_owned());
            }
            ProjectManagementOperation::Init {
                candidate_root,
                analysis_roots,
            }
        }
        "observe" => {
            if candidate_root.is_some() {
                return Err("project observe does not accept --candidate-dir".to_owned());
            }
            if require_clean {
                return Err("project observe does not accept --require-clean".to_owned());
            }
            if format == ProjectDraftFormat::Text {
                return Err("project observe supports only yaml or json output".to_owned());
            }
            ProjectManagementOperation::Observe {
                analysis_roots,
                format,
                output,
            }
        }
        "validate-bindings" => {
            if candidate_root.is_some() {
                return Err("project validate-bindings does not accept --candidate-dir".to_owned());
            }
            if !analysis_roots.is_empty() {
                return Err("project validate-bindings does not accept --analysis-root".to_owned());
            }
            if output.is_some() {
                return Err("project validate-bindings does not accept --output".to_owned());
            }
            if !format_provided {
                format = ProjectDraftFormat::Text;
            }
            if format == ProjectDraftFormat::Yaml {
                return Err(
                    "project validate-bindings supports only text or json output".to_owned(),
                );
            }
            ProjectManagementOperation::ValidateBindings {
                format,
                require_clean,
            }
        }
        other => return Err(format!("unsupported project operation: {other}")),
    };
    Ok(ProjectManagementCommand {
        project_root,
        operation,
    })
}

fn run_project_management_command(
    options: &ProjectManagementCommand,
) -> Result<ProjectManagementResponse, String> {
    match &options.operation {
        ProjectManagementOperation::Init {
            candidate_root,
            analysis_roots,
        } => {
            let candidate_root = candidate_root
                .clone()
                .map(Ok)
                .unwrap_or_else(default_candidate_root)
                .map_err(|error| error.to_string())?;
            let receipt = initialize_project(&ProjectInitOptions {
                project_root: &options.project_root,
                candidate_root: &candidate_root,
                analysis_roots,
            })
            .map_err(|error| error.to_string())?;
            let files = receipt
                .created_files
                .iter()
                .map(|path| {
                    path.strip_prefix(&receipt.project_root)
                        .unwrap_or(path)
                        .display()
                        .to_string()
                })
                .collect::<Vec<_>>()
                .join(", ");
            Ok(ProjectManagementResponse {
                output: format!(
                    "Project initialized with Framework Release {}.\nCreated: {}\nNext: review the generated files, then git add and commit them.\nNext: if source code already exists, run agentic project observe --output .agentic/repository-observation.draft.yaml --project {} and complete the reviewed bindings and accepted Decisions.\nNext: agentic project validate-bindings --project {}\nNext: agentic change init <change-id> --title <title> --intent <intent> --project {}\n",
                    receipt.release_id,
                    files,
                    receipt.project_root.display(),
                    receipt.project_root.display(),
                    receipt.project_root.display()
                ),
                success: true,
            })
        }
        ProjectManagementOperation::Observe {
            analysis_roots,
            format,
            output,
        } => {
            let value = observation_draft(&options.project_root, analysis_roots)
                .map_err(|error| error.to_string())?;
            let serialized = match format {
                ProjectDraftFormat::Yaml => {
                    serde_yaml::to_string(&value).map_err(|error| error.to_string())
                }
                ProjectDraftFormat::Json => serde_json::to_string_pretty(&value)
                    .map(|value| value + "\n")
                    .map_err(|error| error.to_string()),
                ProjectDraftFormat::Text => unreachable!("observe rejects text output"),
            }?;
            let output = if let Some(relative) = output {
                let path =
                    write_observation_draft(&options.project_root, relative, serialized.as_bytes())
                        .map_err(|error| error.to_string())?;
                format!(
                    "Observation draft written to: {}\nNext: review binding_artifacts and complete their logical refs, owners, fact kinds, and authority refs.\nThen: merge only reviewed binding_artifacts into the configured Repository Observation and record the accepted Decisions.\nThen: agentic project validate-bindings --project {}\nOnly the requested draft file was created. Binding candidates were not applied automatically.\n",
                    path.display(),
                    options.project_root.display(),
                )
            } else {
                serialized
            };
            Ok(ProjectManagementResponse {
                output,
                success: true,
            })
        }
        ProjectManagementOperation::ValidateBindings {
            format,
            require_clean,
        } => {
            ensure_project_initialized(&options.project_root)?;
            let project = LoadedProject::load(&options.project_root, None, *require_clean)
                .map_err(|error| error.to_string())?;
            let report = project
                .binding_validation_report()
                .map_err(|error| error.to_string())?;
            let output = match format {
                ProjectDraftFormat::Text => report.render_text(),
                ProjectDraftFormat::Json => serde_json::to_string_pretty(&report.as_value())
                    .map(|value| value + "\n")
                    .map_err(|error| error.to_string())?,
                ProjectDraftFormat::Yaml => {
                    unreachable!("validate-bindings rejects yaml output")
                }
            };
            Ok(ProjectManagementResponse {
                output,
                success: report.is_valid(),
            })
        }
    }
}

fn parse_change_management_command(
    arguments: &[String],
) -> Result<ChangeManagementCommand, String> {
    if arguments.first().map(String::as_str) != Some("init") {
        return Err("change requires the init operation".to_owned());
    }
    let change_id = arguments
        .get(1)
        .filter(|value| !value.starts_with('-'))
        .ok_or_else(|| "change init requires a Change ID".to_owned())?
        .clone();
    let mut project_root = PathBuf::from(".");
    let mut title = None;
    let mut intent = None;
    let mut index = 2;
    while index < arguments.len() {
        match arguments[index].as_str() {
            "--project" => {
                project_root = PathBuf::from(flag_value(arguments, &mut index, "--project")?);
            }
            "--title" => {
                title = Some(flag_value(arguments, &mut index, "--title")?.to_owned());
            }
            "--intent" => {
                intent = Some(flag_value(arguments, &mut index, "--intent")?.to_owned());
            }
            other => return Err(format!("unknown change init argument: {other}")),
        }
        index += 1;
    }
    Ok(ChangeManagementCommand {
        project_root,
        change_id,
        title: title.ok_or_else(|| "change init requires --title <title>".to_owned())?,
        intent: intent.ok_or_else(|| "change init requires --intent <intent>".to_owned())?,
    })
}

fn run_change_management_command(options: &ChangeManagementCommand) -> Result<String, String> {
    let path = initialize_change(
        &options.project_root,
        &options.change_id,
        &options.title,
        &options.intent,
    )
    .map_err(|error| error.to_string())?;
    Ok(format!(
        "Change {} initialized at {}\nNext: review, git add, and commit the Change.\nNext: agentic next {} --project {}",
        options.change_id,
        path.display(),
        options.change_id,
        options.project_root.display()
    ))
}

fn parse_mcp_command(arguments: &[String]) -> Result<McpCommand, String> {
    let mut project_root = PathBuf::from(".");
    let mut release = None;
    let mut index = 0;
    while index < arguments.len() {
        match arguments[index].as_str() {
            "--project" => {
                project_root = PathBuf::from(flag_value(arguments, &mut index, "--project")?);
            }
            "--release" => {
                release = Some(PathBuf::from(flag_value(
                    arguments,
                    &mut index,
                    "--release",
                )?));
            }
            other => return Err(format!("unknown mcp argument: {other}")),
        }
        index += 1;
    }
    Ok(McpCommand {
        project_root,
        release,
    })
}

fn parse_project_command(command: &str, arguments: &[String]) -> Result<ProjectCommand, String> {
    let change_id = arguments
        .first()
        .filter(|value| !value.starts_with('-'))
        .ok_or_else(|| format!("{command} requires a Change ID"))?
        .clone();
    let mut project_root = PathBuf::from(".");
    let mut release = None;
    let mut format = OutputFormat::Text;
    let mut require_clean = false;
    let mut index = 1;
    while index < arguments.len() {
        match arguments[index].as_str() {
            "--project" => {
                project_root = PathBuf::from(flag_value(arguments, &mut index, "--project")?);
            }
            "--release" => {
                release = Some(PathBuf::from(flag_value(
                    arguments,
                    &mut index,
                    "--release",
                )?));
            }
            "--format" => {
                format = match flag_value(arguments, &mut index, "--format")? {
                    "text" => OutputFormat::Text,
                    "json" => OutputFormat::Json,
                    other => return Err(format!("unsupported output format: {other}")),
                };
            }
            "--require-clean" => require_clean = true,
            other => return Err(format!("unknown {command} argument: {other}")),
        }
        index += 1;
    }
    Ok(ProjectCommand {
        change_id,
        project_root,
        release,
        format,
        require_clean,
    })
}

fn flag_value<'a>(
    arguments: &'a [String],
    index: &mut usize,
    flag: &str,
) -> Result<&'a str, String> {
    *index += 1;
    arguments
        .get(*index)
        .map(String::as_str)
        .ok_or_else(|| format!("{flag} requires a value"))
}

fn parse_repository_command(
    command: &str,
    arguments: &[String],
) -> Result<RepositoryCommand, String> {
    let mut project_root = PathBuf::from(".");
    let mut release = None;
    let mut policy = None;
    let mut format = OutputFormat::Text;
    let mut require_clean = false;
    let mut index = 0;
    while index < arguments.len() {
        match arguments[index].as_str() {
            "--project" => {
                project_root = PathBuf::from(flag_value(arguments, &mut index, "--project")?);
            }
            "--release" => {
                release = Some(PathBuf::from(flag_value(
                    arguments,
                    &mut index,
                    "--release",
                )?));
            }
            "--policy" => {
                policy = Some(flag_value(arguments, &mut index, "--policy")?.to_owned());
            }
            "--format" => {
                format = match flag_value(arguments, &mut index, "--format")? {
                    "text" => OutputFormat::Text,
                    "json" => OutputFormat::Json,
                    other => return Err(format!("unsupported output format: {other}")),
                };
            }
            "--require-clean" => require_clean = true,
            other => return Err(format!("unknown {command} argument: {other}")),
        }
        index += 1;
    }
    Ok(RepositoryCommand {
        project_root,
        release,
        policy,
        format,
        require_clean,
    })
}

fn parse_benchmark_command(arguments: &[String]) -> Result<BenchmarkCommand, String> {
    let corpus_root = arguments
        .first()
        .filter(|value| !value.starts_with('-'))
        .ok_or_else(|| "benchmark requires a corpus root".to_owned())
        .map(PathBuf::from)?;
    let mut format = OutputFormat::Text;
    let mut index = 1;
    while index < arguments.len() {
        match arguments[index].as_str() {
            "--format" => {
                format = match flag_value(arguments, &mut index, "--format")? {
                    "text" => OutputFormat::Text,
                    "json" => OutputFormat::Json,
                    other => return Err(format!("unsupported output format: {other}")),
                };
            }
            other => return Err(format!("unknown benchmark argument: {other}")),
        }
        index += 1;
    }
    Ok(BenchmarkCommand {
        corpus_root,
        format,
    })
}

fn parse_catalog_command(arguments: &[String]) -> Result<CatalogCommand, String> {
    if arguments.first().map(String::as_str) != Some("signal-domains") {
        return Err("catalog requires the signal-domains operation".to_owned());
    }
    let mut format = OutputFormat::Text;
    let mut index = 1;
    while index < arguments.len() {
        match arguments[index].as_str() {
            "--format" => {
                format = match flag_value(arguments, &mut index, "--format")? {
                    "text" => OutputFormat::Text,
                    "json" => OutputFormat::Json,
                    other => return Err(format!("unsupported output format: {other}")),
                };
            }
            other => return Err(format!("unknown catalog argument: {other}")),
        }
        index += 1;
    }
    Ok(CatalogCommand { format })
}

fn run_project_command(command: &str, options: &ProjectCommand) -> Result<String, String> {
    ensure_project_initialized(&options.project_root)?;
    let project = LoadedProject::load(
        &options.project_root,
        options.release.as_deref(),
        options.require_clean,
    )
    .map_err(|error| error.to_string())?;
    if options.require_clean {
        project
            .assert_tracked_inputs(&options.change_id)
            .map_err(|error| error.to_string())?;
    }
    let mut application = project.application().map_err(|error| error.to_string())?;
    match command {
        "next" => {
            let response = application.next(&options.change_id).map_err(|error| {
                actionable_project_error(&error.to_string(), &options.change_id)
            })?;
            match options.format {
                OutputFormat::Text => Ok(render_next_text(&options.change_id, &response)),
                OutputFormat::Json => serde_json::to_string_pretty(&next_response_value(
                    &options.change_id,
                    &response,
                ))
                .map(|value| value + "\n")
                .map_err(|error| error.to_string()),
            }
        }
        "explain" => {
            let report = application.explain(&options.change_id).map_err(|error| {
                actionable_project_error(&error.to_string(), &options.change_id)
            })?;
            match options.format {
                OutputFormat::Text => Ok(report.render_text()),
                OutputFormat::Json => serde_json::to_string_pretty(&report.as_value())
                    .map(|value| value + "\n")
                    .map_err(|error| error.to_string()),
            }
        }
        _ => unreachable!("project commands are checked before dispatch"),
    }
}

fn run_contract_health(options: &RepositoryCommand) -> Result<(String, bool), String> {
    ensure_project_initialized(&options.project_root)?;
    let project = LoadedProject::load(
        &options.project_root,
        options.release.as_deref(),
        options.require_clean,
    )
    .map_err(|error| error.to_string())?;
    if options.require_clean {
        project
            .assert_tracked_project_inputs()
            .map_err(|error| error.to_string())?;
    }
    let policy = options
        .policy
        .as_deref()
        .map(|relative| {
            let path = project
                .repository_path(relative)
                .map_err(|error| error.to_string())?;
            if options.require_clean {
                project
                    .assert_tracked_paths(std::slice::from_ref(&path))
                    .map_err(|error| error.to_string())?;
            }
            ContractHealthPolicy::load(&path).map_err(|error| error.to_string())
        })
        .transpose()?;
    let application = project.application().map_err(|error| error.to_string())?;
    let report = application
        .contract_health()
        .map_err(|error| error.to_string())?;
    if let (Some(policy_path), Some(policy)) = (options.policy.as_deref(), policy.as_ref()) {
        let gate = ContractHealthGateReport::build(policy_path, policy, report);
        let passed = gate.passed();
        let output = match options.format {
            OutputFormat::Text => gate.render_text(),
            OutputFormat::Json => serde_json::to_string_pretty(&gate.as_value())
                .map(|value| value + "\n")
                .map_err(|error| error.to_string())?,
        };
        Ok((output, passed))
    } else {
        let output = match options.format {
            OutputFormat::Text => report.render_text(),
            OutputFormat::Json => serde_json::to_string_pretty(&report.as_value())
                .map(|value| value + "\n")
                .map_err(|error| error.to_string())?,
        };
        Ok((output, true))
    }
}

fn run_benchmark(options: &BenchmarkCommand) -> Result<(String, bool), String> {
    let report = run_detector_benchmark(&options.corpus_root).map_err(|error| error.to_string())?;
    let passed = report.passed();
    let output = match options.format {
        OutputFormat::Text => report.render_text(),
        OutputFormat::Json => serde_json::to_string_pretty(&report.as_value())
            .map(|value| value + "\n")
            .map_err(|error| error.to_string())?,
    };
    Ok((output, passed))
}

fn run_catalog(options: &CatalogCommand) -> Result<String, String> {
    let catalog = SignalCatalogRegistry::built_in()
        .map_err(|error| error.to_string())?
        .catalog();
    match options.format {
        OutputFormat::Text => Ok(catalog.render_text()),
        OutputFormat::Json => serde_json::to_string_pretty(&catalog.as_value())
            .map(|value| value + "\n")
            .map_err(|error| error.to_string()),
    }
}

fn ensure_project_initialized(project_root: &std::path::Path) -> Result<(), String> {
    let config = project_root.join(".agentic/config.yaml");
    if config.is_file() {
        Ok(())
    } else {
        Err(format!(
            "Project is not initialized: {} is missing.\nNext: agentic project init --project {}",
            config.display(),
            project_root.display()
        ))
    }
}

fn actionable_project_error(error: &str, change_id: &str) -> String {
    if error.contains("unknown change:") {
        format!("{error}\nNext: agentic change init {change_id} --title <title> --intent <intent>")
    } else {
        error.to_owned()
    }
}

fn usage() {
    eprint!("{}", usage_text());
}

fn usage_text() -> &'static str {
    "Agentic Development Kit\n\
\n\
usage:\n\
  agentic project init [--project <root>] [--candidate-dir <dir>] [--analysis-root <path>]...\n\
  agentic project observe [--project <root>] [--analysis-root <path>]... [--format <yaml|json>] [--output <path>]\n\
  agentic project validate-bindings [--project <root>] [--format <text|json>] [--require-clean]\n\
  agentic change init <change-id> --title <title> --intent <intent> [--project <root>]\n\
  agentic <next|explain> <change-id> [--project <root>] [--release <root>] [--format <text|json>] [--require-clean]\n\
  agentic contract-health [--project <root>] [--release <root>] [--policy <path>] [--format <text|json>] [--require-clean]\n\
  agentic benchmark <corpus-root> [--format <text|json>]\n\
  agentic catalog signal-domains [--format <text|json>]\n\
  agentic mcp [--project <root>] [--release <root>]\n\
  agentic release <build|fetch|install|install-archive|switch|rollback> ...\n\
  agentic binary <install|update|status|rollback> ...\n\
  agentic --help\n\
  agentic --version\n"
}

fn mcp_usage() {
    eprintln!(
        "usage:\n  agentic mcp \
         [--project <root>] [--release <root>]"
    );
}

fn binary_usage() {
    eprintln!(
        "usage:\n  agentic binary <install|update> <candidate-directory> \
         --tag <release-tag> --source-revision <git-sha> \
         --install-root <path> [--format <text|json>]\n  \
         agentic binary <status|rollback> \
         --install-root <path> [--format <text|json>]"
    );
}

fn release_usage() {
    eprintln!(
        "usage:\n  AGENTIC_RELEASE_SIGNING_KEY_HEX=<64-hex-seed> \
         agentic release build <source-root> \
         --lock <base-lock> --source-id <id> --key-id <id> \
         [--expected-public-key <64-hex-public-key>] \
         --output <archive> --lock-output <candidate-lock> \
         [--rules <path>] [--schemas <path>] [--format <text|json>] \
         [--project <root>]\n  \
         agentic release fetch <candidate-lock> \
         [--project <root>]\n  \
         agentic release install <bundle-root> \
         --lock <candidate-lock> [--project <root>]\n  \
         agentic release install-archive <archive> \
         --lock <candidate-lock> [--project <root>]\n  \
         agentic release switch <candidate-lock> [--project <root>]\n  \
         agentic release rollback <backup-lock> [--project <root>]"
    );
}

fn project_usage(command: &str) {
    eprintln!(
        "usage: agentic {command} <change-id> \
         [--project <root>] [--release <root>] \
         [--format <text|json>] [--require-clean]"
    );
}

fn repository_usage(command: &str) {
    eprintln!(
        "usage: agentic {command} \
         [--project <root>] [--release <root>] \
         [--policy <path>] [--format <text|json>] [--require-clean]"
    );
}

fn benchmark_usage() {
    eprintln!("usage: agentic benchmark <corpus-root> [--format <text|json>]");
}

fn catalog_usage() {
    eprintln!("usage: agentic catalog signal-domains [--format <text|json>]");
}

fn project_management_usage() {
    eprintln!(
        "usage:\n  agentic project init [--project <root>] [--candidate-dir <dir>] [--analysis-root <path>]...\n  agentic project observe [--project <root>] [--analysis-root <path>]... [--format <yaml|json>] [--output <path>]\n  agentic project validate-bindings [--project <root>] [--format <text|json>] [--require-clean]"
    );
}

fn change_management_usage() {
    eprintln!(
        "usage: agentic change init <change-id> --title <title> --intent <intent> [--project <root>]"
    );
}
