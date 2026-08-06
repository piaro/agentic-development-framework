//! Optional one-Action adapter between an ADF project and `codex exec`.
//!
//! The runner is deliberately outside the ADF control plane. It launches only
//! when another process invokes it, and the first release accepts Challenger
//! Actions only so independent post-build review can be proven before broader
//! write-capable roles are enabled.

use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Instant;

#[derive(Debug, Clone)]
pub struct CodexRunnerOptions {
    pub project_root: PathBuf,
    pub change_id: String,
    pub expected_action: String,
    pub expected_context: String,
    pub adf_binary: PathBuf,
    pub codex_binary: PathBuf,
    pub model: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CodexRunnerReport {
    pub change_id: String,
    pub action_id: String,
    pub context_digest: String,
    pub execution_id: String,
    pub result_id: String,
    pub summary: String,
    pub thread_id: Option<String>,
    pub input_tokens: u64,
    pub cached_input_tokens: u64,
    pub output_tokens: u64,
    pub reasoning_output_tokens: u64,
    pub tool_calls: u64,
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

#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct CodexEvents {
    thread_id: Option<String>,
    input_tokens: Option<u64>,
    cached_input_tokens: Option<u64>,
    output_tokens: Option<u64>,
    reasoning_output_tokens: Option<u64>,
    tool_calls: u64,
    final_message: Option<String>,
    failed: bool,
}

pub fn run(options: CodexRunnerOptions) -> Result<CodexRunnerReport, String> {
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

    let codex_version = command_text(&options.codex_binary, &["--version"])
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
        "openai".to_owned(),
        "--surface".to_owned(),
        "codex-exec".to_owned(),
        "--started-at".to_owned(),
        Utc::now().to_rfc3339(),
        "--project".to_owned(),
        path_text(&project_root)?.to_owned(),
        "--format".to_owned(),
        "json".to_owned(),
    ];
    if let Some(version) = codex_version.as_deref() {
        begin_arguments.extend(["--runner-version".to_owned(), version.to_owned()]);
    }
    let begin = run_json_owned(&options.adf_binary, &begin_arguments)?;
    let execution_id = begin["execution_id"]
        .as_str()
        .ok_or_else(|| "ADF did not return execution_id".to_owned())?
        .to_owned();

    let started = Instant::now();
    let execution = run_codex(&options, &project_root);
    let duration_ms = started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;

    match execution {
        Ok((events, outcome, exit_code)) => {
            let result = validate_outcome(&outcome, &options);
            match result {
                Ok(result_id) if exit_code == 0 && !events.failed => {
                    let usage = required_usage(&events);
                    match usage {
                        Ok((input, cached, output, reasoning)) => {
                            complete(
                                &options,
                                &project_root,
                                &execution_id,
                                "succeeded",
                                Some(&result_id),
                                duration_ms,
                                &events,
                                exit_code,
                                None,
                            )?;
                            Ok(CodexRunnerReport {
                                change_id: options.change_id,
                                action_id: options.expected_action,
                                context_digest: options.expected_context,
                                execution_id,
                                result_id,
                                summary: outcome.summary,
                                thread_id: events.thread_id,
                                input_tokens: input,
                                cached_input_tokens: cached,
                                output_tokens: output,
                                reasoning_output_tokens: reasoning,
                                tool_calls: events.tool_calls,
                            })
                        }
                        Err(error) => {
                            complete(
                                &options,
                                &project_root,
                                &execution_id,
                                "failed",
                                Some(&result_id),
                                duration_ms,
                                &events,
                                exit_code,
                                Some("usage-missing"),
                            )?;
                            Err(error)
                        }
                    }
                }
                Ok(result_id) => {
                    complete(
                        &options,
                        &project_root,
                        &execution_id,
                        "failed",
                        Some(&result_id),
                        duration_ms,
                        &events,
                        exit_code,
                        Some("codex-turn-failed"),
                    )?;
                    Err("Codex did not complete the turn successfully".to_owned())
                }
                Err(error) => {
                    let completion_status =
                        if outcome.error_code.as_deref() == Some("context-stale") {
                            "stale"
                        } else {
                            "failed"
                        };
                    complete(
                        &options,
                        &project_root,
                        &execution_id,
                        completion_status,
                        outcome.result_id.as_deref(),
                        duration_ms,
                        &events,
                        exit_code,
                        outcome.error_code.as_deref().or(Some("outcome-invalid")),
                    )?;
                    Err(error)
                }
            }
        }
        Err(error) => {
            let events = CodexEvents::default();
            complete(
                &options,
                &project_root,
                &execution_id,
                "failed",
                None,
                duration_ms,
                &events,
                1,
                Some("codex-process-failed"),
            )?;
            Err(error)
        }
    }
}

