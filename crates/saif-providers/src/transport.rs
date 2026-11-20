use crate::catalog::ResolvedRoute;
use crate::credential_pool::{is_terminal_auth_reason, CredentialPool};
use crate::types::{
    LlmRequest, LlmResponse, MessageRole, ProviderError, StreamChunk, ToolCall, UsageMetrics,
};
use async_trait::async_trait;
use futures_util::StreamExt;
use reqwest::Client;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::mpsc;

#[async_trait]
pub trait LlmTransport: Send + Sync {
    async fn complete(
        &self,
        route: &ResolvedRoute,
        request: &LlmRequest,
        stream_tx: Option<mpsc::UnboundedSender<StreamChunk>>,
    ) -> Result<LlmResponse, ProviderError>;
}

#[derive(Clone)]
pub struct OpenAiChatTransport {
    client: Client,
}

impl Default for OpenAiChatTransport {
    fn default() -> Self {
        Self::new()
    }
}

impl OpenAiChatTransport {
    pub fn new() -> Self {
        Self {
            client: Client::new(),
        }
    }

    fn build_payload(&self, route: &ResolvedRoute, req: &LlmRequest) -> Value {
        let messages: Vec<Value> = req
            .messages
            .iter()
            .map(|m| {
                let role = match m.role {
                    MessageRole::System => "system",
                    MessageRole::User => "user",
                    MessageRole::Assistant => "assistant",
                    MessageRole::Tool => "tool",
                };
                let mut obj = serde_json::Map::new();
                obj.insert("role".into(), json!(role));
                obj.insert("content".into(), json!(m.content));
                if let Some(id) = &m.tool_call_id {
                    obj.insert("tool_call_id".into(), json!(id));
                }
                if let Some(name) = &m.tool_name {
                    obj.insert("name".into(), json!(name));
                }
                if let Some(tcs) = &m.tool_calls {
                    let encoded: Vec<Value> = tcs
                        .iter()
                        .map(|tc| {
                            json!({
                                "id": tc.id,
                                "type": "function",
                                "function": {
                                    "name": tc.name,
                                    "arguments": serde_json::to_string(&tc.arguments).unwrap_or_else(|_| "{}".to_string()),
                                }
                            })
                        })
                        .collect();
                    obj.insert("tool_calls".into(), Value::Array(encoded));
                }
                Value::Object(obj)
            })
            .collect();

        let mut body = serde_json::Map::new();
        body.insert("model".into(), json!(req.model));
        body.insert("messages".into(), Value::Array(messages));
        body.insert("stream".into(), json!(req.stream));
        if req.stream {
            body.insert("stream_options".into(), json!({ "include_usage": true }));
        }
        if !req.tools.is_empty() {
            let tools: Vec<Value> = req
                .tools
                .iter()
                .map(|t| {
                    json!({
                        "type": "function",
                        "function": {
                            "name": t.name,
                            "description": t.description,
                            "parameters": t.parameters,
                        }
                    })
                })
                .collect();
            body.insert("tools".into(), Value::Array(tools));
        }
        if !route.omit_temperature {
            if let Some(temp) = req.temperature {
                body.insert("temperature".into(), json!(temp));
            }
        }
        if let Some(max_tok) = req.max_tokens {
            body.insert("max_tokens".into(), json!(max_tok));
        }
        if let Some(effort) = &req.reasoning_effort {
            body.insert("reasoning_effort".into(), json!(effort));
        }
        Value::Object(body)
    }
}

#[async_trait]
impl LlmTransport for OpenAiChatTransport {
    async fn complete(
        &self,
        route: &ResolvedRoute,
        request: &LlmRequest,
        stream_tx: Option<mpsc::UnboundedSender<StreamChunk>>,
    ) -> Result<LlmResponse, ProviderError> {
        let url = format!("{}/chat/completions", route.base_url);
        let payload = self.build_payload(route, request);
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
            let parsed = parse_openai_nonstream(&json_body)?;
            if let Some(tx) = stream_tx {
                let _ = tx.send(StreamChunk::Done(parsed.clone()));
            }
            return Ok(parsed);
        }

