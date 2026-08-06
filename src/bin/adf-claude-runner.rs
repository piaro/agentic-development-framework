use adf::claude_runner::{ClaudeRunnerOptions, run};
use std::env;
use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    let arguments = env::args().skip(1).collect::<Vec<_>>();
    if arguments
        .iter()
        .any(|item| matches!(item.as_str(), "-h" | "--help"))
    {
        print_usage();
        return ExitCode::SUCCESS;
    }
    match parse(&arguments).and_then(run) {
        Ok(report) => match serde_json::to_string_pretty(&report) {
            Ok(report) => {
                println!("{report}");
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("{error}");
                ExitCode::FAILURE
            }
        },
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

fn parse(arguments: &[String]) -> Result<ClaudeRunnerOptions, String> {
    if arguments.first().map(String::as_str) != Some("run") {
        return Err("the first argument must be run".to_owned());
    }
    let mut project_root = None;
    let mut change_id = None;
    let mut expected_action = None;
    let mut expected_context = None;
    let mut adf_binary = PathBuf::from("adf");
    let mut claude_binary = PathBuf::from("claude");
    let mut model = None;
    let mut index = 1;
    while index < arguments.len() {
        let flag = arguments[index].as_str();
        index += 1;
        let value = arguments
            .get(index)
            .ok_or_else(|| format!("{flag} requires a value"))?
            .clone();
        match flag {
            "--project" => project_root = Some(PathBuf::from(value)),
            "--change" => change_id = Some(value),
            "--expected-action" => expected_action = Some(value),
            "--expected-context" => expected_context = Some(value),
            "--adf-bin" => adf_binary = PathBuf::from(value),
            "--claude-bin" => claude_binary = PathBuf::from(value),
            "--model" => model = Some(value),
            other => return Err(format!("unknown argument: {other}")),
        }
        index += 1;
    }
    Ok(ClaudeRunnerOptions {
        project_root: project_root.ok_or_else(|| "--project is required".to_owned())?,
        change_id: change_id.ok_or_else(|| "--change is required".to_owned())?,
        expected_action: expected_action
            .ok_or_else(|| "--expected-action is required".to_owned())?,
        expected_context: expected_context
            .ok_or_else(|| "--expected-context is required".to_owned())?,
        adf_binary,
        claude_binary,
        model,
    })
}

fn print_usage() {
    println!(
        "usage: adf-claude-runner run --project <root> --change <id> --expected-action <id> --expected-context <digest> [--adf-bin <path>] [--claude-bin <path>] [--model <name>]"
    );
}
