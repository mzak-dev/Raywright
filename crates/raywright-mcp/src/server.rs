use std::sync::Arc;

use raywright::Raywright;
use rmcp::ErrorData as McpError;
use rmcp::model::{CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, Implementation, ListToolsResult, PaginatedRequestParams, ServerCapabilities, ServerConfig, Tool};
use rmcp::service::RequestContext;
use rmcp::{RoleServer, ServerHandler, ServiceExt};
use tokio::sync::Mutex;

use crate::policy::Policy;
use crate::session::Session;
use crate::tools::{McpTool, builtins};

/// The server: tools around one [`Session`].
#[derive(Clone)]
pub struct RaywrightServer {
    tools: Arc<Vec<Arc<dyn McpTool>>>,
    session: Arc<Mutex<Session>>,
}

const INSTRUCTIONS: &str = "Raywright drives desktop apps. Start with app_launch (a profile or an allowed program), then read the window with snapshot: every element has a ref (e1, e2...) to pass to click, type, fill, press_key, expect, screenshot. Actions wait for their element and return a fresh snapshot; refs only hold until the next one. To look at an animation, use record_action. Finish with generate_test to turn the session into a Rust test.";

impl RaywrightServer {
    pub fn new(rw: Raywright, policy: Policy) -> Self {
        RaywrightServer { tools: Arc::new(builtins()), session: Arc::new(Mutex::new(Session::new(rw, policy))) }
    }

    /// Adds a tool (replacing a built-in of the same name).
    pub fn tool(mut self, t: Arc<dyn McpTool>) -> Self {
        let mut v: Vec<Arc<dyn McpTool>> = self.tools.iter().filter(|x| x.name() != t.name()).cloned().collect();
        v.push(t);
        self.tools = Arc::new(v);
        self
    }

    pub fn tool_names(&self) -> Vec<String> {
        self.tools.iter().map(|t| t.name().to_string()).collect()
    }

    /// Serves over stdin and stdout until the client goes away.
    pub async fn serve_stdio(self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let session = self.session.clone();
        let running = self.serve(rmcp::transport::stdio()).await?;
        running.waiting().await?;
        let s = session.lock().await;
        for app in &s.apps {
            let _ = app.close().await;
        }
        Ok(())
    }
}

impl ServerHandler for RaywrightServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build()).with_server_info(Implementation::new("raywright", env!("CARGO_PKG_VERSION"))).with_instructions(INSTRUCTIONS)
    }

    async fn list_tools(&self, _request: Option<PaginatedRequestParams>, _context: RequestContext<RoleServer>) -> Result<ListToolsResult, McpError> {
        let tools = self
            .tools
            .iter()
            .map(|t| {
                let schema = match t.schema() {
                    serde_json::Value::Object(m) => m,
                    _ => serde_json::Map::new(),
                };
                Tool::new(t.name().to_string(), t.description().to_string(), Arc::new(schema))
            })
            .collect();
        Ok(ListToolsResult::with_all_items(tools))
    }

    async fn call_tool(&self, request: CallToolRequestParams, _context: RequestContext<RoleServer>) -> Result<CallToolResponse, McpError> {
        let Some(tool) = self.tools.iter().find(|t| t.name() == request.name).cloned() else {
            return Err(McpError::invalid_params(format!("no tool `{}`", request.name), None));
        };
        let args = serde_json::Value::Object(request.arguments.unwrap_or_default());
        let mut s = self.session.lock().await;
        s.log(&format!("tool {} {args}", request.name));
        let result = match tool.call(&mut s, args).await {
            Ok(content) => CallToolResult::success(content),
            Err(msg) => {
                s.log(&format!("error {msg}"));
                CallToolResult::error(vec![ContentBlock::text(msg)])
            }
        };
        Ok(result.into())
    }
}
