use crate::registry::{ToolContext, ToolError};
use base64::Engine;
use futures_util::StreamExt;
use reqwest::Client;
use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;

pub const TTS_RESPONSE_BODY_LIMIT_BYTES: usize = 16 * 1024 * 1024; // 16 MiB cap (`tts_tool_providers.py`)
pub const DEFAULT_XAI_SPEED_MIN: f64 = 0.7;
pub const DEFAULT_XAI_SPEED_MAX: f64 = 1.5;

/// Matches `tools/tts_tool_providers.py:_tts_response_format_from_path`.
pub fn tts_response_format_from_path(output_path: &str) -> &'static str {
    if output_path.ends_with(".ogg") {
        "opus"
    } else if output_path.ends_with(".wav") {
        "wav"
    } else if output_path.ends_with(".flac") {
        "flac"
    } else {
        "mp3"
    }
}

/// Wrap raw linear PCM samples in a 44-byte RIFF/WAVE header (`_wrap_pcm_as_wav`).
pub fn wrap_pcm_as_wav(
    pcm: &[u8],
    sample_rate: u32,
    channels: u16,
    bits_per_sample: u16,
) -> Vec<u8> {
    let data_len = u32::try_from(pcm.len()).unwrap_or(u32::MAX - 36);
    let byte_rate = sample_rate
        .saturating_mul(u32::from(channels))
        .saturating_mul(u32::from(bits_per_sample))
        / 8;
    let block_align = channels.saturating_mul(bits_per_sample) / 8;

    let mut wav = Vec::with_capacity(44 + pcm.len());
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36u32.saturating_add(data_len)).to_le_bytes());
    wav.extend_from_slice(b"WAVE");
    wav.extend_from_slice(b"fmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes()); // PCM
    wav.extend_from_slice(&channels.to_le_bytes());
    wav.extend_from_slice(&sample_rate.to_le_bytes());
    wav.extend_from_slice(&byte_rate.to_le_bytes());
    wav.extend_from_slice(&block_align.to_le_bytes());
    wav.extend_from_slice(&bits_per_sample.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_len.to_le_bytes());
    wav.extend_from_slice(&pcm[..data_len as usize]);
    wav
}

pub fn enforce_byte_limit(bytes: &[u8], limit: usize, label: &str) -> Result<(), ToolError> {
    if bytes.len() > limit {
        return Err(ToolError::Execution(format!(
            "{label} response exceeded {limit} byte limit ({} bytes)",
            bytes.len()
        )));
    }
    Ok(())
}

pub async fn read_bounded_bytes(
    resp: reqwest::Response,
    limit: usize,
    label: &str,
) -> Result<Vec<u8>, ToolError> {
    let mut stream = resp.bytes_stream();
    let mut out = Vec::new();
    while let Some(chunk_res) = stream.next().await {
        let chunk = chunk_res.map_err(|e| ToolError::Execution(format!("{label} stream: {e}")))?;
        if out.len().saturating_add(chunk.len()) > limit {
            return Err(ToolError::Execution(format!(
                "{label} response exceeded {limit} byte cap"
            )));
        }
        out.extend_from_slice(&chunk);
    }
    Ok(out)
}

fn resolve_output_path(ctx: &ToolContext, raw: &str) -> PathBuf {
    let p = PathBuf::from(raw);
    if p.is_absolute() {
        p
    } else {
        ctx.cwd.join(p)
    }
}

