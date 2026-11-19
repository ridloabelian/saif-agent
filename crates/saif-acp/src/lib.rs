use saif_core::{AgentRunner, TurnOptions};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::PathBuf;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

#[derive(Debug, Deserialize)]
struct AcpRequest {
    id: Option<Value>,
    method: String,
    #[serde(default)]
    params: Value,
}

#[derive(Debug, Serialize)]
struct AcpResponse {
    jsonrpc: &'static str,
    id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<Value>,
}

/// Agent Client Protocol stdio JSON-RPC server (`acp_adapter/` parity).
pub async fn run_acp_stdio(agent: AgentRunner, cwd: PathBuf) -> anyhow::Result<()> {
    let mut stdin = BufReader::new(tokio::io::stdin()).lines();
    let mut stdout = tokio::io::stdout();

    while let Ok(Some(line)) = stdin.next_line().await {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(req) = serde_json::from_str::<AcpRequest>(trimmed) else {
            continue;
        };
        let Some(id) = req.id else {
            continue;
        };

        let resp = match req.method.as_str() {
            "initialize" => AcpResponse {
                jsonrpc: "2.0",
                id,
                result: Some(json!({
                    "protocolVersion": 1,
                    "agentInfo": {
                        "name": "saif-acp",
                        "version": env!("CARGO_PKG_VERSION")
                    },
                    "capabilities": {
                        "loadSession": true,
                        "promptCapabilities": { "embeddedContext": true }
                    }
                })),
                error: None,
            },
            "session/new" => {
                let sid = format!("acp-{}", &Uuid::new_v4().to_string()[..12]);
                AcpResponse {
                    jsonrpc: "2.0",
                    id,
                    result: Some(json!({ "sessionId": sid })),
                    error: None,
                }
            }
            "session/prompt" => {
                let prompt = req
                    .params
                    .get("prompt")
                    .and_then(|v| v.as_str())
                    .or_else(|| {
                        req.params
                            .pointer("/prompt/0/text")
                            .and_then(|v| v.as_str())
                    })
                    .unwrap_or("")
                    .to_string();
                let sid = req
                    .params
                    .get("sessionId")
                    .and_then(|v| v.as_str())
                    .map(ToString::to_string);
                let opts = TurnOptions {
                    session_id: sid,
                    source: "acp".to_string(),
                    cwd: cwd.clone(),
                    ..TurnOptions::default()
                };
                match agent
                    .run_conversation(&prompt, None, opts, None, CancellationToken::new(), None)
                    .await
                {
                    Ok(out) => AcpResponse {
                        jsonrpc: "2.0",
                        id,
                        result: Some(json!({
                            "sessionId": out.session_id,
                            "stopReason": "end_turn",
                            "text": out.final_response
                        })),
                        error: None,
                    },
                    Err(e) => AcpResponse {
                        jsonrpc: "2.0",
                        id,
                        result: None,
                        error: Some(json!({ "code": -32000, "message": e.to_string() })),
                    },
                }
            }
            other => AcpResponse {
                jsonrpc: "2.0",
                id,
                result: None,
                error: Some(
                    json!({ "code": -32601, "message": format!("unsupported method: {other}") }),
                ),
            },
        };

        let encoded = serde_json::to_string(&resp)? + "\n";
        stdout.write_all(encoded.as_bytes()).await?;
        stdout.flush().await?;
    }
    Ok(())
}