fn validate_next(next: &Value, options: &CodexRunnerOptions) -> Result<(), String> {
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
        return Err("the first Codex Runner release accepts Challenger Actions only".to_owned());
    }
    Ok(())
}

fn run_codex(
    options: &CodexRunnerOptions,
    project_root: &Path,
) -> Result<(CodexEvents, AgentOutcome, i32), String> {
    let schema_path = write_output_schema()?;
    let mut command = Command::new(&options.codex_binary);
    command.args([
        "exec",
        "--json",
        "--ephemeral",
        "--sandbox",
        "workspace-write",
        "-C",
        path_text(project_root)?,
        "--output-schema",
        path_text(&schema_path)?,
    ]);
    if let Some(model) = options.model.as_deref() {
        command.args(["--model", model]);
    }
    command
        .arg("-")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit());
    let mut child = command
        .spawn()
        .map_err(|error| format!("cannot start codex exec: {error}"))?;
    let prompt = prompt(options);
    child
        .stdin
        .take()
        .ok_or_else(|| "cannot open Codex stdin".to_owned())?
        .write_all(prompt.as_bytes())
        .map_err(|error| format!("cannot send Codex prompt: {error}"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "cannot open Codex stdout".to_owned())?;
    let mut events = CodexEvents::default();
    for line in BufReader::new(stdout).lines() {
        let line = line.map_err(|error| format!("cannot read Codex JSONL: {error}"))?;
        parse_event(&line, &mut events)?;
    }
    let status = child
        .wait()
        .map_err(|error| format!("cannot wait for codex exec: {error}"))?;
    let _ = fs::remove_file(&schema_path);
    let final_message = events
        .final_message
        .as_deref()
        .ok_or_else(|| "Codex returned no final structured response".to_owned())?;
    let outcome = serde_json::from_str(final_message)
        .map_err(|error| format!("Codex final response is invalid: {error}"))?;
    Ok((events, outcome, status.code().unwrap_or(1)))
}

fn parse_event(line: &str, events: &mut CodexEvents) -> Result<(), String> {
    let event: Value = serde_json::from_str(line)
        .map_err(|error| format!("Codex emitted invalid JSONL: {error}"))?;
    match event["type"].as_str() {
        Some("thread.started") => {
            events.thread_id = event["thread_id"].as_str().map(str::to_owned);
        }
        Some("turn.completed") => {
            let usage = &event["usage"];
            events.input_tokens = usage["input_tokens"].as_u64();
            events.cached_input_tokens = usage["cached_input_tokens"].as_u64();
            events.output_tokens = usage["output_tokens"].as_u64();
            events.reasoning_output_tokens = usage["reasoning_output_tokens"].as_u64();
        }
        Some("turn.failed") | Some("error") => events.failed = true,
        Some("item.completed") => match event["item"]["type"].as_str() {
            Some("agent_message") => {
                events.final_message = event["item"]["text"].as_str().map(str::to_owned);
            }
            Some("command_execution" | "mcp_tool_call" | "web_search") => {
                events.tool_calls += 1;
            }
            _ => {}
        },
        _ => {}
    }
    Ok(())
}

