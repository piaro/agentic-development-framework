//! Read-only aggregation of execution metadata reported by Results and runners.
//!
//! Building this report performs no timing, tracing, network access, or model
//! invocation. Missing runner measurements remain explicitly unknown.

use crate::execution_record::{ExecutionEvent, ExecutionStatus};
use crate::kernel::ProjectSnapshot;
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

pub const EXECUTION_LOG_SCHEMA_VERSION: &str = "3";

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ExecutionLogEntry {
    pub execution_id: Option<String>,
    pub result_id: Option<String>,
    pub action_id: String,
    pub role: String,
    pub result_schema: String,
    pub context_bytes: u64,
    pub status: Option<String>,
    pub runner_provider: Option<String>,
    pub runner_surface: Option<String>,
    pub runner_version: Option<String>,
    pub duration_ms: Option<u64>,
    pub model: Option<String>,
    pub input_tokens: Option<u64>,
    pub cache_creation_input_tokens: Option<u64>,
    pub cached_input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub reasoning_output_tokens: Option<u64>,
    pub cost_usd: Option<f64>,
    pub tool_calls: Option<u64>,
    pub retries: Option<u64>,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
    pub thread_id: Option<String>,
    pub exit_code: Option<i32>,
    pub error_code: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ExecutionLog {
    pub schema_version: String,
    pub change_id: String,
    pub entries: Vec<ExecutionLogEntry>,
    pub totals: Value,
}

impl ExecutionLog {
    pub fn build(snapshot: &ProjectSnapshot) -> Self {
        Self::build_with_events(snapshot, &[])
    }

    pub fn build_with_events(snapshot: &ProjectSnapshot, events: &[ExecutionEvent]) -> Self {
        let runner_entries = runner_entries(events);
        let externally_measured_results = runner_entries
            .iter()
            .filter(|entry| entry.status.as_deref() == Some("succeeded"))
            .filter_map(|entry| entry.result_id.as_deref())
            .collect::<BTreeSet<_>>();
        let mut entries = snapshot
            .results
            .iter()
            .filter(|result| {
                result["id"]
                    .as_str()
                    .is_none_or(|id| !externally_measured_results.contains(id))
            })
            .filter_map(result_entry)
            .collect::<Vec<_>>();
        entries.extend(runner_entries);
        entries.sort_by(|left, right| {
            left.started_at
                .cmp(&right.started_at)
                .then_with(|| left.action_id.cmp(&right.action_id))
                .then_with(|| left.execution_id.cmp(&right.execution_id))
        });
        let totals = json!({
            "actions": entries.len(),
            "context_bytes": entries.iter().map(|item| item.context_bytes).sum::<u64>(),
            "duration_ms": sum_known(entries.iter().map(|item| item.duration_ms)),
            "input_tokens": sum_known(entries.iter().map(|item| item.input_tokens)),
            "cache_creation_input_tokens": sum_known(entries.iter().map(|item| item.cache_creation_input_tokens)),
            "cached_input_tokens": sum_known(entries.iter().map(|item| item.cached_input_tokens)),
            "output_tokens": sum_known(entries.iter().map(|item| item.output_tokens)),
            "reasoning_output_tokens": sum_known(entries.iter().map(|item| item.reasoning_output_tokens)),
            "cost_usd": sum_known_f64(entries.iter().map(|item| item.cost_usd)),
            "tool_calls": sum_known(entries.iter().map(|item| item.tool_calls)),
            "retries": sum_known(entries.iter().map(|item| item.retries)),
            "entries_without_token_counts": entries.iter().filter(|item| {
                item.input_tokens.is_none() || item.output_tokens.is_none()
            }).count(),
            "incomplete_executions": entries.iter().filter(|item| {
                item.execution_id.is_some() && item.status.is_none()
            }).count(),
        });
        Self {
            schema_version: EXECUTION_LOG_SCHEMA_VERSION.to_owned(),
            change_id: snapshot.change_id.clone(),
            entries,
            totals,
        }
    }

    pub fn as_value(&self) -> Value {
        serde_json::to_value(self).expect("Execution Log is serializable")
    }

    pub fn render_text(&self) -> String {
        let mut output = format!(
            "change: {}\nactions: {}\n",
            self.change_id,
            self.entries.len()
        );
        for entry in &self.entries {
            output.push_str(&format!(
                "- {} {} status={} context_bytes={} duration_ms={} tokens={}+{} cost_usd={} model={}\n",
                entry.role,
                entry.result_schema,
                entry.status.as_deref().unwrap_or("unknown"),
                entry.context_bytes,
                display_number(entry.duration_ms),
                display_number(entry.input_tokens),
                display_number(entry.output_tokens),
                display_cost(entry.cost_usd),
                entry.model.as_deref().unwrap_or("unknown"),
            ));
        }
        output
    }
}

fn result_entry(result: &Value) -> Option<ExecutionLogEntry> {
    let execution = result["execution"].as_object()?;
    Some(ExecutionLogEntry {
        execution_id: None,
        result_id: result["id"].as_str().map(str::to_owned),
        action_id: result["action_id"].as_str()?.to_owned(),
        role: result["role"].as_str()?.to_owned(),
        result_schema: result["result_schema"].as_str()?.to_owned(),
        context_bytes: execution.get("context_bytes")?.as_u64()?,
        status: Some("result-recorded".to_owned()),
        runner_provider: None,
        runner_surface: None,
        runner_version: None,
        duration_ms: number(execution, "duration_ms"),
        model: text(execution, "model"),
        input_tokens: number(execution, "input_tokens"),
        cache_creation_input_tokens: None,
        cached_input_tokens: None,
        output_tokens: number(execution, "output_tokens"),
        reasoning_output_tokens: None,
        cost_usd: None,
        tool_calls: number(execution, "tool_calls"),
        retries: number(execution, "retries"),
        started_at: text(execution, "started_at"),
        completed_at: text(execution, "completed_at"),
        thread_id: None,
        exit_code: None,
        error_code: None,
    })
}

fn runner_entries(events: &[ExecutionEvent]) -> Vec<ExecutionLogEntry> {
    let mut starts = BTreeMap::new();
    let mut completions = BTreeMap::new();
    for event in events {
        match event {
            ExecutionEvent::Started {
                execution_id,
                change_id: _,
                action_id,
                context_digest: _,
                role,
                result_schema,
                context_bytes,
                runner,
                started_at,
                schema_version: _,
            } => {
                starts.insert(
                    execution_id.clone(),
                    (
                        action_id.clone(),
                        role.clone(),
                        result_schema.clone(),
                        *context_bytes,
                        runner.clone(),
                        started_at.clone(),
                    ),
                );
            }
            ExecutionEvent::Completed {
                execution_id,
                completion,
                ..
            } => {
                completions.insert(execution_id.clone(), completion.clone());
            }
        }
    }
    starts
        .into_iter()
        .map(
            |(
                execution_id,
                (action_id, role, result_schema, context_bytes, runner, started_at),
            )| {
                let completion = completions.remove(&execution_id);
                ExecutionLogEntry {
                    execution_id: Some(execution_id),
                    result_id: completion.as_ref().and_then(|item| item.result_id.clone()),
                    action_id,
                    role,
                    result_schema,
                    context_bytes,
                    status: completion
                        .as_ref()
                        .map(|item| status_text(item.status).to_owned()),
                    runner_provider: Some(runner.provider),
                    runner_surface: Some(runner.surface),
                    runner_version: runner.version,
                    duration_ms: completion.as_ref().and_then(|item| item.duration_ms),
                    model: completion.as_ref().and_then(|item| item.model.clone()),
                    input_tokens: completion.as_ref().and_then(|item| item.input_tokens),
                    cache_creation_input_tokens: completion
                        .as_ref()
                        .and_then(|item| item.cache_creation_input_tokens),
                    cached_input_tokens: completion
                        .as_ref()
                        .and_then(|item| item.cached_input_tokens),
                    output_tokens: completion.as_ref().and_then(|item| item.output_tokens),
                    reasoning_output_tokens: completion
                        .as_ref()
                        .and_then(|item| item.reasoning_output_tokens),
                    cost_usd: completion.as_ref().and_then(|item| item.cost_usd),
                    tool_calls: completion.as_ref().and_then(|item| item.tool_calls),
                    retries: completion.as_ref().and_then(|item| item.retries),
                    completed_at: completion
                        .as_ref()
                        .and_then(|item| item.completed_at.clone()),
                    thread_id: completion.as_ref().and_then(|item| item.thread_id.clone()),
                    exit_code: completion.as_ref().and_then(|item| item.exit_code),
                    error_code: completion.as_ref().and_then(|item| item.error_code.clone()),
                    started_at,
                }
            },
        )
        .collect()
}

fn status_text(status: ExecutionStatus) -> &'static str {
    match status {
        ExecutionStatus::Succeeded => "succeeded",
        ExecutionStatus::Failed => "failed",
        ExecutionStatus::Interrupted => "interrupted",
        ExecutionStatus::Stale => "stale",
    }
}

