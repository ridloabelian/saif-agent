pub mod auxiliary;
pub mod catalog;
pub mod transport;
pub mod types;

pub use auxiliary::{AuxiliaryClient, AuxiliaryTask};
pub use catalog::{
    builtin_providers, get_provider_profile, resolve_provider_route, ApiMode, AuthType,
    ProviderProfile, ResolvedRoute,
};
pub use transport::{
    dispatch_completion, AnthropicMessagesTransport, LlmTransport, OpenAiChatTransport,
};
pub use types::{
    ChatMessage, LlmRequest, LlmResponse, MessageRole, ProviderError, StreamChunk, ToolCall,
    ToolDefinition, UsageMetrics,
};