fn validate_outcome(
    outcome: &AgentOutcome,
    options: &CodexRunnerOptions,
) -> Result<String, String> {
    if outcome.status != "submitted"
        || outcome.change_id != options.change_id
        || outcome.action_id != options.expected_action
        || outcome.context_digest != options.expected_context
        || outcome.error_code.is_some()
    {
        return Err(format!(
            "Codex did not submit the expected Action: {}",
            outcome.summary
        ));
    }
    outcome
        .result_id
        .clone()
        .ok_or_else(|| "a submitted outcome requires result_id".to_owned())
}

fn required_usage(events: &CodexEvents) -> Result<(u64, u64, u64, u64), String> {
    Ok((
        events
            .input_tokens
            .ok_or_else(|| "Codex did not report input_tokens".to_owned())?,
        events
            .cached_input_tokens
            .ok_or_else(|| "Codex did not report cached_input_tokens".to_owned())?,
        events
            .output_tokens
            .ok_or_else(|| "Codex did not report output_tokens".to_owned())?,
        events
            .reasoning_output_tokens
            .ok_or_else(|| "Codex did not report reasoning_output_tokens".to_owned())?,
    ))
}

#[allow(clippy::too_many_arguments)]
fn complete(
    options: &CodexRunnerOptions,
    project_root: &Path,
    execution_id: &str,
    status: &str,
    result_id: Option<&str>,
    duration_ms: u64,
    events: &CodexEvents,
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
        "--tool-calls".to_owned(),
        events.tool_calls.to_string(),
        "--retries".to_owned(),
        "0".to_owned(),
        "--exit-code".to_owned(),
        exit_code.to_string(),
        "--project".to_owned(),
        path_text(project_root)?.to_owned(),
        "--format".to_owned(),
        "json".to_owned(),
    ];
    push_optional(&mut arguments, "--result", result_id);
    push_optional(&mut arguments, "--model", options.model.as_deref());
    push_optional(&mut arguments, "--thread-id", events.thread_id.as_deref());
    push_optional(&mut arguments, "--error-code", error_code);
    push_number(&mut arguments, "--input-tokens", events.input_tokens);
    push_number(
        &mut arguments,
        "--cached-input-tokens",
        events.cached_input_tokens,
    );
    push_number(&mut arguments, "--output-tokens", events.output_tokens);
    push_number(
        &mut arguments,
        "--reasoning-output-tokens",
        events.reasoning_output_tokens,
    );
    run_json_owned(&options.adf_binary, &arguments).map(|_| ())
}

fn prompt(options: &CodexRunnerOptions) -> String {
    format!(
        "You are an independent ADF Challenger execution. Work only on Change {change}, Action {action}, against Context {context}.\n\n1. Call adf_next for {change} before any other ADF write. Stop if its Action ID or Context digest differs.\n2. Read and follow the complete adf-challenger Skill.\n3. Use only the generated ADF Context, current repository state, accepted Contracts, Decisions, Results, Evidence, and repository AGENTS.md as authority. Do not infer product requirements from this launcher prompt.\n4. Perform exactly one issued Challenger Action. Do not implement fixes.\n5. Submit the Result with adf_submit.\n6. Return only the required JSON object. Use status submitted only after adf_submit succeeds and include its result_id. Otherwise use blocked or failed and leave result_id null. Set error_code to context-stale when the Action or Context differs, or to a concise machine-readable code for another stop. Use null after a successful submission.\n\nDo not resume or reconstruct the Builder's conversation. This execution is independent by design.",
        change = options.change_id,
        action = options.expected_action,
        context = options.expected_context,
    )
}

