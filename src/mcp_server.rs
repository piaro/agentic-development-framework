//! Local stdio MCP adapter for one fixed Project root.

use crate::project_application::{
    AbandonActionResponse, IssuedActionKey, NextServiceResponse, ProjectApplicationService,
    RecordWriteResponse, ServiceError, SubmitServiceResponse,
};
use rmcp::{
    Json, ServerHandler, ServiceExt,
    handler::server::{router::tool::ToolRouter, tool::IntoCallToolResult, wrapper::Parameters},
    model::{CallToolResponse, CallToolResult},
    tool, tool_handler, tool_router,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ReportToolResponse {
    pub schema_version: String,
    pub report: Value,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NextToolInput {
    pub change_id: String,
    #[serde(default)]
    pub require_clean: bool,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RepositoryToolInput {
    #[serde(default)]
    pub require_clean: bool,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SubmitToolInput {
    pub change_id: String,
    pub action_id: String,
    pub context_digest: String,
    pub payload: Value,
    #[serde(default)]
    pub output_refs: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EvidenceToolInput {
    pub change_id: String,
    pub action_id: String,
    pub context_digest: String,
    pub evidence: Value,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DecisionToolInput {
    pub change_id: String,
    pub action_id: String,
    pub context_digest: String,
    pub decision: Value,
    pub expected_digest: Value,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ContractToolInput {
    pub change_id: String,
    pub action_id: String,
    pub context_digest: String,
    pub contract: Value,
    pub expected_digest: Value,
    #[serde(default)]
    pub expected_clause_digests: Option<BTreeMap<String, String>>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ActionToolInput {
    pub change_id: String,
    pub action_id: String,
    pub context_digest: String,
}

impl From<&SubmitToolInput> for IssuedActionKey {
    fn from(input: &SubmitToolInput) -> Self {
        action_key(&input.change_id, &input.action_id, &input.context_digest)
    }
}

impl From<&EvidenceToolInput> for IssuedActionKey {
    fn from(input: &EvidenceToolInput) -> Self {
        action_key(&input.change_id, &input.action_id, &input.context_digest)
    }
}

impl From<&DecisionToolInput> for IssuedActionKey {
    fn from(input: &DecisionToolInput) -> Self {
        action_key(&input.change_id, &input.action_id, &input.context_digest)
    }
}

impl From<&ContractToolInput> for IssuedActionKey {
    fn from(input: &ContractToolInput) -> Self {
        action_key(&input.change_id, &input.action_id, &input.context_digest)
    }
}

impl From<&ActionToolInput> for IssuedActionKey {
    fn from(input: &ActionToolInput) -> Self {
        action_key(&input.change_id, &input.action_id, &input.context_digest)
    }
}

fn action_key(change_id: &str, action_id: &str, context_digest: &str) -> IssuedActionKey {
    IssuedActionKey {
        change_id: change_id.to_owned(),
        action_id: action_id.to_owned(),
        context_digest: context_digest.to_owned(),
    }
}

impl IntoCallToolResult for ServiceError {
    fn into_call_tool_result(self) -> Result<CallToolResponse, rmcp::ErrorData> {
        CallToolResult::structured_error(self.as_value()).into_call_tool_result()
    }
}

#[derive(Clone)]
pub struct AgenticMcpServer {
    service: Arc<Mutex<ProjectApplicationService>>,
    tool_router: ToolRouter<Self>,
}

impl AgenticMcpServer {
    pub fn new(service: ProjectApplicationService) -> Self {
        Self {
            service: Arc::new(Mutex::new(service)),
            tool_router: Self::tool_router(),
        }
    }
}

#[tool_router(router = tool_router)]
impl AgenticMcpServer {
    /// Evaluate the current Project and issue its next Action.
    #[tool(
        name = "adf_next",
        description = "Evaluate one Change and issue the next Action with generated Context.",
        annotations(title = "Agentic Next", read_only_hint = true)
    )]
    async fn adf_next(
        &self,
        Parameters(input): Parameters<NextToolInput>,
    ) -> Result<Json<NextServiceResponse>, ServiceError> {
        self.service
            .lock()
            .await
            .next(&input.change_id, input.require_clean)
            .map(Json)
    }

    /// Explain the current deterministic decision without issuing an Action.
    #[tool(
        name = "adf_explain",
        description = "Explain the current decision for one Change without changing Project state.",
        annotations(title = "Agentic Explain", read_only_hint = true)
    )]
    async fn adf_explain(
        &self,
        Parameters(input): Parameters<NextToolInput>,
    ) -> Result<Json<ReportToolResponse>, ServiceError> {
        self.service
            .lock()
            .await
            .explain(&input.change_id, input.require_clean)
            .map(|report| {
                Json(ReportToolResponse {
                    schema_version: "1".to_owned(),
                    report,
                })
            })
    }

    /// Recompute repository-wide Contract health.
    #[tool(
        name = "adf_contract_health",
        description = "Recompute Contract clause health from current Records and repository inputs.",
        annotations(title = "Agentic Contract Health", read_only_hint = true)
    )]
    async fn adf_contract_health(
        &self,
        Parameters(input): Parameters<RepositoryToolInput>,
    ) -> Result<Json<ReportToolResponse>, ServiceError> {
        self.service
            .lock()
            .await
            .contract_health(input.require_clean)
            .map(|report| {
                Json(ReportToolResponse {
                    schema_version: "1".to_owned(),
                    report,
                })
            })
    }

    /// Validate and persist the Result for an Action issued in this MCP session.
    #[tool(
        name = "adf_submit",
        description = "Validate and persist an issued Action Result, then return the reevaluated next Action.",
        annotations(
            title = "Agentic Submit",
            read_only_hint = false,
            destructive_hint = false
        )
    )]
    async fn adf_submit(
        &self,
        Parameters(input): Parameters<SubmitToolInput>,
    ) -> Result<Json<SubmitServiceResponse>, ServiceError> {
        let key = IssuedActionKey::from(&input);
        self.service
            .lock()
            .await
            .submit(&key, input.payload, input.output_refs)
            .map(Json)
    }

    /// Append Evidence for an issued record-evidence Action.
    #[tool(
        name = "adf_add_evidence",
        description = "Append an Evidence Record bound to the issued evidence Action.",
        annotations(
            title = "Agentic Add Evidence",
            read_only_hint = false,
            destructive_hint = false
        )
    )]
    async fn adf_add_evidence(
        &self,
        Parameters(input): Parameters<EvidenceToolInput>,
    ) -> Result<Json<RecordWriteResponse>, ServiceError> {
        let key = IssuedActionKey::from(&input);
        self.service
            .lock()
            .await
            .add_evidence(&key, input.evidence)
            .map(Json)
    }

    /// Record a Decision bound to an issued Human-decision recording Action.
    #[tool(
        name = "adf_apply_decision",
        description = "Apply a Decision that resolves a recorded Human answer.",
        annotations(
            title = "Agentic Apply Decision",
            read_only_hint = false,
            destructive_hint = true
        )
    )]
    async fn adf_apply_decision(
        &self,
        Parameters(input): Parameters<DecisionToolInput>,
    ) -> Result<Json<RecordWriteResponse>, ServiceError> {
        let key = IssuedActionKey::from(&input);
        let expected_digest = parse_expected_digest(&input.expected_digest)?;
        self.service
            .lock()
            .await
            .apply_decision(&key, input.decision, expected_digest)
            .map(Json)
    }

    /// Update a Contract bound to an issued Human-decision recording Action.
    #[tool(
        name = "adf_apply_contract",
        description = "Apply a Contract update using either whole-record or clause-scoped optimistic concurrency control.",
        annotations(
            title = "Agentic Apply Contract",
            read_only_hint = false,
            destructive_hint = true
        )
    )]
    async fn adf_apply_contract(
        &self,
        Parameters(input): Parameters<ContractToolInput>,
    ) -> Result<Json<RecordWriteResponse>, ServiceError> {
        let key = IssuedActionKey::from(&input);
        let expected_digest = parse_expected_digest(&input.expected_digest)?;
        let expected_clause_digests = input.expected_clause_digests.as_ref();
        self.service
            .lock()
            .await
            .apply_contract(
                &key,
                input.contract,
                expected_digest,
                expected_clause_digests,
            )
            .map(Json)
    }

    /// Abandon one unsubmitted Action in local MCP session memory.
    #[tool(
        name = "adf_abandon_action",
        description = "Forget one unsubmitted Action without changing Project Records.",
        annotations(
            title = "Agentic Abandon Action",
            read_only_hint = false,
            destructive_hint = false
        )
    )]
    async fn adf_abandon_action(
        &self,
        Parameters(input): Parameters<ActionToolInput>,
    ) -> Json<AbandonActionResponse> {
        let key = IssuedActionKey::from(&input);
        Json(self.service.lock().await.abandon(&key))
    }
}

#[tool_handler(
    router = self.tool_router,
    name = "adf",
    version = "0.1.0",
    instructions = "Use adf_next before write tools. Submit only Results for Actions issued by this server session."
)]
impl ServerHandler for AgenticMcpServer {}

fn parse_expected_digest(value: &Value) -> Result<Option<&str>, ServiceError> {
    match value {
        Value::Null => Ok(None),
        Value::String(value) => Ok(Some(value)),
        _ => Err(ServiceError::invalid_argument(
            "expected_digest must be a string or null",
        )),
    }
}

pub fn run_stdio_server(
    project_root: impl AsRef<Path>,
    release_root: Option<PathBuf>,
) -> Result<(), String> {
    let service = ProjectApplicationService::new(project_root, release_root)
        .map_err(|error| error.to_string())?;
    let server = AgenticMcpServer::new(service);
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|error| error.to_string())?;
    runtime.block_on(async move {
        server
            .serve(rmcp::transport::stdio())
            .await
            .map_err(|error| error.to_string())?
            .waiting()
            .await
            .map(|_| ())
            .map_err(|error| error.to_string())
    })
}
