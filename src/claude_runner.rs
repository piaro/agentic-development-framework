//! Optional one-Action adapter between an ADF project and Claude Code print mode.
//!
//! The runner stays outside the ADF control plane. It starts only when another
//! process invokes it and accepts Challenger Actions only, matching the safety
//! boundary of the Codex runner.

use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

#[derive(Debug, Clone)]
pub struct ClaudeRunnerOptions {
    pub project_root: PathBuf,
    pub change_id: String,
    pub expected_action: String,
    pub expected_context: String,
    pub adf_binary: PathBuf,
    pub claude_binary: PathBuf,
    pub model: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ClaudeRunnerReport {
    pub change_id: String,
    pub action_id: String,
    pub context_digest: String,
    pub execution_id: String,
    pub result_id: String,
    pub summary: String,
    pub session_id: Option<String>,
    pub model: Option<String>,
    pub input_tokens: u64,
    pub cache_creation_input_tokens: u64,
    pub cached_input_tokens: u64,
    pub output_tokens: u64,
    pub cost_usd: f64,
    pub turns: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
struct AgentOutcome {
    status: String,
    change_id: String,
    action_id: String,
    context_digest: String,
    result_id: Option<String>,
    error_code: Option<String>,
    summary: String,
}

#[derive(Debug, Default, Clone, PartialEq)]
struct ClaudeResult {
    session_id: Option<String>,
    model: Option<String>,
    input_tokens: Option<u64>,
    cache_creation_input_tokens: u64,
    cached_input_tokens: u64,
    output_tokens: Option<u64>,
    cost_usd: Option<f64>,
    turns: Option<u64>,
    outcome: Option<AgentOutcome>,
    failed: bool,
}

pub fn run(options: ClaudeRunnerOptions) -> Result<ClaudeRunnerReport, String> {
    let project_root = options
        .project_root
        .canonicalize()
        .map_err(|error| format!("cannot resolve project root: {error}"))?;
    let next = run_json(
        &options.adf_binary,
        &[
            "next",
            &options.change_id,
            "--project",
            path_text(&project_root)?,
            "--format",
            "json",
        ],
    )?;
    validate_next(&next, &options)?;

    let claude_version = command_text(&options.claude_binary, &["--version"])
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty());
    let mut begin_arguments = vec![
        "execution".to_owned(),
        "begin".to_owned(),
        options.change_id.clone(),
        "--action".to_owned(),
        options.expected_action.clone(),
        "--context".to_owned(),
        options.expected_context.clone(),
        "--provider".to_owned(),
        "anthropic".to_owned(),
        "--surface".to_owned(),
        "claude-code-print".to_owned(),
        "--started-at".to_owned(),
        Utc::now().to_rfc3339(),
        "--project".to_owned(),
        path_text(&project_root)?.to_owned(),
        "--format".to_owned(),
        "json".to_owned(),
    ];
    if let Some(version) = claude_version.as_deref() {
        begin_arguments.extend(["--runner-version".to_owned(), version.to_owned()]);
    }
    let begin = run_json_owned(&options.adf_binary, &begin_arguments)?;
    let execution_id = begin["execution_id"]
        .as_str()
        .ok_or_else(|| "ADF did not return execution_id".to_owned())?
        .to_owned();

    let started = Instant::now();
    let execution = run_claude(&options, &project_root);
    let duration_ms = started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;

    match execution {
        Ok((result, exit_code)) => finish_execution(
            options,
            &project_root,
            execution_id,
            duration_ms,
            result,
            exit_code,
        ),
        Err(error) => {
            complete(
                &options,
                &project_root,
                &execution_id,
                "failed",
                None,
                duration_ms,
                &ClaudeResult::default(),
                1,
                Some("claude-process-failed"),
            )?;
            Err(error)
        }
    }
}

fn finish_execution(
    options: ClaudeRunnerOptions,
    project_root: &Path,
    execution_id: String,
    duration_ms: u64,
    result: ClaudeResult,
    exit_code: i32,
) -> Result<ClaudeRunnerReport, String> {
    let outcome = result
        .outcome
        .as_ref()
        .ok_or_else(|| "Claude Code returned no final structured response".to_owned());
    match outcome.and_then(|outcome| validate_outcome(outcome, &options)) {
        Ok(result_id) if exit_code == 0 && !result.failed => match required_usage(&result) {
            Ok((input, output, cost)) => {
                complete(
                    &options,
                    project_root,
                    &execution_id,
                    "succeeded",
                    Some(&result_id),
                    duration_ms,
                    &result,
                    exit_code,
                    None,
                )?;
                let outcome = result.outcome.expect("validated outcome exists");
                Ok(ClaudeRunnerReport {
                    change_id: options.change_id,
                    action_id: options.expected_action,
                    context_digest: options.expected_context,
                    execution_id,
                    result_id,
                    summary: outcome.summary,
                    session_id: result.session_id,
                    model: result.model,
                    input_tokens: input,
                    cache_creation_input_tokens: result.cache_creation_input_tokens,
                    cached_input_tokens: result.cached_input_tokens,
                    output_tokens: output,
                    cost_usd: cost,
                    turns: result.turns,
                })
            }
            Err(error) => {
                complete(
                    &options,
                    project_root,
                    &execution_id,
                    "failed",
                    Some(&result_id),
                    duration_ms,
                    &result,
                    exit_code,
                    Some("usage-missing"),
                )?;
                Err(error)
            }
        },
        Ok(result_id) => {
            complete(
                &options,
                project_root,
                &execution_id,
                "failed",
                Some(&result_id),
                duration_ms,
                &result,
                exit_code,
                Some("claude-turn-failed"),
            )?;
            Err("Claude Code did not complete the turn successfully".to_owned())
        }
        Err(error) => {
            let outcome = result.outcome.as_ref();
            let completion_status =
                if outcome.and_then(|item| item.error_code.as_deref()) == Some("context-stale") {
                    "stale"
                } else {
                    "failed"
                };
            complete(
                &options,
                project_root,
                &execution_id,
                completion_status,
                outcome.and_then(|item| item.result_id.as_deref()),
                duration_ms,
                &result,
                exit_code,
                outcome
                    .and_then(|item| item.error_code.as_deref())
                    .or(Some("outcome-invalid")),
            )?;
            Err(error)
        }
    }
}

fn validate_next(next: &Value, options: &ClaudeRunnerOptions) -> Result<(), String> {
    let action = &next["next_action"];
    let actual_action = action["id"]
        .as_str()
        .ok_or_else(|| "ADF returned no current Action".to_owned())?;
    let actual_context = next["context"]["digest"]
        .as_str()
        .ok_or_else(|| "ADF returned no generated Context".to_owned())?;
    if actual_action != options.expected_action || actual_context != options.expected_context {
        return Err(format!(
            "the Action or Context is stale: current Action is {actual_action} against {actual_context}"
        ));
    }
    if action["role"].as_str() != Some("Challenger") {
        return Err(
            "the first Claude Code Runner release accepts Challenger Actions only".to_owned(),
        );
    }
    Ok(())
}

fn run_claude(
    options: &ClaudeRunnerOptions,
    project_root: &Path,
) -> Result<(ClaudeResult, i32), String> {
    let schema = output_schema().to_string();
    let prompt = prompt(options);
    let mut command = Command::new(&options.claude_binary);
    command.current_dir(project_root).args([
        "-p",
        &prompt,
        "--output-format",
        "json",
        "--json-schema",
        &schema,
        "--no-session-persistence",
        "--permission-mode",
        "dontAsk",
        "--allowedTools",
        "Bash,Glob,Grep,Read,mcp__*",
    ]);
    if let Some(model) = options.model.as_deref() {
        command.args(["--model", model]);
    }
    let output = command
        .output()
        .map_err(|error| format!("cannot start Claude Code: {error}"))?;
    let exit_code = output.status.code().unwrap_or(1);
    if output.stdout.is_empty() {
        return Err(format!(
            "Claude Code returned no JSON: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let response: Value = serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("Claude Code emitted invalid JSON: {error}"))?;
    let result = parse_result(&response, options)?;
    Ok((result, exit_code))
}

fn parse_result(response: &Value, options: &ClaudeRunnerOptions) -> Result<ClaudeResult, String> {
    let usage = &response["usage"];
    let outcome = response
        .get("structured_output")
        .filter(|value| !value.is_null())
        .map(|value| {
            serde_json::from_value(value.clone())
                .map_err(|error| format!("Claude Code structured output is invalid: {error}"))
        })
        .transpose()?;
    let cost_usd = response["total_cost_usd"].as_f64();
    if cost_usd.is_some_and(|cost| !cost.is_finite() || cost < 0.0) {
        return Err("Claude Code reported invalid total_cost_usd".to_owned());
    }
    Ok(ClaudeResult {
        session_id: response["session_id"].as_str().map(str::to_owned),
        model: used_models(response).or_else(|| options.model.clone()),
        input_tokens: usage["input_tokens"].as_u64(),
        cache_creation_input_tokens: usage["cache_creation_input_tokens"].as_u64().unwrap_or(0),
        cached_input_tokens: usage["cache_read_input_tokens"].as_u64().unwrap_or(0),
        output_tokens: usage["output_tokens"].as_u64(),
        cost_usd,
        turns: response["num_turns"].as_u64(),
        outcome,
        failed: response["is_error"].as_bool().unwrap_or(false)
            || response["subtype"]
                .as_str()
                .is_some_and(|value| value != "success"),
    })
}

fn used_models(response: &Value) -> Option<String> {
    let usage = response
        .get("modelUsage")
        .or_else(|| response.get("model_usage"))?
        .as_object()?;
    if usage.is_empty() {
        return None;
    }
    let mut models = usage.keys().cloned().collect::<Vec<_>>();
    models.sort();
    Some(models.join(","))
}

fn validate_outcome(
    outcome: &AgentOutcome,
    options: &ClaudeRunnerOptions,
) -> Result<String, String> {
    if outcome.status != "submitted"
        || outcome.change_id != options.change_id
        || outcome.action_id != options.expected_action
        || outcome.context_digest != options.expected_context
        || outcome.error_code.is_some()
    {
        return Err(format!(
            "Claude Code did not submit the expected Action: {}",
            outcome.summary
        ));
    }
    outcome
        .result_id
        .clone()
        .ok_or_else(|| "a submitted outcome requires result_id".to_owned())
}

fn required_usage(result: &ClaudeResult) -> Result<(u64, u64, f64), String> {
    Ok((
        result
            .input_tokens
            .ok_or_else(|| "Claude Code did not report input_tokens".to_owned())?,
        result
            .output_tokens
            .ok_or_else(|| "Claude Code did not report output_tokens".to_owned())?,
        result
            .cost_usd
            .ok_or_else(|| "Claude Code did not report total_cost_usd".to_owned())?,
    ))
}

#[allow(clippy::too_many_arguments)]
fn complete(
    options: &ClaudeRunnerOptions,
    project_root: &Path,
    execution_id: &str,
    status: &str,
    result_id: Option<&str>,
    duration_ms: u64,
    result: &ClaudeResult,
    exit_code: i32,
    error_code: Option<&str>,
) -> Result<(), String> {
    let mut arguments = vec![
        "execution".to_owned(),
        "complete".to_owned(),
        execution_id.to_owned(),
        "--change".to_owned(),
        options.change_id.clone(),
        "--status".to_owned(),
        status.to_owned(),
        "--duration-ms".to_owned(),
        duration_ms.to_string(),
        "--completed-at".to_owned(),
        Utc::now().to_rfc3339(),
        "--exit-code".to_owned(),
        exit_code.to_string(),
        "--project".to_owned(),
        path_text(project_root)?.to_owned(),
        "--format".to_owned(),
        "json".to_owned(),
    ];
    push_optional(&mut arguments, "--result", result_id);
    push_optional(&mut arguments, "--model", result.model.as_deref());
    push_optional(&mut arguments, "--thread-id", result.session_id.as_deref());
    push_optional(&mut arguments, "--error-code", error_code);
    push_number(&mut arguments, "--input-tokens", result.input_tokens);
    push_number(
        &mut arguments,
        "--cache-creation-input-tokens",
        Some(result.cache_creation_input_tokens),
    );
    push_number(
        &mut arguments,
        "--cached-input-tokens",
        Some(result.cached_input_tokens),
    );
    push_number(&mut arguments, "--output-tokens", result.output_tokens);
    push_float(&mut arguments, "--cost-usd", result.cost_usd);
    run_json_owned(&options.adf_binary, &arguments).map(|_| ())
}

fn prompt(options: &ClaudeRunnerOptions) -> String {
    format!(
        "You are an independent ADF Challenger execution. Work only on Change {change}, Action {action}, against Context {context}.\n\n1. Call adf_next for {change} before any other ADF write. Stop if its Action ID or Context digest differs.\n2. Read and follow the complete adf-challenger Skill.\n3. Use only the generated ADF Context, current repository state, accepted Contracts, Decisions, Results, Evidence, and repository AGENTS.md as authority. Do not infer product requirements from this launcher prompt.\n4. Perform exactly one issued Challenger Action. Do not implement fixes.\n5. Submit the Result with adf_submit.\n6. Return only the required JSON object. Use status submitted only after adf_submit succeeds and include its result_id. Otherwise use blocked or failed and leave result_id null. Set error_code to context-stale when the Action or Context differs, or to a concise machine-readable code for another stop. Use null after a successful submission.\n\nDo not resume or reconstruct the Builder's conversation. This execution is independent by design.",
        change = options.change_id,
        action = options.expected_action,
        context = options.expected_context,
    )
}

fn output_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "status": {"type": "string", "enum": ["submitted", "blocked", "failed"]},
            "change_id": {"type": "string"},
            "action_id": {"type": "string"},
            "context_digest": {"type": "string"},
            "result_id": {"type": ["string", "null"]},
            "error_code": {"type": ["string", "null"]},
            "summary": {"type": "string"}
        },
        "required": ["status", "change_id", "action_id", "context_digest", "result_id", "error_code", "summary"],
        "additionalProperties": false
    })
}

