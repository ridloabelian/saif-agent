use crate::catalog::ResolvedRoute;
use crate::transport::{classify_http_error, LlmTransport};
use crate::types::{
    LlmRequest, LlmResponse, MessageRole, ProviderError, StreamChunk, ToolCall, ToolDefinition,
    UsageMetrics,
};
use async_trait::async_trait;
use futures_util::StreamExt;
use regex::Regex;
use reqwest::Client;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashSet};
use std::sync::OnceLock;
use tokio::sync::mpsc;

fn cron_session_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^(cron_.+)_\d{8}_\d{6}$").unwrap_or_else(|_| unreachable!()))
}

/// Normalize physical cron session ids into a stable cache scope (`agent/transports/codex.py`).
pub fn cache_scope_from_session_id(session_id: &str) -> String {
    if let Some(caps) = cron_session_re().captures(session_id) {
        if let Some(m) = caps.get(1) {
            return m.as_str().to_string();
        }
    }
    session_id.to_string()
}

/// Bound `prompt_cache_key` to <= 64 chars without changing logical session identity.
pub fn bounded_prompt_cache_key(val: Option<&str>) -> Option<String> {
    let key = val.unwrap_or("").trim();
    if key.is_empty() {
        return None;
    }
    if key.chars().count() <= 64 {
        Some(key.to_string())
    } else {
        let digest = hex::encode(Sha256::digest(key.as_bytes()));
        Some(format!("pck_{}", &digest[..24]))
    }
}

/// Determine provider-reserved function names from `(provider_id, base_url)`.
pub fn reserved_names_for_endpoint(provider_id: &str, base_url: &str) -> Vec<&'static str> {
    let host = url::Url::parse(base_url)
        .ok()
        .and_then(|u| u.host_str().map(|s| s.to_ascii_lowercase()))
        .unwrap_or_default();
    let pid = provider_id.to_ascii_lowercase();

    if host == "api.perplexity.ai" {
        return vec![
            "web_search",
            "search_files",
            "fetch_url",
            "people_search",
            "finance_search",
        ];
    }
    if host == "opencode.ai" || pid.starts_with("opencode") {
        return vec!["web_search", "search_files"];
    }
    if host == "api.x.ai" {
        return vec!["tool_search", "web_search"];
    }
    if matches!(host.as_str(), "api.openai.com" | "chatgpt.com")
        || matches!(pid.as_str(), "openai" | "openai-codex")
    {
        return vec!["tool_search"];
    }
    Vec::new()
}

/// Alias reserved function names (`<name>` -> `hermes_<name>`, collision-safe) for Responses APIs.
pub fn alias_reserved_tools(
    tools: &[ToolDefinition],
    reserved_names: &[&str],
) -> (Vec<ToolDefinition>, BTreeMap<String, String>) {
    let mut taken: HashSet<String> = tools.iter().map(|t| t.name.clone()).collect();
    let mut rewritten = Vec::with_capacity(tools.len());
    let mut alias_map = BTreeMap::new();

    for tool in tools {
        if !reserved_names.contains(&tool.name.as_str()) {
            rewritten.push(tool.clone());
            continue;
        }
        let base = format!("hermes_{}", tool.name);
        let mut alias = base.clone();
        let mut suffix = 2usize;
        while taken.contains(&alias) {
            alias = format!("{base}_{suffix}");
            suffix += 1;
        }
        taken.insert(alias.clone());
        alias_map.insert(alias.clone(), tool.name.clone());
        let mut cloned = tool.clone();
        cloned.name = alias;
        rewritten.push(cloned);
    }

    (rewritten, alias_map)
}

fn resolve_alias(name: &str, alias_map: &BTreeMap<String, String>) -> String {
    if let Some(orig) = alias_map.get(name) {
        return orig.clone();
    }
    if let Some(stripped) = name.strip_prefix("hermes_") {
        if matches!(
            stripped,
            "web_search"
                | "search_files"
                | "fetch_url"
                | "people_search"
                | "finance_search"
                | "tool_search"
        ) {
            return stripped.to_string();
        }
    }
    name.to_string()
}

