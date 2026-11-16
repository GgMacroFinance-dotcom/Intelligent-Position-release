//! MCP 服务端：把组织工具开放给 Codex 这类外部引擎（`docs/ip/m2-spec.md` D-17 到 D-19）。
//!
//! 外部引擎有自己的工具循环，调不到 ip 的 `create_task`、`ask_human` 等工具，于是 ip 开一个
//! Streamable HTTP 的 MCP 服务端让它连进来。每个请求都要带一个“本轮令牌”
//! （`Authorization: Bearer <令牌>`）：令牌由运行时在引擎开始一轮时签发，绑定了是哪个员工、
//! 在哪个对话。用哪些工具、以谁的身份执行，都由令牌决定，引擎冒充不了别的员工。
//!
//! 本模块只管 MCP 协议这一层；令牌怎么签发、工具怎么执行由 [`TurnTools`] 的实现方
//! （运行时）决定。服务是无状态的：不发会话 id，每个请求单独处理。

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use ip_models::ToolSpec;
use ip_tools::ToolOutput;
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, ListToolsResult, PaginatedRequestParams,
    ServerCapabilities, ServerConfig, Tool,
};
use rmcp::service::{RequestContext, RoleServer};
use rmcp::transport::streamable_http_server::session::never::NeverSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use rmcp::{ErrorData, ServerHandler};

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// 按本轮令牌提供工具。由运行时实现。
pub trait TurnTools: Send + Sync + 'static {
    /// 令牌有效时返回这一轮能用的工具；无效（不存在或已过期）时返回 `None`。
    fn specs(&self, token: &str) -> Option<Vec<ToolSpec>>;

    /// 以令牌对应的员工、在令牌对应的对话里执行一个工具；令牌无效时返回 `None`。
    /// `arguments` 是 JSON 对象的文本，和模型给出的工具参数格式一样。
    fn call<'a>(&'a self, token: &'a str, name: &'a str, arguments: String) -> BoxFuture<'a, Option<ToolOutput>>;
}

/// 挂在 `/mcp` 上的服务。只接受本机的 `Host`（rmcp 默认的 DNS rebinding 防护），
/// 外部引擎和 ip 跑在同一台机器上，用 `http://127.0.0.1:<端口>/mcp` 连接。
pub fn http_service(tools: Arc<dyn TurnTools>) -> StreamableHttpService<Gateway, NeverSessionManager> {
    let config = StreamableHttpServerConfig::default()
        .with_legacy_session_mode(false)
        .with_json_response(true);
    StreamableHttpService::new(
        move || Ok(Gateway { tools: tools.clone() }),
        Arc::new(NeverSessionManager::default()),
        config,
    )
}

/// 从 `Authorization: Bearer <令牌>` 里取出令牌。
pub fn bearer(headers: &http::HeaderMap) -> Option<&str> {
    let value = headers.get(http::header::AUTHORIZATION)?.to_str().ok()?;
    let (scheme, token) = value.split_once(' ')?;
    let token = token.trim();
    (scheme.eq_ignore_ascii_case("bearer") && !token.is_empty()).then_some(token)
}

/// 每个 MCP 请求的处理者。
#[derive(Clone)]
pub struct Gateway {
    tools: Arc<dyn TurnTools>,
}

fn token(context: &RequestContext<RoleServer>) -> Result<&str, ErrorData> {
    context
        .extensions
        .get::<http::request::Parts>()
        .and_then(|parts| bearer(&parts.headers))
        .ok_or_else(|| ErrorData::invalid_request("缺少本轮令牌（Authorization: Bearer）", None))
}

fn expired() -> ErrorData {
    ErrorData::invalid_request("本轮令牌无效或已过期", None)
}

fn to_tool(spec: ToolSpec) -> Tool {
    let schema = match spec.parameters {
        serde_json::Value::Object(map) => map,
        _ => serde_json::Map::new(),
    };
    Tool::new(spec.name, spec.description, Arc::new(schema))
}

impl ServerHandler for Gateway {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        let specs = self.tools.specs(token(&context)?).ok_or_else(expired)?;
        Ok(ListToolsResult::with_all_items(
            specs.into_iter().map(to_tool).collect(),
        ))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let token = token(&context)?;
        let arguments = request
            .arguments
            .map_or_else(|| "{}".to_string(), |map| serde_json::Value::Object(map).to_string());
        let out = self
            .tools
            .call(token, &request.name, arguments)
            .await
            .ok_or_else(expired)?;
        let content = vec![ContentBlock::text(out.output)];
        let result = if out.ok {
            CallToolResult::success(content)
        } else {
            CallToolResult::error(content)
        };
        Ok(result.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bearer_parsing() {
        let mut headers = http::HeaderMap::new();
        assert_eq!(bearer(&headers), None);
        headers.insert(http::header::AUTHORIZATION, "Bearer abc".parse().unwrap());
        assert_eq!(bearer(&headers), Some("abc"));
        headers.insert(http::header::AUTHORIZATION, "bearer  abc ".parse().unwrap());
        assert_eq!(bearer(&headers), Some("abc"));
        headers.insert(http::header::AUTHORIZATION, "Basic abc".parse().unwrap());
        assert_eq!(bearer(&headers), None);
        headers.insert(http::header::AUTHORIZATION, "Bearer ".parse().unwrap());
        assert_eq!(bearer(&headers), None);
    }
}