fn run_json(binary: &Path, arguments: &[&str]) -> Result<Value, String> {
    let output = command_text(binary, arguments)?;
    serde_json::from_str(&output).map_err(|error| format!("invalid JSON from ADF: {error}"))
}

fn run_json_owned(binary: &Path, arguments: &[String]) -> Result<Value, String> {
    let arguments = arguments.iter().map(String::as_str).collect::<Vec<_>>();
    run_json(binary, &arguments)
}

fn command_text(binary: &Path, arguments: &[&str]) -> Result<String, String> {
    let output = Command::new(binary)
        .args(arguments)
        .output()
        .map_err(|error| format!("cannot start {}: {error}", binary.display()))?;
    if !output.status.success() {
        return Err(format!(
            "{} failed: {}",
            binary.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    String::from_utf8(output.stdout).map_err(|error| error.to_string())
}

fn path_text(path: &Path) -> Result<&str, String> {
    path.to_str()
        .ok_or_else(|| format!("path is not valid UTF-8: {}", path.display()))
}

fn push_optional(arguments: &mut Vec<String>, flag: &str, value: Option<&str>) {
    if let Some(value) = value {
        arguments.extend([flag.to_owned(), value.to_owned()]);
    }
}

fn push_number(arguments: &mut Vec<String>, flag: &str, value: Option<u64>) {
    if let Some(value) = value {
        arguments.extend([flag.to_owned(), value.to_string()]);
    }
}

fn push_float(arguments: &mut Vec<String>, flag: &str, value: Option<f64>) {
    if let Some(value) = value {
        arguments.extend([flag.to_owned(), value.to_string()]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_structured_outcome_usage_cost_and_models() {
        let digest = format!("sha256:{}", "a".repeat(64));
        let response = json!({
            "type": "result",
            "subtype": "success",
            "is_error": false,
            "session_id": "session.test",
            "num_turns": 4,
            "total_cost_usd": 0.125,
            "usage": {
                "input_tokens": 10,
                "cache_creation_input_tokens": 8,
                "cache_read_input_tokens": 6,
                "output_tokens": 4
            },
            "modelUsage": {
                "claude-test": {"costUSD": 0.125}
            },
            "structured_output": {
                "status": "submitted",
                "change_id": "change.test",
                "action_id": "action.test",
                "context_digest": digest,
                "result_id": "result.test",
                "error_code": null,
                "summary": "ok"
            }
        });
        let result = parse_result(&response, &options(PathBuf::from("claude"))).unwrap();

        assert_eq!(result.session_id.as_deref(), Some("session.test"));
        assert_eq!(result.model.as_deref(), Some("claude-test"));
        assert_eq!(result.cache_creation_input_tokens, 8);
        assert_eq!(result.cached_input_tokens, 6);
        assert_eq!(result.cost_usd, Some(0.125));
    }

    #[test]
    fn stale_context_is_rejected_before_starting_claude() {
        let options = options(PathBuf::from("claude"));
        let next = json!({
            "next_action": {"id": "action.other", "role": "Challenger"},
            "context": {"digest": format!("sha256:{}", "b".repeat(64))}
        });
        assert!(
            validate_next(&next, &options)
                .unwrap_err()
                .contains("stale")
        );
    }

    #[cfg(unix)]
    #[test]
    fn runner_uses_one_non_persistent_challenge_and_records_usage_and_cost() {
        use std::fs;
        use std::os::unix::fs::PermissionsExt;
        use std::sync::atomic::{AtomicU64, Ordering};

        static SEQUENCE: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "adf-claude-runner-test-{}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&root).unwrap();
        let adf = root.join("fake-adf");
        let claude = root.join("fake-claude");
        let completion_log = root.join("completion.args");
        let invocation_log = root.join("claude.args");
        let digest = format!("sha256:{}", "a".repeat(64));
        fs::write(
            &adf,
            format!(
                r#"#!/bin/sh
if [ "$1" = "next" ]; then
  printf '%s\n' '{{"next_action":{{"id":"action.test","role":"Challenger"}},"context":{{"digest":"{digest}"}}}}'
elif [ "$1" = "execution" ] && [ "$2" = "begin" ]; then
  printf '%s\n' '{{"execution_id":"execution.test"}}'
elif [ "$1" = "execution" ] && [ "$2" = "complete" ]; then
  printf '%s\n' "$@" > '{}'
  printf '%s\n' '{{"execution_id":"execution.test","already_completed":false}}'
else
  exit 2
fi
"#,
                completion_log.display()
            ),
        )
        .unwrap();
        fs::write(
            &claude,
            format!(
                r#"#!/bin/sh
if [ "$1" = "--version" ]; then
  printf '%s\n' 'claude-test 1.0'
  exit 0
fi
printf '%s\n' "$@" > '{}'
printf '%s\n' '{{"type":"result","subtype":"success","is_error":false,"session_id":"session.test","num_turns":3,"total_cost_usd":0.125,"usage":{{"input_tokens":10,"cache_creation_input_tokens":8,"cache_read_input_tokens":6,"output_tokens":4}},"modelUsage":{{"claude-test":{{"costUSD":0.125}}}},"structured_output":{{"status":"submitted","change_id":"change.test","action_id":"action.test","context_digest":"{digest}","result_id":"result.test","error_code":null,"summary":"ok"}}}}'
"#,
                invocation_log.display()
            ),
        )
        .unwrap();
        for path in [&adf, &claude] {
            let mut permissions = fs::metadata(path).unwrap().permissions();
            permissions.set_mode(0o700);
            fs::set_permissions(path, permissions).unwrap();
        }

        let report = run(options(claude)).unwrap();

        assert_eq!(report.result_id, "result.test");
        assert_eq!(report.cache_creation_input_tokens, 8);
        assert_eq!(report.cached_input_tokens, 6);
        assert_eq!(report.cost_usd, 0.125);
        let invocation = fs::read_to_string(invocation_log).unwrap();
        assert!(invocation.contains("--no-session-persistence"));
        assert!(invocation.contains("--json-schema"));
        let completion = fs::read_to_string(completion_log).unwrap();
        assert!(completion.contains("--cache-creation-input-tokens\n8"));
        assert!(completion.contains("--cached-input-tokens\n6"));
        assert!(completion.contains("--cost-usd\n0.125"));

        fs::remove_dir_all(root).unwrap();
    }

    fn options(claude_binary: PathBuf) -> ClaudeRunnerOptions {
        ClaudeRunnerOptions {
            project_root: claude_binary
                .parent()
                .filter(|path| path.exists())
                .unwrap_or_else(|| Path::new("."))
                .to_path_buf(),
            change_id: "change.test".to_owned(),
            expected_action: "action.test".to_owned(),
            expected_context: format!("sha256:{}", "a".repeat(64)),
            adf_binary: claude_binary
                .parent()
                .map(|path| path.join("fake-adf"))
                .unwrap_or_else(|| PathBuf::from("adf")),
            claude_binary,
            model: Some("requested-model".to_owned()),
        }
    }
}
