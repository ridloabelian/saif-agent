use crate::catalog::{get_provider_profile, resolve_provider_route, ResolvedRoute};
use crate::transport::{dispatch_completion, TransportRouter};
use crate::types::{ChatMessage, LlmRequest, LlmResponse, ProviderError};
use saif_config::{AuxiliaryTaskConfig, SaifConfig, SecretScope};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuxiliaryTask {
    Compression,
    TitleGen,
    SessionSearch,
    Curator,
    Vision,
}

/// Side-LLM client (`agent/auxiliary_client.py` parity) that resolves per-task models
/// independently of the main turn loop without emitting main `pre/post_api_request` hooks.
#[derive(Clone)]
pub struct AuxiliaryClient {
    router: TransportRouter,
}

impl Default for AuxiliaryClient {
    fn default() -> Self {
        Self::new()
    }
}

impl AuxiliaryClient {
    pub fn new() -> Self {
        Self {
            router: TransportRouter::new(),
        }
    }

    pub fn resolve_route(
        &self,
        cfg: &SaifConfig,
        secrets: &SecretScope,
        task: AuxiliaryTask,
    ) -> Result<ResolvedRoute, ProviderError> {
        let task_cfg: &AuxiliaryTaskConfig = match task {
            AuxiliaryTask::Compression => &cfg.auxiliary.compression,
            AuxiliaryTask::TitleGen => &cfg.auxiliary.title_gen,
            AuxiliaryTask::SessionSearch => &cfg.auxiliary.session_search,
            AuxiliaryTask::Curator => &cfg.auxiliary.curator,
            AuxiliaryTask::Vision => &cfg.auxiliary.vision,
        };

        let base_route = resolve_provider_route(
            cfg,
            secrets,
            task_cfg.provider.as_deref(),
            task_cfg.model.as_deref(),
            task_cfg.base_url.as_deref(),
        )?;

        if task_cfg.model.is_none() {
            if let Some(prof) = get_provider_profile(&base_route.provider_id) {
                return Ok(ResolvedRoute {
                    model: prof.default_aux_model.to_string(),
                    ..base_route
                });
            }
        }
        Ok(base_route)
    }

    pub async fn run_prompt(
        &self,
        cfg: &SaifConfig,
        secrets: &SecretScope,
        task: AuxiliaryTask,
        system_prompt: &str,
        user_prompt: &str,
        max_tokens: Option<u32>,
    ) -> Result<LlmResponse, ProviderError> {
        let route = self.resolve_route(cfg, secrets, task)?;
        let req = LlmRequest {
            model: route.model.clone(),
            messages: vec![
                ChatMessage::system(system_prompt),
                ChatMessage::user(user_prompt),
            ],
            tools: Vec::new(),
            temperature: Some(0.1),
            max_tokens,
            reasoning_effort: None,
            stream: false,
            session_id: None,
        };
        dispatch_completion(&self.router, &route, &req, None).await
    }
}