#[derive(Clone)]
pub struct CodexResponsesTransport {
    client: Client,
}

impl Default for CodexResponsesTransport {
    fn default() -> Self {
        Self::new()
    }
}

impl CodexResponsesTransport {
    pub fn new() -> Self {
        Self {
            client: Client::new(),
        }
    }

    pub fn responses_url(base_url: &str) -> String {
        let trimmed = base_url.trim_end_matches('/');
        if trimmed.ends_with("/responses") {
            trimmed.to_string()
        } else if trimmed.ends_with("/v1") || trimmed.ends_with("/codex") {
            format!("{trimmed}/responses")
        } else {
            format!("{trimmed}/v1/responses")
        }
    }

    pub fn build_payload(
        &self,
        route: &ResolvedRoute,
        req: &LlmRequest,
    ) -> (Value, BTreeMap<String, String>) {
        let reserved = reserved_names_for_endpoint(&route.provider_id, &route.base_url);
        let (wire_tools, alias_map) = alias_reserved_tools(&req.tools, &reserved);
        let reverse_to_wire: BTreeMap<String, String> = alias_map
            .iter()
            .map(|(wire, orig)| (orig.clone(), wire.clone()))
            .collect();

        let mut instructions = Vec::new();
        let mut input = Vec::new();

        for msg in &req.messages {
            match msg.role {
                MessageRole::System => {
                    if !msg.content.trim().is_empty() {
                        instructions.push(msg.content.as_str());
                    }
                }
                MessageRole::User => {
                    input.push(json!({
                        "type": "message",
                        "role": "user",
                        "content": [{ "type": "input_text", "text": msg.content }]
                    }));
                }
                MessageRole::Assistant => {
                    if !msg.content.is_empty() {
                        input.push(json!({
                            "type": "message",
                            "role": "assistant",
                            "content": [{ "type": "output_text", "text": msg.content }]
                        }));
                    }
                    if let Some(tcs) = &msg.tool_calls {
                        for tc in tcs {
                            let wire_name = reverse_to_wire
                                .get(&tc.name)
                                .cloned()
                                .unwrap_or_else(|| tc.name.clone());
                            input.push(json!({
                                "type": "function_call",
                                "call_id": tc.id,
                                "name": wire_name,
                                "arguments": serde_json::to_string(&tc.arguments).unwrap_or_else(|_| "{}".to_string())
                            }));
                        }
                    }
                }
                MessageRole::Tool => {
                    input.push(json!({
                        "type": "function_call_output",
                        "call_id": msg.tool_call_id.clone().unwrap_or_default(),
                        "output": msg.content
                    }));
                }
            }
        }

        let mut body = serde_json::Map::new();
        body.insert("model".into(), json!(req.model));
        if !instructions.is_empty() {
            body.insert("instructions".into(), json!(instructions.join("\n\n")));
        }
        body.insert("input".into(), Value::Array(input));
        body.insert("stream".into(), json!(req.stream));

        if !wire_tools.is_empty() {
            let encoded: Vec<Value> = wire_tools
                .iter()
                .map(|t| {
                    json!({
                        "type": "function",
                        "name": t.name,
                        "description": t.description,
                        "parameters": t.parameters
                    })
                })
                .collect();
            body.insert("tools".into(), Value::Array(encoded));
        }

        if let Some(sid) = req.session_id.as_deref() {
            let scope = cache_scope_from_session_id(sid);
            if let Some(cache_key) = bounded_prompt_cache_key(Some(&scope)) {
                body.insert("prompt_cache_key".into(), json!(cache_key));
            }
        }
        if let Some(effort) = &req.reasoning_effort {
            body.insert("reasoning".into(), json!({ "effort": effort }));
        }
        if !route.omit_temperature {
            if let Some(temp) = req.temperature {
                body.insert("temperature".into(), json!(temp));
            }
        }
        if let Some(max_tok) = req.max_tokens {
            body.insert("max_output_tokens".into(), json!(max_tok));
        }

        (Value::Object(body), alias_map)
    }
}