fn number(execution: &serde_json::Map<String, Value>, field: &str) -> Option<u64> {
    execution.get(field).and_then(Value::as_u64)
}

fn text(execution: &serde_json::Map<String, Value>, field: &str) -> Option<String> {
    execution
        .get(field)
        .and_then(Value::as_str)
        .map(str::to_owned)
}

fn sum_known(values: impl Iterator<Item = Option<u64>>) -> Value {
    let values = values.collect::<Option<Vec<_>>>();
    values
        .map(|values| Value::from(values.into_iter().sum::<u64>()))
        .unwrap_or(Value::Null)
}

fn sum_known_f64(values: impl Iterator<Item = Option<f64>>) -> Value {
    let values = values.collect::<Option<Vec<_>>>();
    values
        .and_then(|values| serde_json::Number::from_f64(values.into_iter().sum::<f64>()))
        .map(Value::Number)
        .unwrap_or(Value::Null)
}

fn display_number(value: Option<u64>) -> String {
    value.map_or_else(|| "unknown".to_owned(), |value| value.to_string())
}

fn display_cost(value: Option<f64>) -> String {
    value.map_or_else(|| "unknown".to_owned(), |value| value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn missing_optional_measurements_do_not_trigger_work_or_estimates() {
        let snapshot = ProjectSnapshot {
            change_id: "change.test".to_owned(),
            change: json!({}),
            contracts: Vec::new(),
            decisions: Vec::new(),
            results: vec![json!({
                "id": "result.test",
                "action_id": "action.test",
                "role": "Analyst",
                "result_schema": "result.impact-assessment",
                "execution": {"context_bytes": 42}
            })],
            evidence: Vec::new(),
            repository: json!({}),
            artifact_digests: BTreeMap::new(),
            digest: String::new(),
        };

        let report = ExecutionLog::build(&snapshot);

        assert_eq!(report.entries[0].context_bytes, 42);
        assert_eq!(report.totals["duration_ms"], Value::Null);
        assert_eq!(report.totals["entries_without_token_counts"], 1);
    }
}
