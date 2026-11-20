use crate::multiplex::GatewayMultiplexer;
use crate::platforms::builtin_platform_specs;
use axum::extract::{Path as AxumPath, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use saif_core::{AgentRunner, TurnOptions};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

#[derive(Clone)]
pub struct GatewayHttpState {
    pub agent: AgentRunner,
    pub cwd: PathBuf,
    pub multiplexer: Option<Arc<GatewayMultiplexer>>,
}

#[derive(Debug, Deserialize)]
pub struct ChatCompletionMsg {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Deserialize)]
pub struct OpenAiChatRequest {
    pub model: Option<String>,
    pub messages: Vec<ChatCompletionMsg>,
}

#[derive(Debug, Serialize)]
pub struct WebhookAck {
    pub accepted: bool,
    pub session_id: String,
    pub response: String,
}

pub fn build_gateway_router(state: GatewayHttpState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/v1/chat/completions", post(openai_chat_completions))
        .route("/v1/platforms", get(list_platforms))
        .route("/webhook", post(webhook_ingress))
        .route("/webhooks/{platform}", post(platform_webhook_ingress))
        .with_state(Arc::new(state))
}

async fn health() -> Json<Value> {
    Json(json!({ "status": "ok", "service": "saif-gateway" }))
}

async fn list_platforms() -> Json<Value> {
    Json(json!({ "platforms": builtin_platform_specs() }))
}

async fn openai_chat_completions(
    State(state): State<Arc<GatewayHttpState>>,
    Json(req): Json<OpenAiChatRequest>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let prompt = req
        .messages
        .iter()
        .rev()
        .find(|m| m.role == "user")
        .map(|m| m.content.clone())
        .unwrap_or_default();

    let opts = TurnOptions {
        source: "api_server".to_string(),
        model_override: req.model.clone(),
        cwd: state.cwd.clone(),
        ..TurnOptions::default()
    };

    let outcome = state
        .agent
        .run_conversation(&prompt, None, opts, None, CancellationToken::new(), None)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(json!({
        "id": format!("chatcmpl-{}", Uuid::new_v4()),
        "object": "chat.completion",
        "model": req.model.unwrap_or_else(|| "saif-default".to_string()),
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": outcome.final_response
            },
            "finish_reason": "stop"
        }],
        "usage": {
            "prompt_tokens": outcome.usage.input_tokens,
            "completion_tokens": outcome.usage.output_tokens,
            "total_tokens": outcome.usage.input_tokens + outcome.usage.output_tokens
        }
    })))
}

async fn webhook_ingress(
    State(state): State<Arc<GatewayHttpState>>,
    Json(payload): Json<Value>,
) -> Result<Json<WebhookAck>, (StatusCode, String)> {
    let prompt = payload
        .get("prompt")
        .or_else(|| payload.get("text"))
        .and_then(|v| v.as_str())
        .unwrap_or("Webhook trigger received.")
        .to_string();

    let opts = TurnOptions {
        source: "webhook".to_string(),
        cwd: state.cwd.clone(),
        ..TurnOptions::default()
    };

    let outcome = state
        .agent
        .run_conversation(&prompt, None, opts, None, CancellationToken::new(), None)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(WebhookAck {
        accepted: true,
        session_id: outcome.session_id,
        response: outcome.final_response,
    }))
}

async fn platform_webhook_ingress(
    State(state): State<Arc<GatewayHttpState>>,
    AxumPath(platform): AxumPath<String>,
    Json(payload): Json<Value>,
) -> Result<Json<Value>, (StatusCode, String)> {
    if let Some(mux) = &state.multiplexer {
        let out = mux
            .handle_inbound_event(&platform, &payload, &state.cwd)
            .await
            .map_err(|e| (StatusCode::BAD_REQUEST, e))?;
        return serde_json::to_value(&out)
            .map(Json)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()));
    }

    let ev = crate::platforms::parse_inbound_webhook(&platform, &payload)
        .map_err(|e| (StatusCode::BAD_REQUEST, e))?
        .ok_or_else(|| (StatusCode::BAD_REQUEST, "empty webhook event".to_string()))?;
    let opts = TurnOptions {
        source: ev.platform.clone(),
        cwd: state.cwd.clone(),
        ..TurnOptions::default()
    };
    let outcome = state
        .agent
        .run_conversation(&ev.text, None, opts, None, CancellationToken::new(), None)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(json!({
        "accepted": true,
        "platform": ev.platform,
        "session_id": outcome.session_id,
        "chunks": [outcome.final_response]
    })))
}