pub async fn handle_text_to_speech(args: &Value, ctx: &ToolContext) -> Result<String, ToolError> {
    let text = args
        .get("text")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ToolError::InvalidArgs("missing 'text'".to_string()))?;
    let provider = args
        .get("provider")
        .and_then(|v| v.as_str())
        .unwrap_or("openai")
        .to_ascii_lowercase();
    let out_raw = args
        .get("output_path")
        .and_then(|v| v.as_str())
        .unwrap_or("speech.mp3");
    let format = tts_response_format_from_path(out_raw);
    let out_path = resolve_output_path(ctx, out_raw);
    let base_override = args.get("api_base_url").and_then(|v| v.as_str());

    let client = Client::new();
    let bytes: Vec<u8> = match provider.as_str() {
        "openai" | "xai" => {
            let env_var = if provider == "xai" {
                "XAI_API_KEY"
            } else {
                "OPENAI_API_KEY"
            };
            let key = ctx.profile.secrets.get_secret(env_var).ok_or_else(|| {
                ToolError::Execution(format!("{env_var} not set for TTS provider '{provider}'"))
            })?;
            let base = base_override.unwrap_or(if provider == "xai" {
                "https://api.x.ai/v1"
            } else {
                "https://api.openai.com/v1"
            });
            let speed = args
                .get("speed")
                .and_then(|v| v.as_f64())
                .map(|s| s.clamp(DEFAULT_XAI_SPEED_MIN, DEFAULT_XAI_SPEED_MAX));
            let mut body = json!({
                "model": args.get("model").and_then(|v| v.as_str()).unwrap_or("gpt-4o-mini-tts"),
                "input": text,
                "voice": args.get("voice").and_then(|v| v.as_str()).unwrap_or(if provider == "xai" { "eve" } else { "alloy" }),
                "response_format": format,
            });
            if let Some(sp) = speed {
                body["speed"] = json!(sp);
            }
            let resp = client
                .post(format!("{}/audio/speech", base.trim_end_matches('/')))
                .bearer_auth(key)
                .json(&body)
                .send()
                .await
                .map_err(|e| ToolError::Execution(e.to_string()))?;
            if !resp.status().is_success() {
                let code = resp.status();
                let err_txt = resp.text().await.unwrap_or_default();
                return Err(ToolError::Execution(format!(
                    "TTS {provider} HTTP {code}: {err_txt}"
                )));
            }
            read_bounded_bytes(resp, TTS_RESPONSE_BODY_LIMIT_BYTES, &provider).await?
        }
        "elevenlabs" => {
            let key = ctx
                .profile
                .secrets
                .get_secret("ELEVENLABS_API_KEY")
                .ok_or_else(|| ToolError::Execution("ELEVENLABS_API_KEY not set".to_string()))?;
            let voice = args
                .get("voice")
                .and_then(|v| v.as_str())
                .unwrap_or("pNInz6obpgDQGcFmaJgB");
            let base = base_override.unwrap_or("https://api.elevenlabs.io/v1");
            let resp = client
                .post(format!(
                    "{}/text-to-speech/{voice}",
                    base.trim_end_matches('/')
                ))
                .header("xi-api-key", key)
                .json(&json!({
                    "text": text,
                    "model_id": args.get("model").and_then(|v| v.as_str()).unwrap_or("eleven_multilingual_v2")
                }))
                .send()
                .await
                .map_err(|e| ToolError::Execution(e.to_string()))?;
            if !resp.status().is_success() {
                return Err(ToolError::Execution(format!(
                    "ElevenLabs HTTP {}",
                    resp.status()
                )));
            }
            read_bounded_bytes(resp, TTS_RESPONSE_BODY_LIMIT_BYTES, "elevenlabs").await?
        }
        "gemini" => {
            let key = ctx
                .profile
                .secrets
                .get_secret("GEMINI_API_KEY")
                .ok_or_else(|| ToolError::Execution("GEMINI_API_KEY not set".to_string()))?;
            let base = base_override.unwrap_or("https://generativelanguage.googleapis.com/v1beta");
            let model = args
                .get("model")
                .and_then(|v| v.as_str())
                .unwrap_or("gemini-2.5-flash-preview-tts");
            let resp = client
                .post(format!(
                    "{}/models/{model}:generateContent",
                    base.trim_end_matches('/')
                ))
                .header("x-goog-api-key", key)
                .json(&json!({
                    "contents": [{ "parts": [{ "text": text }] }],
                    "generationConfig": { "responseModalities": ["AUDIO"] }
                }))
                .send()
                .await
                .map_err(|e| ToolError::Execution(e.to_string()))?;
            if !resp.status().is_success() {
                return Err(ToolError::Execution(format!(
                    "Gemini TTS HTTP {}",
                    resp.status()
                )));
            }
            let raw = read_bounded_bytes(resp, TTS_RESPONSE_BODY_LIMIT_BYTES, "gemini").await?;
            let v: Value = serde_json::from_slice(&raw)
                .map_err(|e| ToolError::Execution(format!("gemini JSON: {e}")))?;
            let b64 = v
                .pointer("/candidates/0/content/parts/0/inlineData/data")
                .and_then(|x| x.as_str())
                .ok_or_else(|| {
                    ToolError::Execution("missing inlineData audio in Gemini response".to_string())
                })?;
            let pcm = base64::engine::general_purpose::STANDARD
                .decode(b64)
                .map_err(|e| ToolError::Execution(format!("base64 decode: {e}")))?;
            wrap_pcm_as_wav(&pcm, 24_000, 1, 16)
        }
        "minimax" | "mistral" | "edge" => {
            return Err(ToolError::Execution(format!(
                "TTS provider '{provider}' requires configured endpoint credentials or sidecar"
            )));
        }
        other => {
            return Err(ToolError::InvalidArgs(format!(
                "unsupported TTS provider '{other}'"
            )))
        }
    };

    enforce_byte_limit(&bytes, TTS_RESPONSE_BODY_LIMIT_BYTES, "tts")?;
    if let Some(parent) = out_path.parent() {
        fs::create_dir_all(parent).map_err(|e| ToolError::Execution(e.to_string()))?;
    }
    fs::write(&out_path, &bytes).map_err(|e| ToolError::Execution(e.to_string()))?;

    Ok(json!({
        "success": true,
        "provider": provider,
        "path": out_path.display().to_string(),
        "format": format,
        "bytes": bytes.len()
    })
    .to_string())
}