        let mut content = String::new();
        let mut reasoning = String::new();
        let mut finish_reason: Option<String> = None;
        let mut usage = UsageMetrics::default();
        let mut tc_acc: BTreeMap<usize, (String, String, String)> = BTreeMap::new();

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
                if let Some(u) = v.get("usage") {
                    usage = parse_openai_usage(u);
                }
                if let Some(choice) = v.get("choices").and_then(|c| c.get(0)) {
                    if let Some(fr) = choice.get("finish_reason").and_then(|x| x.as_str()) {
                        finish_reason = Some(fr.to_string());
                    }
                    if let Some(delta) = choice.get("delta") {
                        if let Some(txt) = delta.get("content").and_then(|x| x.as_str()) {
                            content.push_str(txt);
                            if let Some(tx) = &stream_tx {
                                let _ = tx.send(StreamChunk::ContentDelta(txt.to_string()));
                            }
                        }
                        if let Some(r_txt) = delta
                            .get("reasoning")
                            .or_else(|| delta.get("reasoning_content"))
                            .and_then(|x| x.as_str())
                        {
                            reasoning.push_str(r_txt);
                            if let Some(tx) = &stream_tx {
                                let _ = tx.send(StreamChunk::ReasoningDelta(r_txt.to_string()));
                            }
                        }
                        if let Some(tcs) = delta.get("tool_calls").and_then(|x| x.as_array()) {
                            for tc in tcs {
                                let idx =
                                    tc.get("index").and_then(|x| x.as_u64()).unwrap_or(0) as usize;
                                let entry = tc_acc.entry(idx).or_insert_with(|| {
                                    (String::new(), String::new(), String::new())
                                });
                                if let Some(id) = tc.get("id").and_then(|x| x.as_str()) {
                                    entry.0 = id.to_string();
                                }
                                if let Some(func) = tc.get("function") {
                                    if let Some(name) = func.get("name").and_then(|x| x.as_str()) {
                                        entry.1.push_str(name);
                                    }
                                    if let Some(args) =
                                        func.get("arguments").and_then(|x| x.as_str())
                                    {
                                        entry.2.push_str(args);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        let tool_calls = tc_acc
            .into_iter()
            .map(|(idx, (id, name, args_raw))| {
                let id = if id.is_empty() {
                    format!("call_{idx}")
                } else {
                    id
                };
                let arguments = if args_raw.trim().is_empty() {
                    json!({})
                } else {
                    serde_json::from_str(&args_raw).unwrap_or_else(|_| json!({ "raw": args_raw }))
                };
                ToolCall {
                    id,
                    name,
                    arguments,
                }
            })
            .collect();

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

fn parse_openai_usage(u: &Value) -> UsageMetrics {
    UsageMetrics {
        input_tokens: u.get("prompt_tokens").and_then(|v| v.as_i64()).unwrap_or(0),
        output_tokens: u
            .get("completion_tokens")
            .and_then(|v| v.as_i64())
            .unwrap_or(0),
        cache_read_tokens: u
            .pointer("/prompt_tokens_details/cached_tokens")
            .and_then(|v| v.as_i64())
            .unwrap_or(0),
        cache_write_tokens: 0,
        reasoning_tokens: u
            .pointer("/completion_tokens_details/reasoning_tokens")
            .and_then(|v| v.as_i64())
            .unwrap_or(0),
    }
}

fn parse_openai_nonstream(v: &Value) -> Result<LlmResponse, ProviderError> {
    let choice = v
        .get("choices")
        .and_then(|c| c.get(0))
        .ok_or_else(|| ProviderError::Transport("missing choices[0] in response".to_string()))?;
    let msg = choice
        .get("message")
        .ok_or_else(|| ProviderError::Transport("missing message in choice".to_string()))?;
    let content = msg
        .get("content")
        .and_then(|x| x.as_str())
        .unwrap_or_default()
        .to_string();
    let reasoning = msg
        .get("reasoning")
        .or_else(|| msg.get("reasoning_content"))
        .and_then(|x| x.as_str())
        .map(ToString::to_string);
    let mut tool_calls = Vec::new();
    if let Some(arr) = msg.get("tool_calls").and_then(|x| x.as_array()) {
        for (idx, tc) in arr.iter().enumerate() {
            let id = tc
                .get("id")
                .and_then(|x| x.as_str())
                .map(ToString::to_string)
                .unwrap_or_else(|| format!("call_{idx}"));
            let name = tc
                .pointer("/function/name")
                .and_then(|x| x.as_str())
                .unwrap_or_default()
                .to_string();
            let args_raw = tc
                .pointer("/function/arguments")
                .and_then(|x| x.as_str())
                .unwrap_or("{}");
            let arguments =
                serde_json::from_str(args_raw).unwrap_or_else(|_| json!({ "raw": args_raw }));
            tool_calls.push(ToolCall {
                id,
                name,
                arguments,
            });
        }
    }
    let finish_reason = choice
        .get("finish_reason")
        .and_then(|x| x.as_str())
        .map(ToString::to_string);
    let usage = v.get("usage").map(parse_openai_usage).unwrap_or_default();
    Ok(LlmResponse {
        content,
        reasoning,
        tool_calls,
        finish_reason,
        usage,
    })
}

#[derive(Clone)]
pub struct AnthropicMessagesTransport {
    client: Client,
}

impl Default for AnthropicMessagesTransport {
    fn default() -> Self {
        Self::new()
    }
}

impl AnthropicMessagesTransport {
    pub fn new() -> Self {
        Self {
            client: Client::new(),
        }
    }

    fn build_payload(&self, req: &LlmRequest) -> Value {
        let mut system_blocks: Vec<Value> = Vec::new();
        let mut messages: Vec<Value> = Vec::new();

        for msg in &req.messages {
            match msg.role {
                MessageRole::System => {
                    system_blocks.push(json!({
                        "type": "text",
                        "text": msg.content,
                        "cache_control": { "type": "ephemeral" }
                    }));
                }
                MessageRole::User => {
                    messages.push(json!({
                        "role": "user",
                        "content": [{ "type": "text", "text": msg.content }]
                    }));
                }
                MessageRole::Assistant => {
                    let mut blocks = Vec::new();
                    if !msg.content.is_empty() {
                        blocks.push(json!({ "type": "text", "text": msg.content }));
                    }
                    if let Some(tcs) = &msg.tool_calls {
                        for tc in tcs {
                            blocks.push(json!({
                                "type": "tool_use",
                                "id": tc.id,
                                "name": tc.name,
                                "input": tc.arguments
                            }));
                        }
                    }
                    messages.push(json!({ "role": "assistant", "content": blocks }));
                }
                MessageRole::Tool => {
                    let block = json!({
                        "type": "tool_result",
                        "tool_use_id": msg.tool_call_id.clone().unwrap_or_default(),
                        "content": msg.content
                    });
                    // Group consecutive tool results into one user message for strict role alternation.
                    if let Some(last) = messages.last_mut() {
                        if last.get("role").and_then(|r| r.as_str()) == Some("user") {
                            if let Some(arr) =
                                last.get_mut("content").and_then(|c| c.as_array_mut())
                            {
                                arr.push(block);
                                continue;
                            }
                        }
                    }
                    messages.push(json!({ "role": "user", "content": [block] }));
                }
            }
        }

        let mut body = serde_json::Map::new();
        body.insert("model".into(), json!(req.model));
        body.insert("max_tokens".into(), json!(req.max_tokens.unwrap_or(8192)));
        if !system_blocks.is_empty() {
            body.insert("system".into(), Value::Array(system_blocks));
        }
        body.insert("messages".into(), Value::Array(messages));
        if req.stream {
            body.insert("stream".into(), json!(true));
        }
        if !req.tools.is_empty() {
            let mut tools: Vec<Value> = req
                .tools
                .iter()
                .map(|t| {
                    json!({
                        "name": t.name,
                        "description": t.description,
                        "input_schema": t.parameters
                    })
                })
                .collect();
            if let Some(last_tool) = tools.last_mut().and_then(|v| v.as_object_mut()) {
                last_tool.insert("cache_control".into(), json!({ "type": "ephemeral" }));
            }
            body.insert("tools".into(), Value::Array(tools));
        }
        Value::Object(body)
    }
}

#[async_trait]
impl LlmTransport for AnthropicMessagesTransport {
    async fn complete(
        &self,
        route: &ResolvedRoute,
        request: &LlmRequest,
        stream_tx: Option<mpsc::UnboundedSender<StreamChunk>>,
    ) -> Result<LlmResponse, ProviderError> {
        let url = format!("{}/v1/messages", route.base_url);
        let payload = self.build_payload(request);
        let mut builder = self
            .client
            .post(url)
            .header("anthropic-version", "2023-06-01")
            .header("User-Agent", "saif-cli/0.1.0")
            .json(&payload);
        if let Some(key) = &route.api_key {
            builder = builder.header("x-api-key", key);
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

        if request.stream {
            return parse_anthropic_sse_stream(resp, stream_tx).await;
        }

        let json_body: Value = resp.json().await?;
        let mut content = String::new();
        let mut reasoning = String::new();
        let mut tool_calls = Vec::new();
        if let Some(blocks) = json_body.get("content").and_then(|c| c.as_array()) {
            for b in blocks {
                match b.get("type").and_then(|t| t.as_str()) {
                    Some("text") => {
                        if let Some(t) = b.get("text").and_then(|x| x.as_str()) {
                            content.push_str(t);
                        }
                    }
                    Some("thinking") => {
                        if let Some(t) = b.get("thinking").and_then(|x| x.as_str()) {
                            reasoning.push_str(t);
                        }
                    }
                    Some("tool_use") => {
                        tool_calls.push(ToolCall {
                            id: b
                                .get("id")
                                .and_then(|x| x.as_str())
                                .unwrap_or_default()
                                .to_string(),
                            name: b
                                .get("name")
                                .and_then(|x| x.as_str())
                                .unwrap_or_default()
                                .to_string(),
                            arguments: b.get("input").cloned().unwrap_or_else(|| json!({})),
                        });
                    }
                    _ => {}
                }
            }
        }
        let u = json_body.get("usage");
        let usage = UsageMetrics {
            input_tokens: u
                .and_then(|x| x.get("input_tokens"))
                .and_then(|v| v.as_i64())
                .unwrap_or(0),
            output_tokens: u
                .and_then(|x| x.get("output_tokens"))
                .and_then(|v| v.as_i64())
                .unwrap_or(0),
            cache_read_tokens: u
                .and_then(|x| x.get("cache_read_input_tokens"))
                .and_then(|v| v.as_i64())
                .unwrap_or(0),
            cache_write_tokens: u
                .and_then(|x| x.get("cache_creation_input_tokens"))
                .and_then(|v| v.as_i64())
                .unwrap_or(0),
            reasoning_tokens: 0,
        };
        let finish_reason = json_body
            .get("stop_reason")
            .and_then(|x| x.as_str())
            .map(ToString::to_string);

        let parsed = LlmResponse {
            content,
            reasoning: (!reasoning.is_empty()).then_some(reasoning),
            tool_calls,
            finish_reason,
            usage,
        };
        if let Some(tx) = stream_tx {
            let _ = tx.send(StreamChunk::Done(parsed.clone()));
        }
        Ok(parsed)
    }
}

pub(crate) fn classify_http_error(
    status: u16,
    retry_after_secs: Option<u64>,
    body: String,
) -> ProviderError {
    if status == 429 {
        return ProviderError::RateLimited {
            retry_after_secs,
            body,
        };
    }
    let lower = body.to_ascii_lowercase();
    if status == 400
        && (lower.contains("context length")
            || lower.contains("maximum context")
            || lower.contains("too many tokens"))
    {
        return ProviderError::ContextOverflow(body);
    }
    ProviderError::ApiStatus { status, body }
}

/// Unified transport bundle covering OpenAI Chat, Anthropic Messages, Codex Responses, and AWS Bedrock Converse.
#[derive(Clone, Default)]
pub struct TransportRouter {
    pub openai: OpenAiChatTransport,
    pub anthropic: AnthropicMessagesTransport,
    pub codex: crate::codex::CodexResponsesTransport,
    pub bedrock: crate::bedrock::BedrockConverseTransport,
}

impl TransportRouter {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn dispatch(
        &self,
        route: &ResolvedRoute,
        request: &LlmRequest,
        stream_tx: Option<mpsc::UnboundedSender<StreamChunk>>,
    ) -> Result<LlmResponse, ProviderError> {
        match route.api_mode {
            crate::catalog::ApiMode::AnthropicMessages => {
                self.anthropic.complete(route, request, stream_tx).await
            }
            crate::catalog::ApiMode::CodexResponses => {
                self.codex.complete(route, request, stream_tx).await
            }
            crate::catalog::ApiMode::Bedrock => {
                self.bedrock.complete(route, request, stream_tx).await
            }
            crate::catalog::ApiMode::ChatCompletions => {
                self.openai.complete(route, request, stream_tx).await
            }
        }
    }
}

/// Convenience helper for dispatching according to `ResolvedRoute.api_mode`.
pub async fn dispatch_completion(
    router: &TransportRouter,
    route: &ResolvedRoute,
    request: &LlmRequest,
    stream_tx: Option<mpsc::UnboundedSender<StreamChunk>>,
) -> Result<LlmResponse, ProviderError> {
    router.dispatch(route, request, stream_tx).await
}

/// Dispatch completion with automatic `CredentialPool` cooldown & failover rotation on 401/403/429.
pub async fn dispatch_completion_with_failover(
    router: &TransportRouter,
    route: &ResolvedRoute,
    request: &LlmRequest,
    max_attempts: usize,
    stream_tx: Option<mpsc::UnboundedSender<StreamChunk>>,
) -> Result<LlmResponse, ProviderError> {
    let mut active_route = route.clone();
    let attempts_limit = max_attempts.max(1);

    for attempt in 0..attempts_limit {
        let res = router
            .dispatch(&active_route, request, stream_tx.clone())
            .await;
        match res {
            Ok(resp) => return Ok(resp),
            Err(err) => {
                let fail_info: Option<(Option<u16>, String)> = match &err {
                    ProviderError::RateLimited { body, .. } => Some((Some(429), body.clone())),
                    ProviderError::ApiStatus { status, body }
                        if *status == 401 || *status == 403 =>
                    {
                        Some((Some(*status), body.clone()))
                    }
                    _ => None,
                };
                if let (Some((http_status, body)), Some(home), Some(cred_id)) = (
                    fail_info,
                    active_route.saif_home.as_deref(),
                    active_route.pooled_credential_id.as_deref(),
                ) {
                    let now_epoch = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs();
                    let reason = extract_terminal_reason(&body);
                    let mut pool = CredentialPool::open(home);
                    pool.record_failure(
                        &active_route.provider_id,
                        cred_id,
                        http_status,
                        reason.as_deref(),
                        now_epoch,
                    );
                    let next = pool.select(&active_route.provider_id, now_epoch);
                    let _ = pool.save();

                    if attempt + 1 < attempts_limit {
                        if let Some(next_cred) = next {
                            active_route.api_key = Some(next_cred.secret);
                            active_route.pooled_credential_id = Some(next_cred.id);
                            continue;
                        }
                    }
                }
                return Err(err);
            }
        }
    }
    unreachable!()
}

fn extract_terminal_reason(body: &str) -> Option<String> {
    let lower = body.to_ascii_lowercase();
    for marker in [
        "token_invalidated",
        "token_revoked",
        "invalid_token",
        "invalid_grant",
        "unauthorized_client",
        "refresh_token_reused",
    ] {
        if lower.contains(marker) && is_terminal_auth_reason(marker) {
            return Some(marker.to_string());
        }
    }
    None
}

async fn parse_anthropic_sse_stream(
    resp: reqwest::Response,
    stream_tx: Option<mpsc::UnboundedSender<StreamChunk>>,
) -> Result<LlmResponse, ProviderError> {
    let mut content = String::new();
    let mut reasoning = String::new();
    let mut finish_reason: Option<String> = None;
    let mut usage = UsageMetrics::default();
    let mut blocks: BTreeMap<usize, (String, String, String)> = BTreeMap::new();

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
                Some("message_start") => {
                    if let Some(u) = v.pointer("/message/usage") {
                        usage.input_tokens =
                            u.get("input_tokens").and_then(|x| x.as_i64()).unwrap_or(0);
                        usage.cache_read_tokens = u
                            .get("cache_read_input_tokens")
                            .and_then(|x| x.as_i64())
                            .unwrap_or(0);
                        usage.cache_write_tokens = u
                            .get("cache_creation_input_tokens")
                            .and_then(|x| x.as_i64())
                            .unwrap_or(0);
                    }
                }
                Some("content_block_start") => {
                    let idx = v.get("index").and_then(|x| x.as_u64()).unwrap_or(0) as usize;
                    if let Some(cb) = v.get("content_block") {
                        if cb.get("type").and_then(|x| x.as_str()) == Some("tool_use") {
                            let id = cb
                                .get("id")
                                .and_then(|x| x.as_str())
                                .unwrap_or_default()
                                .to_string();
                            let name = cb
                                .get("name")
                                .and_then(|x| x.as_str())
                                .unwrap_or_default()
                                .to_string();
                            blocks.insert(idx, (id, name, String::new()));
                        }
                    }
                }
                Some("content_block_delta") => {
                    let idx = v.get("index").and_then(|x| x.as_u64()).unwrap_or(0) as usize;
                    if let Some(delta) = v.get("delta") {
                        match delta.get("type").and_then(|x| x.as_str()) {
                            Some("text_delta") => {
                                if let Some(txt) = delta.get("text").and_then(|x| x.as_str()) {
                                    content.push_str(txt);
                                    if let Some(tx) = &stream_tx {
                                        let _ = tx.send(StreamChunk::ContentDelta(txt.to_string()));
                                    }
                                }
                            }
                            Some("thinking_delta") => {
                                if let Some(r_txt) = delta.get("thinking").and_then(|x| x.as_str())
                                {
                                    reasoning.push_str(r_txt);
                                    if let Some(tx) = &stream_tx {
                                        let _ =
                                            tx.send(StreamChunk::ReasoningDelta(r_txt.to_string()));
                                    }
                                }
                            }
                            Some("input_json_delta") => {
                                if let Some(part) =
                                    delta.get("partial_json").and_then(|x| x.as_str())
                                {
                                    if let Some(entry) = blocks.get_mut(&idx) {
                                        entry.2.push_str(part);
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
                Some("message_delta") => {
                    if let Some(sr) = v.pointer("/delta/stop_reason").and_then(|x| x.as_str()) {
                        finish_reason = Some(sr.to_string());
                    }
                    if let Some(out) = v.pointer("/usage/output_tokens").and_then(|x| x.as_i64()) {
                        usage.output_tokens = out;
                    }
                }
                _ => {}
            }
        }
    }

    let tool_calls = blocks
        .into_iter()
        .map(|(idx, (id, name, args_raw))| {
            let id = if id.is_empty() {
                format!("toolu_{idx}")
            } else {
                id
            };
            let arguments = if args_raw.trim().is_empty() {
                json!({})
            } else {
                serde_json::from_str(&args_raw).unwrap_or_else(|_| json!({ "raw": args_raw }))
            };
            ToolCall {
                id,
                name,
                arguments,
            }
        })
        .collect();

    let parsed = LlmResponse {
        content,
        reasoning: (!reasoning.is_empty()).then_some(reasoning),
        tool_calls,
        finish_reason,
        usage,
    };
    if let Some(tx) = stream_tx {
        let _ = tx.send(StreamChunk::Done(parsed.clone()));
    }
    Ok(parsed)
}
