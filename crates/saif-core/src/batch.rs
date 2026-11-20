use crate::runner::{AgentError, AgentRunner, TurnOptions};
use futures_util::future::join_all;
use saif_providers::{ChatMessage, MessageRole, UsageMetrics};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;

/// Exact Hermes `batch_runner._tool_call_succeeded` semantics (`batch_runner.py`).
pub fn tool_call_succeeded(content: &str) -> bool {
    match serde_json::from_str::<Value>(content) {
        Ok(Value::Object(map)) => {
            if let Some(err) = map.get("error") {
                if !err.is_null() {
                    return false;
                }
            }
            if let Some(Value::Object(inner)) = map.get("content") {
                if let Some(err) = inner.get("error") {
                    if !err.is_null() {
                        return false;
                    }
                }
            }
            !matches!(map.get("success"), Some(Value::Bool(false)))
        }
        Ok(_) => true,
        Err(_) => !content.is_empty() && !content.trim().to_ascii_lowercase().starts_with("error:"),
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolRunStats {
    pub count: usize,
    pub success: usize,
    pub failure: usize,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReasoningRunStats {
    pub total_assistant_turns: usize,
    pub turns_with_reasoning: usize,
    pub turns_without_reasoning: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ShareGptTurn {
    pub from: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchRecord {
    pub id: String,
    pub prompt: String,
    pub response: String,
    pub conversations: Vec<ShareGptTurn>,
    pub tool_stats: BTreeMap<String, ToolRunStats>,
    pub reasoning_stats: ReasoningRunStats,
    pub usage: UsageMetrics,
}

pub fn to_sharegpt_conversations(messages: &[ChatMessage]) -> Vec<ShareGptTurn> {
    messages
        .iter()
        .map(|m| {
            let from = match m.role {
                MessageRole::System => "system",
                MessageRole::User => "human",
                MessageRole::Assistant => "gpt",
                MessageRole::Tool => "tool",
            };
            let mut value = m.content.clone();
            if m.role == MessageRole::Assistant {
                if let Some(tcs) = &m.tool_calls {
                    for tc in tcs {
                        let call_json =
                            json!({ "name": tc.name, "arguments": tc.arguments }).to_string();
                        if !value.is_empty() {
                            value.push('\n');
                        }
                        value.push_str(&format!("<tool_call>{call_json}</tool_call>"));
                    }
                }
            }
            ShareGptTurn {
                from: from.to_string(),
                value,
            }
        })
        .collect()
}

pub fn collect_trajectory_stats(
    messages: &[ChatMessage],
) -> (BTreeMap<String, ToolRunStats>, ReasoningRunStats) {
    let mut tool_stats: BTreeMap<String, ToolRunStats> = BTreeMap::new();
    let mut reasoning_stats = ReasoningRunStats::default();

    for m in messages {
        match m.role {
            MessageRole::Assistant => {
                reasoning_stats.total_assistant_turns += 1;
                if m.reasoning
                    .as_deref()
                    .map(|r| !r.trim().is_empty())
                    .unwrap_or(false)
                {
                    reasoning_stats.turns_with_reasoning += 1;
                } else {
                    reasoning_stats.turns_without_reasoning += 1;
                }
            }
            MessageRole::Tool => {
                let tname = m.tool_name.clone().unwrap_or_else(|| "unknown".to_string());
                let entry = tool_stats.entry(tname).or_default();
                entry.count += 1;
                if tool_call_succeeded(&m.content) {
                    entry.success += 1;
                } else {
                    entry.failure += 1;
                }
            }
            _ => {}
        }
    }

    (tool_stats, reasoning_stats)
}

#[derive(Debug, Clone)]
pub struct BatchInputEntry {
    pub id: String,
    pub prompt: String,
}

pub fn load_batch_entries(path: &Path) -> Result<Vec<BatchInputEntry>, AgentError> {
    let raw = fs::read_to_string(path).map_err(|e| {
        AgentError::RoleAlternation(format!(
            "failed reading batch input {}: {e}",
            path.display()
        ))
    })?;
    let mut out = Vec::new();
    for (idx, line) in raw.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let val: Value = serde_json::from_str(line).map_err(|e| {
            AgentError::RoleAlternation(format!("invalid JSON on line {}: {e}", idx + 1))
        })?;
        let prompt = val
            .get("prompt")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let id = val
            .get("id")
            .and_then(|v| v.as_str())
            .map(ToString::to_string)
            .unwrap_or_else(|| format!("row-{idx}"));
        out.push(BatchInputEntry { id, prompt });
    }
    Ok(out)
}

pub async fn run_batch_dataset(
    agent: AgentRunner,
    input_path: &Path,
    output_path: &Path,
    workers: usize,
    resume: bool,
    cwd: PathBuf,
) -> Result<Vec<BatchRecord>, AgentError> {
    let entries = load_batch_entries(input_path)?;
    let mut done_ids = HashSet::new();
    let mut prior_records = Vec::new();

    if resume && output_path.is_file() {
        if let Ok(existing) = fs::read_to_string(output_path) {
            for line in existing.lines() {
                if let Ok(rec) = serde_json::from_str::<BatchRecord>(line) {
                    done_ids.insert(rec.id.clone());
                    prior_records.push(rec);
                }
            }
        }
    }

    let pending: Vec<BatchInputEntry> = entries
        .into_iter()
        .filter(|e| !done_ids.contains(&e.id))
        .collect();

    let sem = Arc::new(Semaphore::new(workers.max(1)));
    let futs = pending.into_iter().map(|entry| {
        let sem = Arc::clone(&sem);
        let agent = agent.clone();
        let cwd = cwd.clone();
        async move {
            let _permit = sem.acquire().await.map_err(|_| AgentError::Interrupted)?;
            let opts = TurnOptions {
                source: "batch".to_string(),
                skip_persistence: true,
                cwd,
                ..TurnOptions::default()
            };
            let outcome = agent
                .run_conversation(
                    &entry.prompt,
                    None,
                    opts,
                    None,
                    CancellationToken::new(),
                    None,
                )
                .await?;
            let (tool_stats, reasoning_stats) = collect_trajectory_stats(&outcome.messages);
            Ok::<BatchRecord, AgentError>(BatchRecord {
                id: entry.id,
                prompt: entry.prompt,
                response: outcome.final_response,
                conversations: to_sharegpt_conversations(&outcome.messages),
                tool_stats,
                reasoning_stats,
                usage: outcome.usage,
            })
        }
    });

    let results = join_all(futs).await;
    let mut all_records = prior_records;
    let mut new_records = Vec::new();
    for res in results {
        let rec = res?;
        all_records.push(rec.clone());
        new_records.push(rec);
    }

    if let Some(parent) = output_path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let mut file = fs::OpenOptions::new()
        .create(true)
        .write(true)
        .append(resume)
        .truncate(!resume)
        .open(output_path)
        .map_err(|e| AgentError::RoleAlternation(format!("batch output write failed: {e}")))?;

    for rec in &new_records {
        let encoded = serde_json::to_string(rec)
            .map_err(|e| AgentError::RoleAlternation(format!("serialize failed: {e}")))?;
        writeln!(file, "{encoded}")
            .map_err(|e| AgentError::RoleAlternation(format!("write failed: {e}")))?;
    }

    Ok(all_records)
}
