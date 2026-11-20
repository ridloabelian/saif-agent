use saif_config::{SaifConfig, SecretScope};
use saif_providers::{AuxiliaryClient, AuxiliaryTask, ChatMessage, MessageRole};

pub const SUMMARY_PREFIX: &str =
    "[CONTEXT COMPACTION SUMMARY — Do NOT answer this directly; resume the active task from this state.]\n\n";

#[derive(Debug, Clone)]
pub struct CompressionResult {
    pub messages: Vec<ChatMessage>,
    pub pruned_tool_outputs: usize,
    pub summarized_messages: usize,
    pub used_llm_summary: bool,
    pub summary_text: Option<String>,
}

pub fn estimate_tokens_rough(messages: &[ChatMessage]) -> usize {
    let chars: usize = messages
        .iter()
        .map(|m| m.content.len() + m.reasoning.as_ref().map(|r| r.len()).unwrap_or(0) + 16)
        .sum();
    chars / 4
}

/// 2-tier context compressor (`agent/context_compressor.py` parity):
/// 1. Prunes bulky old tool outputs (zero LLM calls).
/// 2. Preserves system[0] + tool_call/tool_result pairs in the tail; summarizes middle turns
///    via `AuxiliaryClient` with deterministic fallback on network/stream errors.
pub struct ContextCompressor;

impl ContextCompressor {
    pub async fn compress(
        messages: &[ChatMessage],
        keep_tail_tokens: usize,
        cfg: Option<&SaifConfig>,
        secrets: Option<&SecretScope>,
    ) -> CompressionResult {
        if messages.len() <= 3 {
            return CompressionResult {
                messages: messages.to_vec(),
                pruned_tool_outputs: 0,
                summarized_messages: 0,
                used_llm_summary: false,
                summary_text: None,
            };
        }

        let mut working = messages.to_vec();
        let mut pruned_tool_outputs = 0usize;

        // Tier 1: Prune oversized old tool outputs (never touching the last 4 messages).
        let protect_tail = working.len().saturating_sub(4).max(1);
        for msg in &mut working[1..protect_tail] {
            if msg.role == MessageRole::Tool && msg.content.len() > 600 {
                let preview: String = msg.content.chars().take(240).collect();
                msg.content = format!(
                    "{preview}\n[PRUNED TOOL OUTPUT ({} chars elided)]",
                    msg.content.len()
                );
                pruned_tool_outputs += 1;
            }
        }

        // Find split index keeping at least `keep_tail_tokens` and never breaking tool pairs.
        let mut tail_tokens = 0usize;
        let mut split_idx = working.len();
        for idx in (1..working.len()).rev() {
            tail_tokens += (working[idx].content.len() + 16) / 4;
            split_idx = idx;
            if tail_tokens >= keep_tail_tokens && working[idx].role == MessageRole::User {
                break;
            }
        }
        // Advance split_idx if it lands inside a Tool reply sequence.
        while split_idx < working.len() && working[split_idx].role == MessageRole::Tool {
            split_idx += 1;
        }

        if split_idx <= 2 {
            return CompressionResult {
                messages: working,
                pruned_tool_outputs,
                summarized_messages: 0,
                used_llm_summary: false,
                summary_text: None,
            };
        }

        let middle = &working[1..split_idx];
        let summarized_messages = middle.len();
        let transcript_slice: String = middle
            .iter()
            .map(|m| {
                format!(
                    "{:?}: {}",
                    m.role,
                    m.content.chars().take(400).collect::<String>()
                )
            })
            .collect::<Vec<_>>()
            .join("\n");

        let mut used_llm_summary = false;
        let summary_body = if let (Some(c), Some(s)) = (cfg, secrets) {
            let aux = AuxiliaryClient::new();
            match aux
                .run_prompt(
                    c,
                    s,
                    AuxiliaryTask::Compression,
                    "Summarize the conversation state concisely: goals, files touched, decisions made, and open work.",
                    &transcript_slice,
                    Some(1024),
                )
                .await
            {
                Ok(resp) if !resp.content.trim().is_empty() => {
                    used_llm_summary = true;
                    resp.content
                }
                _ => deterministic_fallback_summary(middle),
            }
        } else {
            deterministic_fallback_summary(middle)
        };

        let summary_text = format!("{SUMMARY_PREFIX}{summary_body}");
        let mut next = Vec::with_capacity(2 + (working.len() - split_idx));
        next.push(working[0].clone()); // Preserve byte-stable system[0]
        if working
            .get(split_idx)
            .is_some_and(|m| m.role == MessageRole::User)
        {
            let mut coalesced = summary_text.clone();
            coalesced.push_str("\n\n");
            coalesced.push_str(&working[split_idx].content);
            next.push(ChatMessage::user(coalesced));
            next.extend_from_slice(&working[split_idx + 1..]);
        } else {
            next.push(ChatMessage::user(summary_text.clone()));
            next.extend_from_slice(&working[split_idx..]);
        }

        CompressionResult {
            messages: next,
            pruned_tool_outputs,
            summarized_messages,
            used_llm_summary,
            summary_text: Some(summary_text),
        }
    }
}

fn deterministic_fallback_summary(middle: &[ChatMessage]) -> String {
    let mut user_goals = Vec::new();
    let mut tools_used = Vec::new();
    for m in middle {
        match m.role {
            MessageRole::User if user_goals.len() < 5 => {
                user_goals.push(m.content.chars().take(140).collect::<String>());
            }
            MessageRole::Tool => {
                if let Some(name) = &m.tool_name {
                    if !tools_used.contains(name) {
                        tools_used.push(name.clone());
                    }
                }
            }
            _ => {}
        }
    }
    format!(
        "Deterministic compaction summary ({} messages):\n- User requests: {}\n- Tools invoked: {}",
        middle.len(),
        user_goals.join(" | "),
        if tools_used.is_empty() {
            "none".to_string()
        } else {
            tools_used.join(", ")
        }
    )
}