#[async_trait]
impl LlmTransport for CodexResponsesTransport {
    async fn complete(
        &self,
        route: &ResolvedRoute,
        request: &LlmRequest,
        stream_tx: Option<mpsc::UnboundedSender<StreamChunk>>,
    ) -> Result<LlmResponse, ProviderError> {
        let url = Self::responses_url(&route.base_url);
        let (payload, alias_map) = self.build_payload(route, request);

        let mut builder = self
            .client
            .post(url)
            .header("User-Agent", "saif-cli/0.1.0")
            .json(&payload);
        if let Some(key) = &route.api_key {
            builder = builder.bearer_auth(key);
        }

        let resp = builder.send().await?;
        let status = resp.status();
        if !status.is_success() {
            let retry_after = resp
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.parse::<u64>().ok());
            let body = resp.text().await.unwrap_or_default();
            return Err(classify_http_error(status.as_u16(), retry_after, body));
        }

        if !request.stream {
            let json_body: Value = resp.json().await?;
            let parsed = parse_responses_json(&json_body, &alias_map)?;
            if let Some(tx) = stream_tx {
                let _ = tx.send(StreamChunk::Done(parsed.clone()));
            }
            return Ok(parsed);
        }

        let mut content = String::new();
        let mut reasoning = String::new();
        let mut tool_calls = Vec::new();
        let mut finish_reason: Option<String> = None;
        let mut usage = UsageMetrics::default();

