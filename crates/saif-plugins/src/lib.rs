pub mod catalog;
pub mod hooks;
pub mod host;
pub mod manifest;

pub use catalog::{canonical_repo_slug, validate_catalog_entry, CatalogEntry, RemovedCatalog};
pub use hooks::{HookEvent, HookPayload, HookRegistry, HookSubscriber};
pub use host::{PluginHostClient, RemotePluginRpc};
pub use manifest::{PluginError, PluginKind, PluginManager, PluginManifest};
