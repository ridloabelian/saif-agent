use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct SkinColors {
    pub banner_border: String,
    pub banner_title: String,
    pub banner_accent: String,
    pub banner_dim: String,
    pub banner_text: String,
    pub response_border: String,
}

impl Default for SkinColors {
    fn default() -> Self {
        Self {
            banner_border: "cyan".to_string(),
            banner_title: "white".to_string(),
            banner_accent: "yellow".to_string(),
            banner_dim: "dark_gray".to_string(),
            banner_text: "gray".to_string(),
            response_border: "blue".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct SkinSpinner {
    pub waiting_face: String,
    pub thinking_face: String,
    pub thinking_verbs: Vec<String>,
}

impl Default for SkinSpinner {
    fn default() -> Self {
        Self {
            waiting_face: "(•‿•)".to_string(),
            thinking_face: "(⊙_⊙)".to_string(),
            thinking_verbs: vec![
                "Thinking".to_string(),
                "Reasoning".to_string(),
                "Inspecting".to_string(),
                "Working".to_string(),
            ],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct SkinBranding {
    pub agent_name: String,
    pub welcome: String,
    pub response_label: String,
    pub prompt_symbol: String,
}

impl Default for SkinBranding {
    fn default() -> Self {
        Self {
            agent_name: "Saif Agent".to_string(),
            welcome: "Ready. Type a prompt or /help for commands.".to_string(),
            response_label: "Assistant".to_string(),
            prompt_symbol: "❯ ".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct SkinConfig {
    pub name: String,
    pub colors: SkinColors,
    pub spinner: SkinSpinner,
    pub tool_prefix: String,
    pub branding: SkinBranding,
}

impl Default for SkinConfig {
    fn default() -> Self {
        Self {
            name: "default".to_string(),
            colors: SkinColors::default(),
            spinner: SkinSpinner::default(),
            tool_prefix: "⚙".to_string(),
            branding: SkinBranding::default(),
        }
    }
}

pub fn builtin_skin(name: &str) -> SkinConfig {
    match name.trim().to_ascii_lowercase().as_str() {
        "ares" => SkinConfig {
            name: "ares".to_string(),
            colors: SkinColors {
                banner_border: "red".to_string(),
                banner_accent: "light_red".to_string(),
                response_border: "red".to_string(),
                ..SkinColors::default()
            },
            ..SkinConfig::default()
        },
        "mono" => SkinConfig {
            name: "mono".to_string(),
            colors: SkinColors {
                banner_border: "white".to_string(),
                banner_accent: "white".to_string(),
                response_border: "gray".to_string(),
                ..SkinColors::default()
            },
            ..SkinConfig::default()
        },
        "slate" => SkinConfig {
            name: "slate".to_string(),
            colors: SkinColors {
                banner_border: "dark_gray".to_string(),
                banner_accent: "cyan".to_string(),
                response_border: "dark_gray".to_string(),
                ..SkinColors::default()
            },
            ..SkinConfig::default()
        },
        _ => SkinConfig::default(),
    }
}

pub fn load_skin(home_dir: &Path, name: &str) -> SkinConfig {
    let custom_path = home_dir.join("skins").join(format!("{name}.yaml"));
    if let Ok(raw) = fs::read_to_string(custom_path) {
        if let Ok(parsed) = serde_yaml::from_str::<SkinConfig>(&raw) {
            return parsed;
        }
    }
    builtin_skin(name)
}