fn write_output_schema() -> Result<PathBuf, String> {
    let path = std::env::temp_dir().join(format!(
        "adf-codex-runner-{}-output-schema.json",
        std::process::id()
    ));
    let schema = json!({
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
    });
    fs::write(
        &path,
        serde_json::to_vec_pretty(&schema).map_err(|error| error.to_string())?,
    )
    .map_err(|error| format!("cannot write output schema: {error}"))?;
    Ok(path)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_turn_usage_and_final_structured_message() {
        let mut events = CodexEvents::default();
        parse_event(
            r#"{"type":"thread.started","thread_id":"thread-1"}"#,
            &mut events,
        )
        .unwrap();
        parse_event(
            r#"{"type":"item.completed","item":{"type":"mcp_tool_call"}}"#,
            &mut events,
        )
        .unwrap();
        parse_event(
            r#"{"type":"item.completed","item":{"type":"agent_message","text":"{\"status\":\"submitted\"}"}}"#,
            &mut events,
        )
        .unwrap();
        parse_event(
            r#"{"type":"turn.completed","usage":{"input_tokens":10,"cached_input_tokens":4,"output_tokens":3,"reasoning_output_tokens":2}}"#,
            &mut events,
        )
        .unwrap();

        assert_eq!(events.thread_id.as_deref(), Some("thread-1"));
        assert_eq!(events.tool_calls, 1);
        assert_eq!(events.input_tokens, Some(10));
        assert_eq!(events.reasoning_output_tokens, Some(2));
        assert_eq!(
            events.final_message.as_deref(),
            Some(r#"{"status":"submitted"}"#)
        );
    }

    #[test]
    fn stale_context_is_rejected_before_starting_codex() {
        let options = CodexRunnerOptions {
            project_root: PathBuf::from("."),
            change_id: "change.test".to_owned(),
            expected_action: "action.expected".to_owned(),
            expected_context: "sha256:expected".to_owned(),
            adf_binary: PathBuf::from("adf"),
            codex_binary: PathBuf::from("codex"),
            model: None,
        };
        let next = json!({
            "next_action": {"id": "action.other", "role": "Challenger"},
            "context": {"digest": "sha256:other"}
        });
        assert!(
            validate_next(&next, &options)
                .unwrap_err()
                .contains("stale")
        );
    }

    #[cfg(unix)]
    #[test]
    fn runner_uses_one_ephemeral_challenge_and_reports_final_usage() {
        use std::os::unix::fs::PermissionsExt;
        use std::sync::atomic::{AtomicU64, Ordering};

        static SEQUENCE: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "adf-codex-runner-test-{}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&root).unwrap();
        let adf = root.join("fake-adf");
        let codex = root.join("fake-codex");
        let completion_log = root.join("completion.args");
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
            &codex,
            format!(
                r#"#!/bin/sh
if [ "$1" = "--version" ]; then
  printf '%s\n' 'codex-test 1.0'
  exit 0
fi
cat >/dev/null
printf '%s\n' '{{"type":"thread.started","thread_id":"thread.test"}}'
printf '%s\n' '{{"type":"item.completed","item":{{"type":"mcp_tool_call"}}}}'
printf '%s\n' '{{"type":"item.completed","item":{{"type":"agent_message","text":"{{\"status\":\"submitted\",\"change_id\":\"change.test\",\"action_id\":\"action.test\",\"context_digest\":\"{digest}\",\"result_id\":\"result.test\",\"error_code\":null,\"summary\":\"ok\"}}"}}}}'
printf '%s\n' '{{"type":"turn.completed","usage":{{"input_tokens":10,"cached_input_tokens":8,"output_tokens":4,"reasoning_output_tokens":2}}}}'
"#
            ),
        )
        .unwrap();
        for path in [&adf, &codex] {
            let mut permissions = fs::metadata(path).unwrap().permissions();
            permissions.set_mode(0o700);
            fs::set_permissions(path, permissions).unwrap();
        }

        let report = run(CodexRunnerOptions {
            project_root: root.clone(),
            change_id: "change.test".to_owned(),
            expected_action: "action.test".to_owned(),
            expected_context: digest,
            adf_binary: adf,
            codex_binary: codex,
            model: Some("test-model".to_owned()),
        })
        .unwrap();

        assert_eq!(report.result_id, "result.test");
        assert_eq!(report.cached_input_tokens, 8);
        let completion = fs::read_to_string(completion_log).unwrap();
        assert!(completion.contains("--cached-input-tokens\n8"));
        assert!(completion.contains("--reasoning-output-tokens\n2"));

        fs::remove_dir_all(root).unwrap();
    }
}
