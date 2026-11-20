use crate::compressor::{estimate_tokens_rough, ContextCompressor};
use crate::prompt::{
    head_tail_truncate, scan_context_content, PromptBuilder, MAX_SUBDIR_HINT_CHARS,
};
use futures_util::future::BoxFuture;
use saif_config::{ProfileScope, SaifConfig};
use saif_persistence::{PersistenceStore, SqliteStore, ToolCallRecord, UsageTotals};
use saif_plugins::{HookEvent, HookPayload, HookRegistry};
use saif_providers::{
    dispatch_completion, resolve_provider_route, ChatMessage, LlmRequest, LlmTransport,
    MessageRole, ProviderError, StreamChunk, ToolCall, ToolDefinition, TransportRouter,
    UsageMetrics,
};
use saif_skills::SkillStore;
use saif_tools::{ToolContext, ToolRegistry};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Arc;
use thiserror::Error;
use tokio::sync::{mpsc, Semaphore};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum AgentError {
    #[error("provider error: {0}")]
    Provider(#[from] ProviderError),
    #[error("persistence error: {0}")]
    Persistence(#[from] saif_persistence::PersistenceError),
    #[error("role alternation invariant violated: {0}")]
    RoleAlternation(String),
    #[error("prompt cache invariant violated: system[0] mutated mid-conversation")]
    PromptCacheBroken,
    #[error("interrupted")]
    Interrupted,
}

#[derive(Debug, Clone)]
pub enum AgentEvent {
    SessionStarted {
        session_id: String,
    },
    ThinkingDelta(String),
    ContentDelta(String),
    ToolStart {
        name: String,
        arguments: Value,
    },
    ToolComplete {
        name: String,
        output_preview: String,
        ok: bool,
    },
    Compacted {
        pruned: usize,
        summarized: usize,
    },
    Usage(UsageMetrics),
    FinalResponse(String),
}

#[derive(Debug, Clone)]
pub struct TurnOptions {
    pub session_id: Option<String>,
    pub source: String,
    pub model_override: Option<String>,
    pub provider_override: Option<String>,
    pub base_url_override: Option<String>,
    pub extra_system_prompt: Option<String>,
    pub max_iterations: Option<usize>,
    pub spawn_depth: usize,
    pub is_delegated_child: bool,
    pub skip_persistence: bool,
    pub cwd: PathBuf,
}

impl Default for TurnOptions {
    fn default() -> Self {
        Self {
            session_id: None,
            source: "cli".to_string(),
            model_override: None,
            provider_override: None,
            base_url_override: None,
            extra_system_prompt: None,
            max_iterations: None,
            spawn_depth: 0,
            is_delegated_child: false,
            skip_persistence: false,
            cwd: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        }
    }
}

#[derive(Debug, Clone)]
pub struct AgentOutcome {
    pub session_id: String,
    pub final_response: String,
    pub messages: Vec<ChatMessage>,
    pub usage: UsageMetrics,
    pub api_calls: usize,
}

/// Validates Hermes strict role alternation invariant (`agent/AGENTS.md`):
/// `System` only at index 0; no two consecutive same-role messages unless `Tool -> Tool`,
/// and `/steer` (`Assistant(tool_calls) -> Tool+ -> User`) is valid.
pub fn validate_role_alternation(messages: &[ChatMessage]) -> Result<(), AgentError> {
    if messages.is_empty() {
        return Ok(());
    }
    if messages[0].role != MessageRole::System {
        return Err(AgentError::RoleAlternation(
            "messages[0] must have role=System".to_string(),
        ));
    }
    for pair in messages.windows(2) {
        let prev = &pair[0];
        let curr = &pair[1];
        if curr.role == MessageRole::System {
            return Err(AgentError::RoleAlternation(
                "System message must not appear after index 0".to_string(),
            ));
        }
        if prev.role == curr.role && prev.role != MessageRole::Tool {
            return Err(AgentError::RoleAlternation(format!(
                "Consecutive {:?} messages are forbidden",
                curr.role
            )));
        }
        if curr.role == MessageRole::Tool
            && !matches!(prev.role, MessageRole::Assistant | MessageRole::Tool)
        {
            return Err(AgentError::RoleAlternation(
                "Tool message must follow Assistant(tool_calls) or Tool".to_string(),
            ));
        }
    }
    Ok(())
}

#[derive(Clone)]
pub struct AgentRunner {
    profile: Arc<ProfileScope>,
    config: Arc<SaifConfig>,
    registry: ToolRegistry,
    store: Option<SqliteStore>,
    hooks: HookRegistry,
    transports: TransportRouter,
    custom_transport: Option<Arc<dyn LlmTransport>>,
    delegation_semaphore: Arc<Semaphore>,
}

impl AgentRunner {
    pub fn new(
        profile: Arc<ProfileScope>,
        config: Arc<SaifConfig>,
        registry: ToolRegistry,
        store: Option<SqliteStore>,
        hooks: HookRegistry,
    ) -> Self {
        let max_children = config.delegation.max_concurrent_children.max(1);
        Self {
            profile,
            config,
            registry,
            store,
            hooks,
            transports: TransportRouter::new(),
            custom_transport: None,
            delegation_semaphore: Arc::new(Semaphore::new(max_children)),
        }
    }

    pub fn with_custom_transport(mut self, transport: Arc<dyn LlmTransport>) -> Self {
        self.custom_transport = Some(transport);
        self
    }

    /// Resolve `/skill <name> ...` into a user-role prompt message without altering `system[0]`.
    pub fn maybe_expand_skill_slash(&self, input: &str) -> String {
        let trimmed = input.trim();
        let Some(rest) = trimmed.strip_prefix("/skill ") else {
            return input.to_string();
        };
        let mut parts = rest.splitn(2, char::is_whitespace);
        let name = parts.next().unwrap_or("").trim();
        let extra = parts.next().unwrap_or("").trim();
        let skill_store = SkillStore::new(self.profile.skills_dir(), &self.config.skills.disabled);
        if let Ok(doc) = skill_store.view_skill(name) {
            format!(
                "[Skill Command: {name}]\n{}\n\nUser instruction: {extra}",
                doc.raw
            )
        } else {
            input.to_string()
        }
    }

    pub fn run_conversation<'a>(
        &'a self,
        user_message: &'a str,
        prior_messages: Option<Vec<ChatMessage>>,
        opts: TurnOptions,
        event_tx: Option<mpsc::UnboundedSender<AgentEvent>>,
        cancel: CancellationToken,
        mut steer_rx: Option<&'a mut mpsc::UnboundedReceiver<String>>,
    ) -> BoxFuture<'a, Result<AgentOutcome, AgentError>> {
        Box::pin(async move {
            let session_id = opts
                .session_id
                .clone()
                .unwrap_or_else(|| format!("sess-{}", &Uuid::new_v4().to_string()[..12]));

            if let Some(tx) = &event_tx {
                let _ = tx.send(AgentEvent::SessionStarted {
                    session_id: session_id.clone(),
                });
            }

            let route = resolve_provider_route(
                &self.config,
                &self.profile.secrets,
                opts.provider_override.as_deref(),
                opts.model_override.as_deref(),
                opts.base_url_override.as_deref(),
            )?;

            let mut messages = match prior_messages {
                Some(mut existing) if !existing.is_empty() => {
                    existing.push(ChatMessage::user(
                        self.maybe_expand_skill_slash(user_message),
                    ));
                    existing
                }
                _ => {
                    let sys = PromptBuilder::build_system_prompt(
                        &self.profile,
                        &self.config,
                        &opts.cwd,
                        opts.extra_system_prompt.as_deref(),
                    )
                    .await;
                    if !opts.skip_persistence {
                        if let Some(store) = &self.store {
                            let _ = store
                                .create_session(
                                    &session_id,
                                    &opts.source,
                                    Some(&route.model),
                                    Some(&sys),
                                    Some(&opts.cwd.to_string_lossy()),
                                    Some(&self.profile.profile_name),
                                )
                                .await;
                        }
                    }
                    vec![
                        ChatMessage::system(sys),
                        ChatMessage::user(self.maybe_expand_skill_slash(user_message)),
                    ]
                }
            };

            let frozen_system_prompt = messages[0].content.clone();

            if !opts.skip_persistence {
                if let Some(store) = &self.store {
                    let _ = store
                        .append_message(
                            &session_id,
                            "user",
                            messages.last().map(|m| m.content.as_str()),
                            None,
                            None,
                            None,
                            None,
                            None,
                            None,
                        )
                        .await;
                }
            }

            let tool_ctx = ToolContext {
                session_id: session_id.clone(),
                cwd: opts.cwd.clone(),
                profile: Arc::clone(&self.profile),
                config: Arc::clone(&self.config),
                store: self.store.clone(),
                kanban: None,
                task_id: None,
                spawn_depth: opts.spawn_depth,
                is_delegated_child: opts.is_delegated_child,
            };

            // Freeze tool schema bytes at session start to uphold prompt-caching invariant.
            let frozen_tools: Vec<ToolDefinition> = self.registry.get_definitions(&tool_ctx, &[]);
            let max_iters = opts
                .max_iterations
                .unwrap_or(self.config.agent.max_iterations)
                .max(1);

            let mut total_usage = UsageMetrics::default();
            let mut api_calls = 0usize;
            let mut grace_used = false;
            let mut final_response = String::new();

            self.hooks
                .emit(HookPayload {
                    event: HookEvent::OnSessionStart,
                    session_id: Some(session_id.clone()),
                    profile: self.profile.profile_name.clone(),
                    data: json!({ "source": opts.source, "model": route.model }),
                })
                .await;

            loop {
                if cancel.is_cancelled() {
                    return Err(AgentError::Interrupted);
                }

                // Invariant guards before every model call
                if messages[0].content != frozen_system_prompt {
                    return Err(AgentError::PromptCacheBroken);
                }
                validate_role_alternation(&messages)?;

                // Optional context compression when rough estimate crosses threshold
                let approx_tokens = estimate_tokens_rough(&messages);
                let trigger_tokens =
                    ((128_000_f64) * self.config.compression.threshold_ratio) as usize;
                if self.config.compression.enabled && approx_tokens > trigger_tokens {
                    let compacted = ContextCompressor::compress(
                        &messages,
                        self.config.compression.keep_tail_tokens,
                        Some(&self.config),
                        Some(&self.profile.secrets),
                    )
                    .await;
                    messages = compacted.messages;
                    if let Some(tx) = &event_tx {
                        let _ = tx.send(AgentEvent::Compacted {
                            pruned: compacted.pruned_tool_outputs,
                            summarized: compacted.summarized_messages,
                        });
                    }
                }

                let is_grace_turn = api_calls >= max_iters;
                let tools_for_call = if is_grace_turn {
                    Vec::new()
                } else {
                    frozen_tools.clone()
                };

                self.hooks
                    .emit(HookPayload {
                        event: HookEvent::PreLlmCall,
                        session_id: Some(session_id.clone()),
                        profile: self.profile.profile_name.clone(),
                        data: json!({ "iteration": api_calls }),
                    })
                    .await;

                let req = LlmRequest {
                    model: route.model.clone(),
                    messages: messages.clone(),
                    tools: tools_for_call,
                    temperature: (!route.omit_temperature).then_some(0.2),
                    max_tokens: None,
                    reasoning_effort: self.config.agent.reasoning_effort.clone(),
                    stream: event_tx.is_some(),
                    session_id: Some(session_id.clone()),
                };

                let (chunk_tx, mut chunk_rx) = mpsc::unbounded_channel();
                let forward_handle = event_tx.clone().map(|ev| {
                    tokio::spawn(async move {
                        while let Some(chunk) = chunk_rx.recv().await {
                            match chunk {
                                StreamChunk::ContentDelta(d) => {
                                    let _ = ev.send(AgentEvent::ContentDelta(d));
                                }
                                StreamChunk::ReasoningDelta(r) => {
                                    let _ = ev.send(AgentEvent::ThinkingDelta(r));
                                }
                                StreamChunk::Done(_) => break,
                            }
                        }
                    })
                });

                let tx_opt = event_tx.as_ref().map(|_| chunk_tx);
                let resp = if let Some(custom) = &self.custom_transport {
                    custom.complete(&route, &req, tx_opt).await?
                } else {
                    dispatch_completion(&self.transports, &route, &req, tx_opt).await?
                };
                if let Some(h) = forward_handle {
                    let _ = h.await;
                }

                api_calls += 1;
                total_usage.input_tokens += resp.usage.input_tokens;
                total_usage.output_tokens += resp.usage.output_tokens;
                total_usage.cache_read_tokens += resp.usage.cache_read_tokens;
                total_usage.cache_write_tokens += resp.usage.cache_write_tokens;
                total_usage.reasoning_tokens += resp.usage.reasoning_tokens;

                if let Some(tx) = &event_tx {
                    let _ = tx.send(AgentEvent::Usage(resp.usage.clone()));
                }

                if !opts.skip_persistence {
                    if let Some(store) = &self.store {
                        let tc_records: Option<Vec<ToolCallRecord>> = (!resp.tool_calls.is_empty())
                            .then(|| {
                                resp.tool_calls
                                    .iter()
                                    .map(|tc| ToolCallRecord {
                                        id: tc.id.clone(),
                                        name: tc.name.clone(),
                                        arguments: tc.arguments.clone(),
                                    })
                                    .collect()
                            });
                        let _ = store
                            .append_message(
                                &session_id,
                                "assistant",
                                Some(&resp.content),
                                None,
                                None,
                                tc_records.as_deref(),
                                resp.reasoning.as_deref(),
                                resp.finish_reason.as_deref(),
                                None,
                            )
                            .await;
                        let _ = store
                            .record_usage(
                                &session_id,
                                &route.model,
                                &UsageTotals {
                                    input_tokens: resp.usage.input_tokens,
                                    output_tokens: resp.usage.output_tokens,
                                    cache_read_tokens: resp.usage.cache_read_tokens,
                                    cache_write_tokens: resp.usage.cache_write_tokens,
                                    reasoning_tokens: resp.usage.reasoning_tokens,
                                    api_calls: 1,
                                },
                            )
                            .await;
                    }
                }

                self.hooks
                    .emit(HookPayload {
                        event: HookEvent::PostLlmCall,
                        session_id: Some(session_id.clone()),
                        profile: self.profile.profile_name.clone(),
                        data: json!({ "tool_calls": resp.tool_calls.len() }),
                    })
                    .await;

                let has_tools = !resp.tool_calls.is_empty();
                messages.push(ChatMessage::assistant(
                    resp.content.clone(),
                    has_tools.then_some(resp.tool_calls.clone()),
                    resp.reasoning.clone(),
                ));

                if !has_tools || is_grace_turn {
                    final_response = resp.content;
                    break;
                }

                for tc in resp.tool_calls {
                    let out = self
                        .execute_tool_call(&tc, &tool_ctx, &opts, event_tx.as_ref())
                        .await;
                    messages.push(ChatMessage::tool(
                        tc.id.clone(),
                        tc.name.clone(),
                        out.clone(),
                    ));

                    if !opts.skip_persistence {
                        if let Some(store) = &self.store {
                            let _ = store
                                .append_message(
                                    &session_id,
                                    "tool",
                                    Some(&out),
                                    Some(&tc.name),
                                    Some(&tc.id),
                                    None,
                                    None,
                                    None,
                                    None,
                                )
                                .await;
                        }
                    }
                }

                // Deliver `/steer` strictly as a standalone user row AFTER tool results.
                if let Some(rx) = steer_rx.as_deref_mut() {
                    while let Ok(steer_text) = rx.try_recv() {
                        messages.push(ChatMessage::user(steer_text.clone()));
                        if !opts.skip_persistence {
                            if let Some(store) = &self.store {
                                let _ = store
                                    .append_message(
                                        &session_id,
                                        "user",
                                        Some(&steer_text),
                                        None,
                                        None,
                                        None,
                                        None,
                                        None,
                                        Some("steer"),
                                    )
                                    .await;
                            }
                        }
                    }
                }

                if api_calls >= max_iters && !grace_used {
                    grace_used = true;
                    continue;
                }
                if api_calls > max_iters {
                    break;
                }
            }

            if let Some(tx) = &event_tx {
                let _ = tx.send(AgentEvent::FinalResponse(final_response.clone()));
            }
            self.hooks
                .emit(HookPayload {
                    event: HookEvent::OnSessionEnd,
                    session_id: Some(session_id.clone()),
                    profile: self.profile.profile_name.clone(),
                    data: json!({ "api_calls": api_calls }),
                })
                .await;

            Ok(AgentOutcome {
                session_id,
                final_response,
                messages,
                usage: total_usage,
                api_calls,
            })
        })
    }

    async fn execute_tool_call(
        &self,
        tc: &ToolCall,
        ctx: &ToolContext,
        opts: &TurnOptions,
        event_tx: Option<&mpsc::UnboundedSender<AgentEvent>>,
    ) -> String {
        if let Some(tx) = event_tx {
            let _ = tx.send(AgentEvent::ToolStart {
                name: tc.name.clone(),
                arguments: tc.arguments.clone(),
            });
        }

        self.hooks
            .emit(HookPayload {
                event: HookEvent::PreToolCall,
                session_id: Some(ctx.session_id.clone()),
                profile: self.profile.profile_name.clone(),
                data: json!({ "tool": tc.name, "arguments": tc.arguments }),
            })
            .await;

        let result = if tc.name == "delegate_task" {
            self.handle_delegate_task(&tc.arguments, opts).await
        } else {
            self.registry
                .dispatch(&tc.name, tc.arguments.clone(), ctx)
                .await
        };

        let (ok, mut output) = match result {
            Ok(s) => (true, s),
            Err(e) => (
                false,
                json!({ "success": false, "error": e.to_string() }).to_string(),
            ),
        };

        // Append subdirectory AGENTS.md hint onto tool result if file tool targeted a nested dir
        if let Some(path_str) = tc.arguments.get("path").and_then(|v| v.as_str()) {
            let target_path = ctx.cwd.join(path_str);
            if let Some(dir) = target_path.parent() {
                if dir != ctx.cwd && dir.starts_with(&ctx.cwd) {
                    let hint_file = dir.join("AGENTS.md");
                    if hint_file.is_file() {
                        if let Ok(raw) = std::fs::read_to_string(&hint_file) {
                            let safe = scan_context_content(&raw, "AGENTS.md", false);
                            let capped = head_tail_truncate(&safe, MAX_SUBDIR_HINT_CHARS);
                            output
                                .push_str(&format!("\n\n[Subdirectory AGENTS.md hint]\n{capped}"));
                        }
                    }
                }
            }
        }

        self.hooks
            .emit(HookPayload {
                event: HookEvent::PostToolCall,
                session_id: Some(ctx.session_id.clone()),
                profile: self.profile.profile_name.clone(),
                data: json!({ "tool": tc.name, "ok": ok }),
            })
            .await;

        if let Some(tx) = event_tx {
            let preview: String = output.chars().take(160).collect();
            let _ = tx.send(AgentEvent::ToolComplete {
                name: tc.name.clone(),
                output_preview: preview,
                ok,
            });
        }
        output
    }

    async fn handle_delegate_task(
        &self,
        args: &Value,
        parent_opts: &TurnOptions,
    ) -> Result<String, saif_tools::ToolError> {
        if parent_opts.spawn_depth >= self.config.delegation.max_spawn_depth {
            return Err(saif_tools::ToolError::Execution(format!(
                "max_spawn_depth ({}) reached",
                self.config.delegation.max_spawn_depth
            )));
        }
        let goal = args
            .get("goal")
            .and_then(|v| v.as_str())
            .ok_or_else(|| saif_tools::ToolError::InvalidArgs("missing 'goal'".to_string()))?;

        let _permit = self
            .delegation_semaphore
            .acquire()
            .await
            .map_err(|e| saif_tools::ToolError::Execution(e.to_string()))?;

        let child_opts = TurnOptions {
            session_id: None,
            source: "delegation".to_string(),
            model_override: parent_opts.model_override.clone(),
            provider_override: parent_opts.provider_override.clone(),
            base_url_override: parent_opts.base_url_override.clone(),
            extra_system_prompt: None,
            max_iterations: Some(self.config.delegation.max_iterations),
            spawn_depth: parent_opts.spawn_depth + 1,
            is_delegated_child: true,
            skip_persistence: parent_opts.skip_persistence,
            cwd: parent_opts.cwd.clone(),
        };

        let outcome = self
            .run_conversation(goal, None, child_opts, None, CancellationToken::new(), None)
            .await
            .map_err(|e| saif_tools::ToolError::Execution(e.to_string()))?;

        Ok(json!({
            "success": true,
            "child_session_id": outcome.session_id,
            "summary": outcome.final_response,
            "api_calls": outcome.api_calls
        })
        .to_string())
    }
}
