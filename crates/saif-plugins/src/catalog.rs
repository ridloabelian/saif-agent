use crate::manifest::PluginError;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CatalogEntry {
    pub name: String,
    pub repo: String,
    pub commit: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub docs_url: Option<String>,
}

impl CatalogEntry {
    /// Return `docs_url` only when it is a valid `https://` URL with a non-empty authority host.
    pub fn sanitized_docs_url(&self) -> Option<&str> {
        let raw = self.docs_url.as_deref()?.trim();
        if raw.starts_with("https:///") {
            return None;
        }
        let parsed = url::Url::parse(raw).ok()?;
        if parsed.scheme() == "https"
            && parsed.has_host()
            && parsed.username().is_empty()
            && parsed.password().is_none()
            && parsed.host_str().is_some_and(|h| !h.is_empty())
        {
            Some(raw)
        } else {
            None
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RemovedCatalog {
    #[serde(default)]
    pub repos: Vec<String>,
}

/// Normalize git/HTTPS/SSH URLs into canonical `host/owner/repo` lowercase form
/// (`plugins/AGENTS.md` `removed.yaml` kill-list rule).
pub fn canonical_repo_slug(url: &str) -> String {
    let mut s = url.trim().to_ascii_lowercase();
    if let Some(rest) = s.strip_prefix("git@") {
        s = rest.replacen(':', "/", 1);
    } else if let Some(rest) = s.strip_prefix("ssh://") {
        s = rest.trim_start_matches("git@").to_string();
    } else if let Some(rest) = s.strip_prefix("https://") {
        s = rest.to_string();
    } else if let Some(rest) = s.strip_prefix("http://") {
        s = rest.to_string();
    }
    if let Some(rest) = s.strip_prefix("www.") {
        s = rest.to_string();
    }
    s.trim_end_matches(".git").trim_end_matches('/').to_string()
}

/// Enforce Hermes's mandatory 40-hex SHA commit pin and `removed.yaml` kill list.
pub fn validate_catalog_entry(
    entry: &CatalogEntry,
    removed: &RemovedCatalog,
    allow_removed: bool,
) -> Result<(), PluginError> {
    let sha = entry.commit.trim();
    if sha.len() != 40 || !sha.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(PluginError::Invalid(format!(
            "catalog entry '{}' must pin a full 40-hex commit SHA, got '{}'",
            entry.name, entry.commit
        )));
    }
    let slug = canonical_repo_slug(&entry.repo);
    let blocked: HashSet<String> = removed
        .repos
        .iter()
        .map(|r| canonical_repo_slug(r))
        .collect();
    if blocked.contains(&slug) && !allow_removed {
        return Err(PluginError::RemovedFromCatalog(slug));
    }
    Ok(())
}