pub async fn handle_speech_to_text(args: &Value, ctx: &ToolContext) -> Result<String, ToolError> {
    let audio_path = args
        .get("audio_path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ToolError::InvalidArgs("missing 'audio_path'".to_string()))?;
    let resolved = resolve_output_path(ctx, audio_path);
    let raw_bytes = fs::read(&resolved).map_err(|e| {
        ToolError::Execution(format!("cannot read audio '{}': {e}", resolved.display()))
    })?;
    let key = ctx
        .profile
        .secrets
        .get_secret("OPENAI_API_KEY")
        .ok_or_else(|| ToolError::Execution("OPENAI_API_KEY not set for STT".to_string()))?;
    let base = args
        .get("api_base_url")
        .and_then(|v| v.as_str())
        .unwrap_or("https://api.openai.com/v1");
    let b64 = base64::engine::general_purpose::STANDARD.encode(&raw_bytes);
    let resp = Client::new()
        .post(format!(
            "{}/audio/transcriptions",
            base.trim_end_matches('/')
        ))
        .bearer_auth(key)
        .json(&json!({
            "model": args.get("model").and_then(|v| v.as_str()).unwrap_or("whisper-1"),
            "audio_base64": b64,
            "filename": resolved.file_name().and_then(|s| s.to_str()).unwrap_or("audio.wav")
        }))
        .send()
        .await
        .map_err(|e| ToolError::Execution(e.to_string()))?;
    let status = resp.status();
    let v: Value = resp
        .json()
        .await
        .map_err(|e| ToolError::Execution(format!("STT JSON: {e}")))?;
    if !status.is_success() {
        return Err(ToolError::Execution(format!("STT HTTP {status}: {v}")));
    }
    Ok(json!({
        "success": true,
        "text": v.get("text").and_then(|t| t.as_str()).unwrap_or("")
    })
    .to_string())
}

pub async fn handle_image_generate(args: &Value, ctx: &ToolContext) -> Result<String, ToolError> {
    let prompt = args
        .get("prompt")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ToolError::InvalidArgs("missing 'prompt'".to_string()))?;
    let key = ctx
        .profile
        .secrets
        .get_secret("OPENAI_API_KEY")
        .or_else(|| ctx.profile.secrets.get_secret("OPENROUTER_API_KEY"))
        .ok_or_else(|| ToolError::Execution("missing image generation API key".to_string()))?;
    let base = args
        .get("api_base_url")
        .and_then(|v| v.as_str())
        .unwrap_or("https://api.openai.com/v1");
    let resp = Client::new()
        .post(format!("{}/images/generations", base.trim_end_matches('/')))
        .bearer_auth(key)
        .json(&json!({
            "model": args.get("model").and_then(|v| v.as_str()).unwrap_or("gpt-image-1"),
            "prompt": prompt,
            "size": args.get("size").and_then(|v| v.as_str()).unwrap_or("1024x1024")
        }))
        .send()
        .await
        .map_err(|e| ToolError::Execution(e.to_string()))?;
    let status = resp.status();
    let v: Value = resp
        .json()
        .await
        .map_err(|e| ToolError::Execution(format!("image_generate JSON: {e}")))?;
    if !status.is_success() {
        return Err(ToolError::Execution(format!(
            "image_generate HTTP {status}: {v}"
        )));
    }
    if let (Some(out_raw), Some(b64)) = (
        args.get("output_path").and_then(|x| x.as_str()),
        v.pointer("/data/0/b64_json").and_then(|x| x.as_str()),
    ) {
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(b64)
            .map_err(|e| ToolError::Execution(format!("image base64: {e}")))?;
        let out_path = resolve_output_path(ctx, out_raw);
        if let Some(parent) = out_path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        fs::write(&out_path, &decoded).map_err(|e| ToolError::Execution(e.to_string()))?;
        return Ok(json!({
            "success": true,
            "path": out_path.display().to_string(),
            "bytes": decoded.len()
        })
        .to_string());
    }
    Ok(
        json!({ "success": true, "data": v.get("data").cloned().unwrap_or(Value::Null) })
            .to_string(),
    )
}

pub async fn handle_video_generate(args: &Value, ctx: &ToolContext) -> Result<String, ToolError> {
    let prompt = args
        .get("prompt")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ToolError::InvalidArgs("missing 'prompt'".to_string()))?;
    let key = ctx
        .profile
        .secrets
        .get_secret("XAI_API_KEY")
        .or_else(|| ctx.profile.secrets.get_secret("OPENAI_API_KEY"))
        .ok_or_else(|| ToolError::Execution("missing XAI_API_KEY/OPENAI_API_KEY".to_string()))?;
    let base = args
        .get("api_base_url")
        .and_then(|v| v.as_str())
        .unwrap_or("https://api.x.ai/v1");
    let resp = Client::new()
        .post(format!("{}/videos/generations", base.trim_end_matches('/')))
        .bearer_auth(key)
        .json(&json!({
            "model": args.get("model").and_then(|v| v.as_str()).unwrap_or("grok-imagine-video"),
            "prompt": prompt
        }))
        .send()
        .await
        .map_err(|e| ToolError::Execution(e.to_string()))?;
    let status = resp.status();
    let v: Value = resp
        .json()
        .await
        .map_err(|e| ToolError::Execution(format!("video_generate JSON: {e}")))?;
    if !status.is_success() {
        return Err(ToolError::Execution(format!(
            "video_generate HTTP {status}: {v}"
        )));
    }
    Ok(json!({ "success": true, "result": v }).to_string())
}
