use crate::registry::{ToolContext, ToolError};
use regex::Regex;
use reqwest::Client;
use serde_json::{json, Value};
use std::net::{IpAddr, ToSocketAddrs};
use std::sync::OnceLock;
use std::time::Duration;

fn html_tag_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?is)<script[^>]*>.*?</script>|<style[^>]*>.*?</style>|<[^>]+>")
            .expect("valid static html regex")
    })
}

fn is_forbidden_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_multicast()
                || v4.octets()[0] == 100 && (v4.octets()[1] & 0b1100_0000) == 64
        }
        IpAddr::V6(v6) => {
            v6.is_loopback() || v6.is_unspecified() || v6.is_multicast() || v6.is_unique_local()
        }
    }
}

pub fn validate_public_url(raw_url: &str, allow_private: bool) -> Result<reqwest::Url, ToolError> {
    let url = reqwest::Url::parse(raw_url)
        .map_err(|e| ToolError::InvalidArgs(format!("invalid URL '{raw_url}': {e}")))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(ToolError::InvalidArgs(format!(
            "unsupported URL scheme '{}': only http/https allowed",
            url.scheme()
        )));
    }
    if allow_private {
        return Ok(url);
    }
    let host = url
        .host_str()
        .ok_or_else(|| ToolError::InvalidArgs("URL missing host".to_string()))?;
    if host.eq_ignore_ascii_case("localhost") || host.ends_with(".local") {
        return Err(ToolError::Execution(format!(
            "SSRF guard blocked private/local host '{host}'"
        )));
    }
    if let Ok(ip) = host.parse::<IpAddr>() {
        if is_forbidden_ip(ip) {
            return Err(ToolError::Execution(format!(
                "SSRF guard blocked non-public IP '{ip}'"
            )));
        }
    } else if let Ok(addrs) = (host, url.port_or_known_default().unwrap_or(80)).to_socket_addrs() {
        for addr in addrs {
            if is_forbidden_ip(addr.ip()) {
                return Err(ToolError::Execution(format!(
                    "SSRF guard blocked host '{host}' resolving to non-public IP {}",
                    addr.ip()
                )));
            }
        }
    }
    Ok(url)
}

/// Validate a redirect target on every hop against SSRF rules and cross-origin credential leaks.
pub fn validate_redirect_target(
    next_url: &reqwest::Url,
    previous_url: &reqwest::Url,
    allow_private: bool,
) -> Result<(), String> {
    let same_origin = (
        next_url.scheme(),
        next_url.host_str(),
        next_url.port_or_known_default(),
    ) == (
        previous_url.scheme(),
        previous_url.host_str(),
        previous_url.port_or_known_default(),
    );
    if !same_origin && (!next_url.username().is_empty() || next_url.password().is_some()) {
        return Err("SSRF guard blocked credential-bearing cross-origin redirect".to_string());
    }
    validate_public_url(next_url.as_str(), allow_private).map_err(|e| e.to_string())?;
    Ok(())
}

/// Build a reqwest redirect policy that re-validates every redirect hop against SSRF guards.
pub fn build_ssrf_safe_redirect_policy(allow_private: bool) -> reqwest::redirect::Policy {
    reqwest::redirect::Policy::custom(move |attempt| {
        if attempt.previous().len() >= 10 {
            return attempt.error("too many redirects (max 10)");
        }
        if let Some(prev) = attempt.previous().last() {
            if let Err(reason) = validate_redirect_target(attempt.url(), prev, allow_private) {
                return attempt.error(reason);
            }
        }
        attempt.follow()
    })
}

pub async fn handle_web_extract(args: &Value, ctx: &ToolContext) -> Result<String, ToolError> {
    let url_raw = args
        .get("url")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ToolError::InvalidArgs("missing 'url'".to_string()))?;
    let url = validate_public_url(url_raw, ctx.config.security.allow_private_urls)?;

    let client = Client::builder()
        .timeout(Duration::from_secs(20))
        .user_agent("saif-agent/0.1")
        .redirect(build_ssrf_safe_redirect_policy(
            ctx.config.security.allow_private_urls,
        ))
        .build()
        .map_err(|e| ToolError::Execution(e.to_string()))?;

    let resp = client
        .get(url.clone())
        .send()
        .await
        .map_err(|e| ToolError::Execution(format!("HTTP GET failed: {e}")))?;
    let status = resp.status().as_u16();
    let body = resp
        .text()
        .await
        .map_err(|e| ToolError::Execution(format!("body read failed: {e}")))?;

    let cleaned = html_tag_re().replace_all(&body, " ");
    let collapsed = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    let truncated: String = collapsed.chars().take(16_000).collect();

    Ok(json!({
        "success": true,
        "url": url.to_string(),
        "status": status,
        "content": truncated
    })
    .to_string())
}

pub async fn handle_web_search(args: &Value, ctx: &ToolContext) -> Result<String, ToolError> {
    let query = args
        .get("query")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ToolError::InvalidArgs("missing 'query'".to_string()))?;

    let client = Client::builder()
        .timeout(Duration::from_secs(20))
        .user_agent("saif-agent/0.1")
        .build()
        .map_err(|e| ToolError::Execution(e.to_string()))?;

    if let Some(tavily_key) = ctx.profile.secrets.get_secret("TAVILY_API_KEY") {
        let resp = client
            .post("https://api.tavily.com/search")
            .json(&json!({
                "api_key": tavily_key,
                "query": query,
                "max_results": 5
            }))
            .send()
            .await
            .map_err(|e| ToolError::Execution(format!("Tavily search failed: {e}")))?;
        let val: Value = resp
            .json()
            .await
            .map_err(|e| ToolError::Execution(format!("Tavily parse failed: {e}")))?;
        return Ok(json!({
            "success": true,
            "provider": "tavily",
            "results": val.get("results").cloned().unwrap_or_else(|| json!([]))
        })
        .to_string());
    }

    if let Some(brave_key) = ctx.profile.secrets.get_secret("BRAVE_API_KEY") {
        let resp = client
            .get("https://api.search.brave.com/res/v1/web/search")
            .query(&[("q", query), ("count", "5")])
            .header("X-Subscription-Token", brave_key)
            .send()
            .await
            .map_err(|e| ToolError::Execution(format!("Brave search failed: {e}")))?;
        let val: Value = resp
            .json()
            .await
            .map_err(|e| ToolError::Execution(format!("Brave parse failed: {e}")))?;
        return Ok(json!({
            "success": true,
            "provider": "brave",
            "results": val.pointer("/web/results").cloned().unwrap_or_else(|| json!([]))
        })
        .to_string());
    }

    // Fallback: DuckDuckGo Instant Answer API (free, no key needed)
    let resp = client
        .get("https://api.duckduckgo.com/")
        .query(&[("q", query), ("format", "json"), ("no_html", "1")])
        .send()
        .await
        .map_err(|e| ToolError::Execution(format!("DuckDuckGo search failed: {e}")))?;
    let val: Value = resp.json().await.unwrap_or_else(|_| json!({}));

    Ok(json!({
        "success": true,
        "provider": "duckduckgo",
        "abstract": val.get("AbstractText").cloned().unwrap_or(Value::Null),
        "related_topics": val.get("RelatedTopics").cloned().unwrap_or_else(|| json!([]))
    })
    .to_string())
}