        let mut buffer = String::new();
        let mut byte_stream = resp.bytes_stream();
        while let Some(chunk_res) = byte_stream.next().await {
            let bytes = chunk_res?;
            buffer.push_str(&String::from_utf8_lossy(&bytes));
            while let Some(pos) = buffer.find('\n') {
                let line = buffer[..pos].trim_end_matches('\r').to_string();
                buffer.drain(..=pos);
                let Some(data) = line.strip_prefix("data:") else {
                    continue;
                };
                let data = data.trim();
                if data.is_empty() || data == "[DONE]" {
                    continue;
                }
                let Ok(v) = serde_json::from_str::<Value>(data) else {
                    continue;
                };
                match v.get("type").and_then(|t| t.as_str()) {
                    Some("response.output_text.delta") => {
                        if let Some(delta) = v.get("delta").and_then(|d| d.as_str()) {
                            content.push_str(delta);
                            if let Some(tx) = &stream_tx {
                                let _ = tx.send(StreamChunk::ContentDelta(delta.to_string()));
                            }
                        }
                    }
                    Some("response.reasoning_summary_text.delta") => {
                        if let Some(delta) = v.get("delta").and_then(|d| d.as_str()) {
                            reasoning.push_str(delta);
                            if let Some(tx) = &stream_tx {
                                let _ = tx.send(StreamChunk::ReasoningDelta(delta.to_string()));
                            }
                        }
                    }
                    Some("response.output_item.done") => {
                        if let Some(item) = v.get("item") {
                            if item.get("type").and_then(|t| t.as_str()) == Some("function_call") {
                                if let Some(tc) = parse_response_function_call(item, &alias_map) {
                                    tool_calls.push(tc);
                                }
                            }
                        }
                    }
                    Some("response.completed") => {
                        if let Some(resp_obj) = v.get("response") {
                            usage = parse_responses_usage(resp_obj.get("usage"));
                            finish_reason = resp_obj
                                .get("status")
                                .and_then(|s| s.as_str())
                                .map(ToString::to_string);
                            if content.is_empty() && tool_calls.is_empty() {
                                if let Ok(full) = parse_responses_json(resp_obj, &alias_map) {
                                    content = full.content;
                                    tool_calls = full.tool_calls;
                                    if full.reasoning.is_some() {
                                        reasoning = full.reasoning.unwrap_or_default();
                                    }
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
        }

        if finish_reason.is_none() {
            finish_reason = Some(if tool_calls.is_empty() {
                "stop".to_string()
            } else {
                "tool_calls".to_string()
            });
        }

        let response = LlmResponse {
            content,
            reasoning: (!reasoning.is_empty()).then_some(reasoning),
            tool_calls,
            finish_reason,
            usage,
        };
        if let Some(tx) = stream_tx {
            let _ = tx.send(StreamChunk::Done(response.clone()));
        }
        Ok(response)
    }
}

fn parse_response_function_call(
    item: &Value,
    alias_map: &BTreeMap<String, String>,
) -> Option<ToolCall> {
    let raw_name = item.get("name").and_then(|v| v.as_str())?;
    let call_id = item
        .get("call_id")
        .or_else(|| item.get("id"))
        .and_then(|v| v.as_str())
        .unwrap_or("call_0")
        .to_string();
    let args_val = match item.get("arguments") {
        Some(Value::String(s)) => serde_json::from_str(s).unwrap_or_else(|_| json!({ "raw": s })),
        Some(other) => other.clone(),
        None => json!({}),
    };
    Some(ToolCall {
        id: call_id,
        name: resolve_alias(raw_name, alias_map),
        arguments: args_val,
    })
}

fn parse_responses_usage(u: Option<&Value>) -> UsageMetrics {
    let Some(u) = u else {
        return UsageMetrics::default();
    };
    UsageMetrics {
        input_tokens: u.get("input_tokens").and_then(|v| v.as_i64()).unwrap_or(0),
        output_tokens: u.get("output_tokens").and_then(|v| v.as_i64()).unwrap_or(0),
        cache_read_tokens: u
            .pointer("/input_tokens_details/cached_tokens")
            .and_then(|v| v.as_i64())
            .unwrap_or(0),
        cache_write_tokens: 0,
        reasoning_tokens: u
            .pointer("/output_tokens_details/reasoning_tokens")
            .and_then(|v| v.as_i64())
            .unwrap_or(0),
    }
}

pub fn parse_responses_json(
    v: &Value,
    alias_map: &BTreeMap<String, String>,
) -> Result<LlmResponse, ProviderError> {
    let mut content = String::new();
    let mut reasoning = String::new();
    let mut tool_calls = Vec::new();

    if let Some(items) = v.get("output").and_then(|o| o.as_array()) {
        for item in items {
            match item.get("type").and_then(|t| t.as_str()) {
                Some("message") => {
                    if let Some(parts) = item.get("content").and_then(|c| c.as_array()) {
                        for p in parts {
                            if let Some(t) = p.get("text").and_then(|x| x.as_str()) {
                                content.push_str(t);
                            }
                        }
                    }
                }
                Some("reasoning") => {
                    if let Some(parts) = item.get("summary").and_then(|c| c.as_array()) {
                        for p in parts {
                            if let Some(t) = p.get("text").and_then(|x| x.as_str()) {
                                reasoning.push_str(t);
                            }
                        }
                    }
                }
                Some("function_call") => {
                    if let Some(tc) = parse_response_function_call(item, alias_map) {
                        tool_calls.push(tc);
                    }
                }
                _ => {}
            }
        }
    }

    let finish_reason = if !tool_calls.is_empty() {
        Some("tool_calls".to_string())
    } else {
        v.get("status")
            .and_then(|s| s.as_str())
            .map(|s| if s == "completed" { "stop" } else { s })
            .map(ToString::to_string)
    };

    Ok(LlmResponse {
        content,
        reasoning: (!reasoning.is_empty()).then_some(reasoning),
        tool_calls,
        finish_reason,
        usage: parse_responses_usage(v.get("usage")),
    })
}
