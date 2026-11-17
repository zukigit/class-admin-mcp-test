mod admin_tools;
mod public_tools;

use std::collections::HashSet;
use std::sync::Arc;

use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::tool::ToolCallContext;
use rmcp::model::{
    CacheScope, CallToolRequestParams, CallToolResponse, CallToolResult, Implementation, ListToolsResult,
    PaginatedRequestParams, ResultType, ServerCapabilities, ServerConfig,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData, RoleServer, ServerHandler};
use serde::Serialize;
use serde_json::json;
use sqlx::SqlitePool;

use crate::auth::Caller;
use crate::config::BookingArgs;
use crate::error::{AppError, AppResult};
use crate::rate_limit::Limiters;

/// Shared state for all requests.
pub struct AppState {
    pub pool: SqlitePool,
    pub booking: BookingArgs,
    pub limiters: Limiters,
    /// `None`: admin login is switched off (no secret configured).
    pub jwt_secret: Option<Vec<u8>>,
}

#[derive(Clone)]
pub struct McpServer {
    state: Arc<AppState>,
    tool_router: ToolRouter<Self>,
    admin_tool_names: Arc<HashSet<String>>,
}

/// Built once and cloned for every request.
#[derive(Clone)]
pub struct McpServerFactory {
    template: McpServer,
}

impl McpServerFactory {
    pub fn new(state: Arc<AppState>) -> Self {
        let admin = McpServer::admin_tools();
        let admin_tool_names = admin.list_all().into_iter().map(|t| t.name.to_string()).collect();
        let mut tool_router = McpServer::public_tools();
        tool_router.merge(admin);
        Self {
            template: McpServer {
                state,
                tool_router,
                admin_tool_names: Arc::new(admin_tool_names),
            },
        }
    }

    pub fn create(&self) -> McpServer {
        self.template.clone()
    }
}

const INSTRUCTIONS: &str = "Class booking server for a school. \
Anyone can browse published courses and upcoming classes (fees in USD, spots left) and book a seat \
with a name and email. New bookings are pending until the school approves them. \
Keep the booking reference and cancel code returned by book_class: they are needed to check or cancel the booking. \
Admin tools are only available with an admin token.";

impl McpServer {
    fn caller_from(ctx: &RequestContext<RoleServer>) -> Caller {
        ctx.extensions
            .get::<http::request::Parts>()
            .and_then(|p| p.extensions.get::<Caller>().cloned())
            .unwrap_or_else(|| Caller::public("unknown".into()))
    }

    pub(crate) fn caller(parts: &http::request::Parts) -> Caller {
        parts
            .extensions
            .get::<Caller>()
            .cloned()
            .unwrap_or_else(|| Caller::public("unknown".into()))
    }
}

/// Turns a result into a tool reply. Errors are tool errors (`isError: true`)
/// with a safe message, so the agent can read and explain them.
pub(crate) fn reply<T: Serialize>(result: AppResult<T>) -> Result<CallToolResult, ErrorData> {
    match result {
        Ok(v) => {
            let value = serde_json::to_value(v).map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
            Ok(CallToolResult::structured(value))
        }
        Err(e) => Ok(error_reply(&e)),
    }
}

pub(crate) fn error_reply(e: &AppError) -> CallToolResult {
    CallToolResult::structured_error(json!({ "error": e.public_message() }))
}

impl ServerHandler for McpServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("class-admin", env!("CARGO_PKG_VERSION")))
            .with_instructions(INSTRUCTIONS)
    }

    /// Layer 1: admin tools are listed only for admins.
    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        let is_admin = Self::caller_from(&context).is_admin();
        let tools = self
            .tool_router
            .list_all()
            .into_iter()
            .filter(|t| is_admin || !self.admin_tool_names.contains(t.name.as_ref()))
            .collect();
        let supports_cache_hints = context
            .protocol_version()
            .is_some_and(|v| v >= rmcp::model::ProtocolVersion::V_2026_07_28);
        Ok(ListToolsResult {
            result_type: Some(ResultType::COMPLETE),
            tools,
            meta: None,
            next_cursor: None,
            ttl_ms: supports_cache_hints.then_some(0),
            // The list depends on who asks, so it must not be shared between users.
            cache_scope: supports_cache_hints.then_some(CacheScope::Private),
        })
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        // Extra check before dispatch; each admin tool also calls `require_admin` itself.
        if self.admin_tool_names.contains(request.name.as_ref()) && !Self::caller_from(&context).is_admin() {
            return Ok(error_reply(&AppError::Forbidden).into());
        }
        let tcc = ToolCallContext::new(self, request, context);
        self.tool_router.call(tcc).await
    }

    fn get_tool(&self, name: &str) -> Option<rmcp::model::Tool> {
        self.tool_router.get(name).cloned()
    }
}
