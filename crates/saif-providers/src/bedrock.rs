use crate::catalog::ResolvedRoute;
use crate::transport::{classify_http_error, LlmTransport};
use crate::types::{
    LlmRequest, LlmResponse, MessageRole, ProviderError, StreamChunk, ToolCall, UsageMetrics,
};
use async_trait::async_trait;
use chrono::Utc;
use regex::Regex;
use reqwest::Client;
use saif_config::SecretScope;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::sync::OnceLock;
use tokio::sync::mpsc;

fn xai_grok_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?i)^(?:[a-z]+\.)?xai\.grok").unwrap_or_else(|_| unreachable!()))
}

pub fn is_bedrock_xai_grok_no_sampling(model: &str) -> bool {
    xai_grok_re().is_match(model.trim())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AwsCredentials {
    pub access_key_id: String,
    pub secret_access_key: String,
    pub session_token: Option<String>,
    pub region: String,
    pub profile_name: Option<String>,
}

/// Fail-closed AWS credential resolution (`bedrock_adapter.py:scoped_aws_session_kwargs`).
pub fn resolve_scoped_aws_credentials(
    secrets: &SecretScope,
    is_multiplex: bool,
) -> Result<AwsCredentials, ProviderError> {
    let access_key = secrets.get_secret("AWS_ACCESS_KEY_ID");
    let secret_key = secrets.get_secret("AWS_SECRET_ACCESS_KEY");
    let session_token = secrets.get_secret("AWS_SESSION_TOKEN");
    let profile_name = secrets.get_secret("AWS_PROFILE");
    let region = secrets
        .get_secret("AWS_REGION")
        .or_else(|| secrets.get_secret("AWS_DEFAULT_REGION"))
        .unwrap_or_else(|| "us-east-1".to_string());

    let complete = (access_key.is_some() && secret_key.is_some()) || profile_name.is_some();
    if is_multiplex && !complete {
        return Err(ProviderError::Auth(
            "Bedrock auth is refused for this profile: under multiplexed profiles, ambient host AWS credentials must not be borrowed; set AWS_ACCESS_KEY_ID + AWS_SECRET_ACCESS_KEY or AWS_PROFILE in this profile's .env".to_string(),
        ));
    }

    match (access_key, secret_key) {
        (Some(ak), Some(sk)) => Ok(AwsCredentials {
            access_key_id: ak,
            secret_access_key: sk,
            session_token,
            region,
            profile_name,
        }),
        _ => Err(ProviderError::MissingCredentials {
            provider: "bedrock".to_string(),
            env_var: "AWS_ACCESS_KEY_ID / AWS_SECRET_ACCESS_KEY".to_string(),
        }),
    }
}

/// Pure-Rust RFC 2104 HMAC-SHA256 over `sha2::Sha256` (zero extra crates).
pub fn hmac_sha256(key: &[u8], data: &[u8]) -> [u8; 32] {
    const BLOCK_LEN: usize = 64;
    let mut k_block = [0u8; BLOCK_LEN];
    if key.len() > BLOCK_LEN {
        let hashed = Sha256::digest(key);
        k_block[..32].copy_from_slice(&hashed);
    } else {
        k_block[..key.len()].copy_from_slice(key);
    }

    let mut ipad = [0x36u8; BLOCK_LEN];
    let mut opad = [0x5cu8; BLOCK_LEN];
    for i in 0..BLOCK_LEN {
        ipad[i] ^= k_block[i];
        opad[i] ^= k_block[i];
    }

    let mut inner = Sha256::new();
    inner.update(ipad);
    inner.update(data);
    let inner_hash = inner.finalize();

    let mut outer = Sha256::new();
    outer.update(opad);
    outer.update(inner_hash);
    let out = outer.finalize();

    let mut result = [0u8; 32];
    result.copy_from_slice(&out);
    result
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SigV4Headers {
    pub authorization: String,
    pub amz_date: String,
    pub content_sha256: String,
    pub security_token: Option<String>,
    pub host: String,
}

fn aws_pct_encode(segment: &str) -> String {
    let mut out = String::new();
    for b in segment.as_bytes() {
        match *b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'_' | b'-' | b'~' | b'.' => {
                out.push(*b as char);
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn canonical_uri_path(raw_path: &str) -> String {
    if raw_path.is_empty() || raw_path == "/" {
        return "/".to_string();
    }
    let parts: Vec<String> = raw_path.split('/').map(aws_pct_encode).collect();
    parts.join("/")
}

/// Sign an HTTP request using AWS Signature Version 4 (`AWS4-HMAC-SHA256`).
#[allow(clippy::too_many_arguments)]
pub fn sign_aws_sigv4_headers(
    method: &str,
    full_url: &str,
    region: &str,
    service: &str,
    access_key: &str,
    secret_key: &str,
    session_token: Option<&str>,
    payload: &[u8],
    amz_date: &str,
) -> Result<SigV4Headers, ProviderError> {
    let parsed = url::Url::parse(full_url)
        .map_err(|e| ProviderError::Transport(format!("invalid bedrock url '{full_url}': {e}")))?;
    let host = match parsed.port() {
        Some(port) => format!(
            "{}:{port}",
            parsed.host_str().unwrap_or("bedrock-runtime.amazonaws.com")
        ),
        None => parsed
            .host_str()
            .unwrap_or("bedrock-runtime.amazonaws.com")
            .to_string(),
    };
    let date_stamp = &amz_date[..8.min(amz_date.len())];
    let payload_hash = hex::encode(Sha256::digest(payload));
    let canonical_uri = canonical_uri_path(parsed.path());

    let mut query_pairs: Vec<(String, String)> = parsed
        .query_pairs()
        .map(|(k, v)| (aws_pct_encode(&k), aws_pct_encode(&v)))
        .collect();
    query_pairs.sort();
    let canonical_query = query_pairs
        .into_iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("&");

    let mut hdrs: BTreeMap<&str, String> = BTreeMap::new();
    hdrs.insert("host", host.clone());
    hdrs.insert("x-amz-content-sha256", payload_hash.clone());
    hdrs.insert("x-amz-date", amz_date.to_string());
    if let Some(tok) = session_token.filter(|t| !t.is_empty()) {
        hdrs.insert("x-amz-security-token", tok.to_string());
    }

    let canonical_headers: String = hdrs
        .iter()
        .map(|(k, v)| format!("{k}:{}\n", v.trim()))
        .collect();
    let signed_headers: String = hdrs.keys().copied().collect::<Vec<_>>().join(";");

    let canonical_request = format!(
        "{method}\n{canonical_uri}\n{canonical_query}\n{canonical_headers}\n{signed_headers}\n{payload_hash}"
    );
    let canonical_hash = hex::encode(Sha256::digest(canonical_request.as_bytes()));
    let credential_scope = format!("{date_stamp}/{region}/{service}/aws4_request");
    let string_to_sign =
        format!("AWS4-HMAC-SHA256\n{amz_date}\n{credential_scope}\n{canonical_hash}");

    let k_secret = format!("AWS4{secret_key}");
    let k_date = hmac_sha256(k_secret.as_bytes(), date_stamp.as_bytes());
    let k_region = hmac_sha256(&k_date, region.as_bytes());
    let k_service = hmac_sha256(&k_region, service.as_bytes());
    let k_signing = hmac_sha256(&k_service, b"aws4_request");
    let signature = hex::encode(hmac_sha256(&k_signing, string_to_sign.as_bytes()));

    let authorization = format!(
        "AWS4-HMAC-SHA256 Credential={access_key}/{credential_scope}, SignedHeaders={signed_headers}, Signature={signature}"
    );

    Ok(SigV4Headers {
        authorization,
        amz_date: amz_date.to_string(),
        content_sha256: payload_hash,
        security_token: session_token.map(ToString::to_string),
        host,
    })
}

#[derive(Clone)]
pub struct BedrockConverseTransport {
    client: Client,
}

impl Default for BedrockConverseTransport {
    fn default() -> Self {
        Self::new()
    }
}

impl BedrockConverseTransport {
    pub fn new() -> Self {
        Self {
            client: Client::new(),
        }
    }

    pub fn build_payload(&self, route: &ResolvedRoute, req: &LlmRequest) -> Value {
        let mut system_blocks = Vec::new();
        let mut messages: Vec<Value> = Vec::new();

        for msg in &req.messages {
            match msg.role {
                MessageRole::System => {
                    if !msg.content.trim().is_empty() {
                        system_blocks.push(json!({ "text": msg.content }));
                    }
                }
                MessageRole::User => {
                    messages.push(json!({
                        "role": "user",
                        "content": [{ "text": msg.content }]
                    }));
                }
                MessageRole::Assistant => {
                    let mut blocks = Vec::new();
                    if !msg.content.is_empty() {
                        blocks.push(json!({ "text": msg.content }));
                    }
                    if let Some(tcs) = &msg.tool_calls {
                        for tc in tcs {
                            blocks.push(json!({
                                "toolUse": {
                                    "toolUseId": tc.id,
                                    "name": tc.name,
                                    "input": tc.arguments
                                }
                            }));
                        }
                    }
                    messages.push(json!({ "role": "assistant", "content": blocks }));
                }
                MessageRole::Tool => {
                    let block = json!({
                        "toolResult": {
                            "toolUseId": msg.tool_call_id.clone().unwrap_or_default(),
                            "content": [{ "text": msg.content }],
                            "status": "success"
                        }
                    });
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
        if !system_blocks.is_empty() {
            body.insert("system".into(), Value::Array(system_blocks));
        }
        body.insert("messages".into(), Value::Array(messages));

        if !req.tools.is_empty() {
            let specs: Vec<Value> = req
                .tools
                .iter()
                .map(|t| {
                    json!({
                        "toolSpec": {
                            "name": t.name,
                            "description": t.description,
                            "inputSchema": { "json": t.parameters }
                        }
                    })
                })
                .collect();
            body.insert("toolConfig".into(), json!({ "tools": specs }));
        }

        let mut inf = serde_json::Map::new();
        if let Some(max_tok) = req.max_tokens {
            inf.insert("maxTokens".into(), json!(max_tok));
        }
        if !route.omit_temperature && !is_bedrock_xai_grok_no_sampling(&req.model) {
            if let Some(temp) = req.temperature {
                inf.insert("temperature".into(), json!(temp));
            }
        }
        if !inf.is_empty() {
            body.insert("inferenceConfig".into(), Value::Object(inf));
        }

        Value::Object(body)
    }
}

#[async_trait]
impl LlmTransport for BedrockConverseTransport {
    async fn complete(
        &self,
        route: &ResolvedRoute,
        request: &LlmRequest,
        stream_tx: Option<mpsc::UnboundedSender<StreamChunk>>,
    ) -> Result<LlmResponse, ProviderError> {
        let region = route.aws_region.as_deref().unwrap_or("us-east-1");
        if route.multiplex_active
            && (route.aws_access_key_id.is_none() || route.aws_secret_access_key.is_none())
            && route.aws_profile.is_none()
        {
            return Err(ProviderError::Auth(
                "Bedrock auth is refused for this multiplexed profile: missing scoped AWS credentials".to_string(),
            ));
        }

        let access_key = route
            .aws_access_key_id
            .as_deref()
            .or(route.api_key.as_deref())
            .ok_or_else(|| ProviderError::MissingCredentials {
                provider: "bedrock".to_string(),
                env_var: "AWS_ACCESS_KEY_ID".to_string(),
            })?;
        let secret_key = route.aws_secret_access_key.as_deref().ok_or_else(|| {
            ProviderError::MissingCredentials {
                provider: "bedrock".to_string(),
                env_var: "AWS_SECRET_ACCESS_KEY".to_string(),
            }
        })?;

        let endpoint_base = if route.base_url.trim().is_empty() {
            format!("https://bedrock-runtime.{region}.amazonaws.com")
        } else {
            route.base_url.trim_end_matches('/').to_string()
        };
        let encoded_model = aws_pct_encode(&request.model);
        let url = format!("{endpoint_base}/model/{encoded_model}/converse");

        let payload = self.build_payload(route, request);
        let payload_bytes = serde_json::to_vec(&payload)?;
        let amz_date = Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
        let sig = sign_aws_sigv4_headers(
            "POST",
            &url,
            region,
            "bedrock",
            access_key,
            secret_key,
            route.aws_session_token.as_deref(),
            &payload_bytes,
            &amz_date,
        )?;

        let mut req_builder = self
            .client
            .post(url)
            .header("Content-Type", "application/json")
            .header("Authorization", sig.authorization)
            .header("x-amz-date", sig.amz_date)
            .header("x-amz-content-sha256", sig.content_sha256)
            .body(payload_bytes);
        if let Some(tok) = sig.security_token {
            req_builder = req_builder.header("x-amz-security-token", tok);
        }

        let resp = req_builder.send().await?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(classify_http_error(status.as_u16(), None, body));
        }

        let val: Value = resp.json().await?;
        let parsed = parse_bedrock_converse(&val);
        if let Some(tx) = stream_tx {
            if !parsed.content.is_empty() {
                let _ = tx.send(StreamChunk::ContentDelta(parsed.content.clone()));
            }
            let _ = tx.send(StreamChunk::Done(parsed.clone()));
        }
        Ok(parsed)
    }
}

pub fn parse_bedrock_converse(v: &Value) -> LlmResponse {
    let mut content = String::new();
    let mut reasoning = String::new();
    let mut tool_calls = Vec::new();

    if let Some(blocks) = v
        .pointer("/output/message/content")
        .and_then(|c| c.as_array())
    {
        for b in blocks {
            if let Some(t) = b.get("text").and_then(|x| x.as_str()) {
                content.push_str(t);
            }
            if let Some(r) = b
                .pointer("/reasoningContent/reasoningText/text")
                .and_then(|x| x.as_str())
            {
                reasoning.push_str(r);
            }
            if let Some(tu) = b.get("toolUse") {
                tool_calls.push(ToolCall {
                    id: tu
                        .get("toolUseId")
                        .and_then(|x| x.as_str())
                        .unwrap_or("call_0")
                        .to_string(),
                    name: tu
                        .get("name")
                        .and_then(|x| x.as_str())
                        .unwrap_or_default()
                        .to_string(),
                    arguments: tu.get("input").cloned().unwrap_or_else(|| json!({})),
                });
            }
        }
    }

    let u = v.get("usage");
    let usage = UsageMetrics {
        input_tokens: u
            .and_then(|x| x.get("inputTokens"))
            .and_then(|n| n.as_i64())
            .unwrap_or(0),
        output_tokens: u
            .and_then(|x| x.get("outputTokens"))
            .and_then(|n| n.as_i64())
            .unwrap_or(0),
        cache_read_tokens: u
            .and_then(|x| x.get("cacheReadInputTokens"))
            .and_then(|n| n.as_i64())
            .unwrap_or(0),
        cache_write_tokens: u
            .and_then(|x| x.get("cacheWriteInputTokens"))
            .and_then(|n| n.as_i64())
            .unwrap_or(0),
        reasoning_tokens: 0,
    };

    let finish_reason = v
        .get("stopReason")
        .and_then(|s| s.as_str())
        .map(ToString::to_string);

    LlmResponse {
        content,
        reasoning: (!reasoning.is_empty()).then_some(reasoning),
        tool_calls,
        finish_reason,
        usage,
    }
}
