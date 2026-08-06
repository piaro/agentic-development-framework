//! Read-only aggregation of execution metadata already stored with Results.
//!
//! Building this report performs no timing, tracing, network access, or model
//! invocation. Missing orchestrator measurements remain explicitly unknown.

use crate::kernel::ProjectSnapshot;
use serde::Serialize;
use serde_json::{Value, json};

pub const EXECUTION_LOG_SCHEMA_VERSION: &str = "1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExecutionLogEntry {
    pub result_id: String,
    pub action_id: String,
    pub role: String,
    pub result_schema: String,
    pub context_bytes: u64,
    pub duration_ms: Option<u64>,
    pub model: Option<String>,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub tool_calls: Option<u64>,
    pub retries: Option<u64>,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExecutionLog {
    pub schema_version: String,
    pub change_id: String,
    pub entries: Vec<ExecutionLogEntry>,
    pub totals: Value,
}

impl ExecutionLog {
    pub fn build(snapshot: &ProjectSnapshot) -> Self {
        let entries = snapshot
            .results
            .iter()
            .filter_map(entry)
            .collect::<Vec<_>>();
        let totals = json!({
            "actions": entries.len(),
            "context_bytes": entries.iter().map(|item| item.context_bytes).sum::<u64>(),
            "duration_ms": sum_known(entries.iter().map(|item| item.duration_ms)),
            "input_tokens": sum_known(entries.iter().map(|item| item.input_tokens)),
            "output_tokens": sum_known(entries.iter().map(|item| item.output_tokens)),
            "tool_calls": sum_known(entries.iter().map(|item| item.tool_calls)),
            "retries": sum_known(entries.iter().map(|item| item.retries)),
            "entries_without_token_counts": entries.iter().filter(|item| {
                item.input_tokens.is_none() || item.output_tokens.is_none()
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
                "- {} {} context_bytes={} duration_ms={} tokens={}+{} model={}\n",
                entry.role,
                entry.result_schema,
                entry.context_bytes,
                display_number(entry.duration_ms),
                display_number(entry.input_tokens),
                display_number(entry.output_tokens),
                entry.model.as_deref().unwrap_or("unknown"),
            ));
        }
        output
    }
}

fn entry(result: &Value) -> Option<ExecutionLogEntry> {
    let execution = result["execution"].as_object()?;
    Some(ExecutionLogEntry {
        result_id: result["id"].as_str()?.to_owned(),
        action_id: result["action_id"].as_str()?.to_owned(),
        role: result["role"].as_str()?.to_owned(),
        result_schema: result["result_schema"].as_str()?.to_owned(),
        context_bytes: execution.get("context_bytes")?.as_u64()?,
        duration_ms: number(execution, "duration_ms"),
        model: execution
            .get("model")
            .and_then(Value::as_str)
            .map(str::to_owned),
        input_tokens: number(execution, "input_tokens"),
        output_tokens: number(execution, "output_tokens"),
        tool_calls: number(execution, "tool_calls"),
        retries: number(execution, "retries"),
        started_at: execution
            .get("started_at")
            .and_then(Value::as_str)
            .map(str::to_owned),
        completed_at: execution
            .get("completed_at")
            .and_then(Value::as_str)
            .map(str::to_owned),
    })
}

fn number(execution: &serde_json::Map<String, Value>, field: &str) -> Option<u64> {
    execution.get(field).and_then(Value::as_u64)
}

fn sum_known(values: impl Iterator<Item = Option<u64>>) -> Value {
    let values = values.collect::<Option<Vec<_>>>();
    values
        .map(|values| Value::from(values.into_iter().sum::<u64>()))
        .unwrap_or(Value::Null)
}

fn display_number(value: Option<u64>) -> String {
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
