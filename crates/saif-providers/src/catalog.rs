use crate::credential_pool::{read_auth_json_secret, CredentialPool};
use crate::types::ProviderError;
use saif_config::{SaifConfig, SecretScope};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ApiMode {
    ChatCompletions,
    AnthropicMessages,
    CodexResponses,
    Bedrock,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AuthType {
    ApiKey,
    OAuth,
    None,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderProfile {
    pub id: &'static str,
    pub display_name: &'static str,
    pub base_url: &'static str,
    pub api_mode: ApiMode,
    pub auth_type: AuthType,
    pub env_var: &'static str,
    pub default_model: &'static str,
    pub default_aux_model: &'static str,
    pub omit_temperature: bool,
    pub supports_prompt_caching: bool,
}

#[derive(Debug, Clone)]
pub struct ResolvedRoute {
    pub provider_id: String,
    pub base_url: String,
    pub api_mode: ApiMode,
    pub model: String,
    pub api_key: Option<String>,
    pub omit_temperature: bool,
    pub supports_prompt_caching: bool,
    pub multiplex_active: bool,
    pub pooled_credential_id: Option<String>,
    pub saif_home: Option<PathBuf>,
    pub aws_region: Option<String>,
    pub aws_access_key_id: Option<String>,
    pub aws_secret_access_key: Option<String>,
    pub aws_session_token: Option<String>,
    pub aws_profile: Option<String>,
}

pub fn builtin_providers() -> Vec<ProviderProfile> {
    vec![
        ProviderProfile {
            id: "openrouter",
            display_name: "OpenRouter",
            base_url: "https://openrouter.ai/api/v1",
            api_mode: ApiMode::ChatCompletions,
            auth_type: AuthType::ApiKey,
            env_var: "OPENROUTER_API_KEY",
            default_model: "anthropic/claude-sonnet-4",
            default_aux_model: "google/gemini-2.5-flash",
            omit_temperature: false,
            supports_prompt_caching: true,
        },
        ProviderProfile {
            id: "nous",
            display_name: "Nous Portal",
            base_url: "https://inference-api.nousresearch.com/v1",
            api_mode: ApiMode::ChatCompletions,
            auth_type: AuthType::OAuth,
            env_var: "NOUS_API_KEY",
            default_model: "hermes-4-405b",
            default_aux_model: "hermes-4-70b",
            omit_temperature: false,
            supports_prompt_caching: false,
        },
        ProviderProfile {
            id: "openai",
            display_name: "OpenAI",
            base_url: "https://api.openai.com/v1",
            api_mode: ApiMode::ChatCompletions,
            auth_type: AuthType::ApiKey,
            env_var: "OPENAI_API_KEY",
            default_model: "gpt-4.1",
            default_aux_model: "gpt-4.1-mini",
            omit_temperature: false,
            supports_prompt_caching: true,
        },
        ProviderProfile {
            id: "openai-codex",
            display_name: "OpenAI Codex",
            base_url: "https://chatgpt.com/backend-api/codex",
            api_mode: ApiMode::CodexResponses,
            auth_type: AuthType::OAuth,
            env_var: "OPENAI_API_KEY",
            default_model: "gpt-5.3-codex",
            default_aux_model: "gpt-5-codex-mini",
            omit_temperature: true,
            supports_prompt_caching: true,
        },
        ProviderProfile {
            id: "anthropic",
            display_name: "Anthropic",
            base_url: "https://api.anthropic.com",
            api_mode: ApiMode::AnthropicMessages,
            auth_type: AuthType::ApiKey,
            env_var: "ANTHROPIC_API_KEY",
            default_model: "claude-sonnet-4-20250514",
            default_aux_model: "claude-3-5-haiku-20241022",
            omit_temperature: false,
            supports_prompt_caching: true,
        },
        ProviderProfile {
            id: "gemini",
            display_name: "Google Gemini",
            base_url: "https://generativelanguage.googleapis.com/v1beta/openai",
            api_mode: ApiMode::ChatCompletions,
            auth_type: AuthType::ApiKey,
            env_var: "GEMINI_API_KEY",
            default_model: "gemini-2.5-pro",
            default_aux_model: "gemini-2.5-flash",
            omit_temperature: false,
            supports_prompt_caching: true,
        },
        ProviderProfile {
            id: "deepseek",
            display_name: "DeepSeek",
            base_url: "https://api.deepseek.com/v1",
            api_mode: ApiMode::ChatCompletions,
            auth_type: AuthType::ApiKey,
            env_var: "DEEPSEEK_API_KEY",
            default_model: "deepseek-chat",
            default_aux_model: "deepseek-chat",
            omit_temperature: false,
            supports_prompt_caching: true,
        },
        ProviderProfile {
            id: "alibaba",
            display_name: "Alibaba Qwen (DashScope)",
            base_url: "https://dashscope-intl.aliyuncs.com/compatible-mode/v1",
            api_mode: ApiMode::ChatCompletions,
            auth_type: AuthType::ApiKey,
            env_var: "DASHSCOPE_API_KEY",
            default_model: "qwen3-coder-plus",
            default_aux_model: "qwen-turbo",
            omit_temperature: false,
            supports_prompt_caching: true,
        },
        ProviderProfile {
            id: "xai",
            display_name: "xAI Grok",
            base_url: "https://api.x.ai/v1",
            api_mode: ApiMode::ChatCompletions,
            auth_type: AuthType::ApiKey,
            env_var: "XAI_API_KEY",
            default_model: "grok-3",
            default_aux_model: "grok-3-mini",
            omit_temperature: false,
            supports_prompt_caching: false,
        },
        ProviderProfile {
            id: "groq",
            display_name: "Groq",
            base_url: "https://api.groq.com/openai/v1",
            api_mode: ApiMode::ChatCompletions,
            auth_type: AuthType::ApiKey,
            env_var: "GROQ_API_KEY",
            default_model: "llama-3.3-70b-versatile",
            default_aux_model: "llama-3.1-8b-instant",
            omit_temperature: false,
            supports_prompt_caching: false,
        },
        ProviderProfile {
            id: "fireworks",
            display_name: "Fireworks AI",
            base_url: "https://api.fireworks.ai/inference/v1",
            api_mode: ApiMode::ChatCompletions,
            auth_type: AuthType::ApiKey,
            env_var: "FIREWORKS_API_KEY",
            default_model: "accounts/fireworks/models/deepseek-v3",
            default_aux_model: "accounts/fireworks/models/llama-v3p1-8b-instruct",
            omit_temperature: false,
            supports_prompt_caching: false,
        },
        ProviderProfile {
            id: "together",
            display_name: "Together AI",
            base_url: "https://api.together.xyz/v1",
            api_mode: ApiMode::ChatCompletions,
            auth_type: AuthType::ApiKey,
            env_var: "TOGETHER_API_KEY",
            default_model: "meta-llama/Llama-3.3-70B-Instruct-Turbo",
            default_aux_model: "meta-llama/Meta-Llama-3.1-8B-Instruct-Turbo",
            omit_temperature: false,
            supports_prompt_caching: false,
        },
        ProviderProfile {
            id: "deepinfra",
            display_name: "DeepInfra",
            base_url: "https://api.deepinfra.com/v1/openai",
            api_mode: ApiMode::ChatCompletions,
            auth_type: AuthType::ApiKey,
            env_var: "DEEPINFRA_API_KEY",
            default_model: "deepseek-ai/DeepSeek-V3",
            default_aux_model: "meta-llama/Llama-3.2-3B-Instruct",
            omit_temperature: false,
            supports_prompt_caching: false,
        },
        ProviderProfile {
            id: "nebius",
            display_name: "Nebius Token Factory",
            base_url: "https://api.studio.nebius.com/v1",
            api_mode: ApiMode::ChatCompletions,
            auth_type: AuthType::ApiKey,
            env_var: "NEBIUS_API_KEY",
            default_model: "deepseek-ai/DeepSeek-R1",
            default_aux_model: "meta-llama/Meta-Llama-3.1-8B-Instruct",
            omit_temperature: false,
            supports_prompt_caching: false,
        },
        ProviderProfile {
            id: "nvidia",
            display_name: "NVIDIA NIM",
            base_url: "https://integrate.api.nvidia.com/v1",
            api_mode: ApiMode::ChatCompletions,
            auth_type: AuthType::ApiKey,
            env_var: "NVIDIA_API_KEY",
            default_model: "nvidia/llama-3.3-nemotron-super-49b-v1",
            default_aux_model: "meta/llama-3.1-8b-instruct",
            omit_temperature: false,
            supports_prompt_caching: false,
        },
        ProviderProfile {
            id: "mistral",
            display_name: "Mistral",
            base_url: "https://api.mistral.ai/v1",
            api_mode: ApiMode::ChatCompletions,
            auth_type: AuthType::ApiKey,
            env_var: "MISTRAL_API_KEY",
            default_model: "mistral-large-latest",
            default_aux_model: "mistral-small-latest",
            omit_temperature: false,
            supports_prompt_caching: false,
        },
        ProviderProfile {
            id: "kimi-coding",
            display_name: "Moonshot Kimi",
            base_url: "https://api.moonshot.ai/v1",
            api_mode: ApiMode::ChatCompletions,
            auth_type: AuthType::ApiKey,
            env_var: "MOONSHOT_API_KEY",
            default_model: "kimi-k2.5",
            default_aux_model: "moonshot-v1-8k",
            omit_temperature: true,
            supports_prompt_caching: false,
        },
        ProviderProfile {
            id: "minimax",
            display_name: "MiniMax",
            base_url: "https://api.minimax.io/v1",
            api_mode: ApiMode::ChatCompletions,
            auth_type: AuthType::ApiKey,
            env_var: "MINIMAX_API_KEY",
            default_model: "MiniMax-M1",
            default_aux_model: "MiniMax-Text-01",
            omit_temperature: false,
            supports_prompt_caching: false,
        },
        ProviderProfile {
            id: "ollama",
            display_name: "Ollama Local",
            base_url: "http://127.0.0.1:11434/v1",
            api_mode: ApiMode::ChatCompletions,
            auth_type: AuthType::None,
            env_var: "OLLAMA_API_KEY",
            default_model: "qwen2.5-coder:14b",
            default_aux_model: "qwen2.5:3b",
            omit_temperature: false,
            supports_prompt_caching: false,
        },
        ProviderProfile {
            id: "lmstudio",
            display_name: "LM Studio Local",
            base_url: "http://127.0.0.1:1234/v1",
            api_mode: ApiMode::ChatCompletions,
            auth_type: AuthType::None,
            env_var: "LMSTUDIO_API_KEY",
            default_model: "local-model",
            default_aux_model: "local-model",
            omit_temperature: false,
            supports_prompt_caching: false,
        },
        ProviderProfile {
            id: "bedrock",
            display_name: "AWS Bedrock Converse",
            base_url: "https://bedrock-runtime.us-east-1.amazonaws.com",
            api_mode: ApiMode::Bedrock,
            auth_type: AuthType::OAuth,
            env_var: "AWS_ACCESS_KEY_ID",
            default_model: "us.anthropic.claude-sonnet-4-20250514-v1:0",
            default_aux_model: "us.anthropic.claude-3-5-haiku-20241022-v1:0",
            omit_temperature: false,
            supports_prompt_caching: true,
        },
        ProviderProfile {
            id: "vertex",
            display_name: "Google Vertex AI",
            base_url: "https://us-central1-aiplatform.googleapis.com/v1/projects/default/locations/us-central1/endpoints/openapi",
            api_mode: ApiMode::ChatCompletions,
            auth_type: AuthType::OAuth,
            env_var: "VERTEX_API_KEY",
            default_model: "google/gemini-2.5-pro",
            default_aux_model: "google/gemini-2.5-flash",
            omit_temperature: false,
            supports_prompt_caching: true,
        },
        ProviderProfile {
            id: "azure-openai",
            display_name: "Azure OpenAI / Entra",
            base_url: "https://models.inference.ai.azure.com",
            api_mode: ApiMode::ChatCompletions,
            auth_type: AuthType::OAuth,
            env_var: "AZURE_OPENAI_API_KEY",
            default_model: "gpt-4.1",
            default_aux_model: "gpt-4.1-mini",
            omit_temperature: false,
            supports_prompt_caching: true,
        },
        ProviderProfile {
            id: "zai",
            display_name: "Z.AI / Zhipu GLM",
            base_url: "https://api.z.ai/api/paas/v4",
            api_mode: ApiMode::ChatCompletions,
            auth_type: AuthType::ApiKey,
            env_var: "ZAI_API_KEY",
            default_model: "glm-4.5",
            default_aux_model: "glm-4.5-air",
            omit_temperature: false,
            supports_prompt_caching: false,
        },
        ProviderProfile {
            id: "qwen-portal",
            display_name: "Qwen Portal OAuth",
            base_url: "https://portal.qwen.ai/v1",
            api_mode: ApiMode::ChatCompletions,
            auth_type: AuthType::OAuth,
            env_var: "QWEN_OAUTH_TOKEN",
            default_model: "qwen3-coder-plus",
            default_aux_model: "qwen-turbo",
            omit_temperature: false,
            supports_prompt_caching: false,
        },
        ProviderProfile {
            id: "cerebras",
            display_name: "Cerebras",
            base_url: "https://api.cerebras.ai/v1",
            api_mode: ApiMode::ChatCompletions,
            auth_type: AuthType::ApiKey,
            env_var: "CEREBRAS_API_KEY",
            default_model: "llama-3.3-70b",
            default_aux_model: "llama3.1-8b",
            omit_temperature: false,
            supports_prompt_caching: false,
        },
        ProviderProfile {
            id: "sambanova",
            display_name: "SambaNova Cloud",
            base_url: "https://api.sambanova.ai/v1",
            api_mode: ApiMode::ChatCompletions,
            auth_type: AuthType::ApiKey,
            env_var: "SAMBANOVA_API_KEY",
            default_model: "Meta-Llama-3.3-70B-Instruct",
            default_aux_model: "Meta-Llama-3.1-8B-Instruct",
            omit_temperature: false,
            supports_prompt_caching: false,
        },
        ProviderProfile {
            id: "chutes",
            display_name: "Chutes AI",
            base_url: "https://llm.chutes.ai/v1",
            api_mode: ApiMode::ChatCompletions,
            auth_type: AuthType::ApiKey,
            env_var: "CHUTES_API_KEY",
            default_model: "deepseek-ai/DeepSeek-V3-0324",
            default_aux_model: "Qwen/Qwen2.5-7B-Instruct",
            omit_temperature: false,
            supports_prompt_caching: false,
        },
        ProviderProfile {
            id: "hyperbolic",
            display_name: "Hyperbolic",
            base_url: "https://api.hyperbolic.xyz/v1",
            api_mode: ApiMode::ChatCompletions,
            auth_type: AuthType::ApiKey,
            env_var: "HYPERBOLIC_API_KEY",
            default_model: "deepseek-ai/DeepSeek-V3",
            default_aux_model: "meta-llama/Llama-3.2-3B-Instruct",
            omit_temperature: false,
            supports_prompt_caching: false,
        },
        ProviderProfile {
            id: "featherless",
            display_name: "Featherless AI",
            base_url: "https://api.featherless.ai/v1",
            api_mode: ApiMode::ChatCompletions,
            auth_type: AuthType::ApiKey,
            env_var: "FEATHERLESS_API_KEY",
            default_model: "meta-llama/Meta-Llama-3.1-70B-Instruct",
            default_aux_model: "meta-llama/Meta-Llama-3.1-8B-Instruct",
            omit_temperature: false,
            supports_prompt_caching: false,
        },
        ProviderProfile {
            id: "novita",
            display_name: "Novita AI",
            base_url: "https://api.novita.ai/v3/openai",
            api_mode: ApiMode::ChatCompletions,
            auth_type: AuthType::ApiKey,
            env_var: "NOVITA_API_KEY",
            default_model: "deepseek/deepseek-v3-0324",
            default_aux_model: "meta-llama/llama-3.1-8b-instruct",
            omit_temperature: false,
            supports_prompt_caching: false,
        },
        ProviderProfile {
            id: "perplexity",
            display_name: "Perplexity Sonar",
            base_url: "https://api.perplexity.ai",
            api_mode: ApiMode::ChatCompletions,
            auth_type: AuthType::ApiKey,
            env_var: "PERPLEXITY_API_KEY",
            default_model: "sonar-pro",
            default_aux_model: "sonar",
            omit_temperature: false,
            supports_prompt_caching: false,
        },
        ProviderProfile {
            id: "cohere",
            display_name: "Cohere Command",
            base_url: "https://api.cohere.ai/compatibility/v1",
            api_mode: ApiMode::ChatCompletions,
            auth_type: AuthType::ApiKey,
            env_var: "COHERE_API_KEY",
            default_model: "command-a-03-2025",
            default_aux_model: "command-r7b-12-2024",
            omit_temperature: false,
            supports_prompt_caching: false,
        },
        ProviderProfile {
            id: "huggingface",
            display_name: "Hugging Face Router",
            base_url: "https://router.huggingface.co/v1",
            api_mode: ApiMode::ChatCompletions,
            auth_type: AuthType::ApiKey,
            env_var: "HF_TOKEN",
            default_model: "Qwen/Qwen2.5-Coder-32B-Instruct",
            default_aux_model: "Qwen/Qwen2.5-7B-Instruct",
            omit_temperature: false,
            supports_prompt_caching: false,
        },
        ProviderProfile {
            id: "github-copilot",
            display_name: "GitHub Models / Copilot",
            base_url: "https://models.inference.ai.azure.com",
            api_mode: ApiMode::ChatCompletions,
            auth_type: AuthType::OAuth,
            env_var: "GITHUB_TOKEN",
            default_model: "gpt-4.1",
            default_aux_model: "gpt-4.1-mini",
            omit_temperature: false,
            supports_prompt_caching: false,
        },
        ProviderProfile {
            id: "custom",
            display_name: "Custom OpenAI-Compatible",
            base_url: "http://127.0.0.1:8000/v1",
            api_mode: ApiMode::ChatCompletions,
            auth_type: AuthType::ApiKey,
            env_var: "OPENAI_API_KEY",
            default_model: "default",
            default_aux_model: "default",
            omit_temperature: false,
            supports_prompt_caching: false,
        },
    ]
}

pub fn get_provider_profile(id: &str) -> Option<ProviderProfile> {
    let needle = id.trim().to_ascii_lowercase();
    builtin_providers().into_iter().find(|p| p.id == needle)
}

pub fn resolve_provider_route(
    cfg: &SaifConfig,
    secrets: &SecretScope,
    provider_override: Option<&str>,
    model_override: Option<&str>,
    base_url_override: Option<&str>,
) -> Result<ResolvedRoute, ProviderError> {
    resolve_provider_route_for_home(
        None,
        cfg,
        secrets,
        provider_override,
        model_override,
        base_url_override,
    )
}

pub fn resolve_provider_route_for_home(
    saif_home: Option<&Path>,
    cfg: &SaifConfig,
    secrets: &SecretScope,
    provider_override: Option<&str>,
    model_override: Option<&str>,
    base_url_override: Option<&str>,
) -> Result<ResolvedRoute, ProviderError> {
    let chosen_provider = provider_override
        .filter(|s| !s.trim().is_empty())
        .or_else(|| (!cfg.provider.trim().is_empty()).then_some(cfg.provider.as_str()))
        .unwrap_or_else(|| auto_detect_provider(secrets));

    let profile = get_provider_profile(chosen_provider).unwrap_or_else(|| {
        get_provider_profile("custom").unwrap_or_else(|| builtin_providers().remove(0))
    });

    let base_url = base_url_override
        .filter(|s| !s.trim().is_empty())
        .or_else(|| cfg.base_url.as_deref().filter(|s| !s.trim().is_empty()))
        .unwrap_or(profile.base_url)
        .trim_end_matches('/')
        .to_string();

    let model = model_override
        .filter(|s| !s.trim().is_empty())
        .or_else(|| (!cfg.model.trim().is_empty()).then_some(cfg.model.as_str()))
        .unwrap_or(profile.default_model)
        .to_string();

    let api_mode = match cfg.api_mode.as_deref() {
        Some("anthropic") | Some("anthropic_messages") => ApiMode::AnthropicMessages,
        Some("codex") | Some("codex_responses") => ApiMode::CodexResponses,
        Some("bedrock") => ApiMode::Bedrock,
        Some("chat_completions") | Some("openai") => ApiMode::ChatCompletions,
        _ => profile.api_mode,
    };

    let now_epoch = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let mut pooled_credential_id = None;
    let mut api_key = None;
    if let Some(home) = saif_home {
        let mut pool = CredentialPool::open(home);
        if let Some(cred) = pool.select(profile.id, now_epoch) {
            pooled_credential_id = Some(cred.id);
            api_key = Some(cred.secret);
            let _ = pool.save();
        }
    }
    if api_key.is_none() {
        api_key = secrets.get_secret(profile.env_var);
    }
    if api_key.is_none() {
        if let Some(home) = saif_home {
            api_key = read_auth_json_secret(home, profile.id);
        }
    }

    if profile.auth_type == AuthType::ApiKey
        && api_key.is_none()
        && !base_url.starts_with("http://127.0.0.1")
        && !base_url.starts_with("http://localhost")
    {
        return Err(ProviderError::MissingCredentials {
            provider: profile.id.to_string(),
            env_var: profile.env_var.to_string(),
        });
    }

    let multiplex_active = secrets.is_multiplex();
    let aws_region = secrets
        .get_secret("AWS_REGION")
        .or_else(|| secrets.get_secret("AWS_DEFAULT_REGION"))
        .or_else(|| Some("us-east-1".to_string()));
    let aws_access_key_id = secrets.get_secret("AWS_ACCESS_KEY_ID");
    let aws_secret_access_key = secrets.get_secret("AWS_SECRET_ACCESS_KEY");
    let aws_session_token = secrets.get_secret("AWS_SESSION_TOKEN");
    let aws_profile = secrets.get_secret("AWS_PROFILE");

    let resolved_base =
        if api_mode == ApiMode::Bedrock && base_url_override.is_none() && cfg.base_url.is_none() {
            format!(
                "https://bedrock-runtime.{}.amazonaws.com",
                aws_region.as_deref().unwrap_or("us-east-1")
            )
        } else {
            base_url
        };

    Ok(ResolvedRoute {
        provider_id: profile.id.to_string(),
        base_url: resolved_base,
        api_mode,
        model,
        api_key,
        omit_temperature: profile.omit_temperature,
        supports_prompt_caching: profile.supports_prompt_caching,
        multiplex_active,
        pooled_credential_id,
        saif_home: saif_home.map(Path::to_path_buf),
        aws_region,
        aws_access_key_id,
        aws_secret_access_key,
        aws_session_token,
        aws_profile,
    })
}

fn auto_detect_provider(secrets: &SecretScope) -> &'static str {
    for candidate in [
        ("openrouter", "OPENROUTER_API_KEY"),
        ("anthropic", "ANTHROPIC_API_KEY"),
        ("openai", "OPENAI_API_KEY"),
        ("gemini", "GEMINI_API_KEY"),
        ("deepseek", "DEEPSEEK_API_KEY"),
        ("groq", "GROQ_API_KEY"),
        ("xai", "XAI_API_KEY"),
        ("zai", "ZAI_API_KEY"),
        ("cerebras", "CEREBRAS_API_KEY"),
        ("sambanova", "SAMBANOVA_API_KEY"),
        ("chutes", "CHUTES_API_KEY"),
        ("hyperbolic", "HYPERBOLIC_API_KEY"),
        ("featherless", "FEATHERLESS_API_KEY"),
        ("novita", "NOVITA_API_KEY"),
        ("perplexity", "PERPLEXITY_API_KEY"),
        ("cohere", "COHERE_API_KEY"),
        ("huggingface", "HF_TOKEN"),
    ] {
        if secrets.get_secret(candidate.1).is_some() {
            return candidate.0;
        }
    }
    "openrouter"
}
