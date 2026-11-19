pub mod builtin_tools;
pub mod env;
pub mod fs_tools;
pub mod fuzzy_match;
pub mod mcp;
pub mod patch_parser;
pub mod registry;
pub mod web_tools;

pub use builtin_tools::{register_all_builtin_tools, TodoItem, TodoStore};
pub use env::{
    DockerEnv, ExecResult, ExecutionEnvironment, LocalEnv, ProcessEntry, ProcessRegistry, SshEnv,
};
pub use fuzzy_match::{fuzzy_find_and_replace, FuzzyReplaceResult};
pub use mcp::{McpClient, McpServerConfig, McpTrustStore};
pub use patch_parser::{parse_v4a_patch, Hunk, HunkLine, OperationType, PatchOperation};
pub use registry::{
    register_default_toolsets, ToolContext, ToolError, ToolHandler, ToolRegistry, ToolsetCatalog,
    CORE_TOOLS,
};
