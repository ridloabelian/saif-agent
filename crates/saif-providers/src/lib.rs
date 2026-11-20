pub mod auxiliary;
pub mod bedrock;
pub mod catalog;
pub mod codex;
pub mod credential_pool;
pub mod transport;
pub mod types;

pub use auxiliary::{AuxiliaryClient, AuxiliaryTask};
pub use bedrock::{
    hmac_sha256, is_bedrock_xai_grok_no_sampling, resolve_scoped_aws_credentials,
    sign_aws_sigv4_headers, AwsCredentials, BedrockConverseTransport, SigV4Headers,
};
pub use catalog::{
    builtin_providers, get_provider_profile, resolve_provider_route,
    resolve_provider_route_for_home, ApiMode, AuthType, ProviderProfile, ResolvedRoute,
};
pub use codex::{
    alias_reserved_tools, bounded_prompt_cache_key, cache_scope_from_session_id,
    reserved_names_for_endpoint, CodexResponsesTransport,
};
pub use credential_pool::{
    is_terminal_auth_reason, read_auth_json_secret, CredStatus, CredentialPool, PoolStrategy,
    PooledCredential, EXHAUSTED_TTL_401_SECONDS, EXHAUSTED_TTL_429_SECONDS,
    EXHAUSTED_TTL_DEFAULT_SECONDS, EXHAUSTED_TTL_SOLE_CREDENTIAL_SECONDS,
};
pub use transport::{
    dispatch_completion, dispatch_completion_with_failover, AnthropicMessagesTransport,
    LlmTransport, OpenAiChatTransport, TransportRouter,
};
pub use types::{
    ChatMessage, LlmRequest, LlmResponse, MessageRole, ProviderError, StreamChunk, ToolCall,
    ToolDefinition, UsageMetrics,
};
