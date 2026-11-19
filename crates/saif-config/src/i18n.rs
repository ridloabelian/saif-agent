use std::collections::HashMap;
use std::fs;
use std::path::Path;

/// Layered flat dotted-key localization (`user overlay -> bundled -> en -> bare key`).
#[derive(Debug, Clone)]
pub struct I18nCatalog {
    lang: String,
    active: HashMap<String, String>,
    english: HashMap<String, String>,
}

impl I18nCatalog {
    pub fn load(home_dir: &Path, lang: &str) -> Self {
        let mut english = bundled_strings("en");
        let mut active = bundled_strings(lang);

        let en_overlay = home_dir.join("locales").join("en.yaml");
        if let Ok(raw) = fs::read_to_string(en_overlay) {
            flatten_yaml_into(&raw, &mut english);
        }

        let lang_overlay = home_dir.join("locales").join(format!("{lang}.yaml"));
        if let Ok(raw) = fs::read_to_string(lang_overlay) {
            flatten_yaml_into(&raw, &mut active);
        }

        Self {
            lang: lang.to_string(),
            active,
            english,
        }
    }

    pub fn language(&self) -> &str {
        &self.lang
    }

    pub fn t<'a>(&'a self, key: &'a str) -> &'a str {
        self.active
            .get(key)
            .or_else(|| self.english.get(key))
            .map(String::as_str)
            .unwrap_or(key)
    }
}

fn bundled_strings(lang: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    match lang {
        "id" => {
            map.insert(
                "cli.ready".to_string(),
                "Siap. Ketik prompt atau /help untuk daftar perintah.".to_string(),
            );
            map.insert("cli.goodbye".to_string(), "Sampai jumpa.".to_string());
            map.insert(
                "cli.session_created".to_string(),
                "Sesi baru dimulai.".to_string(),
            );
        }
        _ => {
            map.insert(
                "cli.ready".to_string(),
                "Ready. Type a prompt or /help for commands.".to_string(),
            );
            map.insert("cli.goodbye".to_string(), "Goodbye.".to_string());
            map.insert(
                "cli.session_created".to_string(),
                "New session started.".to_string(),
            );
        }
    }
    map
}

fn flatten_yaml_into(raw: &str, out: &mut HashMap<String, String>) {
    let Ok(val) = serde_yaml::from_str::<serde_yaml::Value>(raw) else {
        return;
    };
    flatten_value("", &val, out);
}

fn flatten_value(prefix: &str, val: &serde_yaml::Value, out: &mut HashMap<String, String>) {
    match val {
        serde_yaml::Value::String(s) => {
            if !prefix.is_empty() {
                out.insert(prefix.to_string(), s.clone());
            }
        }
        serde_yaml::Value::Mapping(map) => {
            for (k, v) in map {
                if let Some(ks) = k.as_str() {
                    let next = if prefix.is_empty() {
                        ks.to_string()
                    } else {
                        format!("{prefix}.{ks}")
                    };
                    flatten_value(&next, v, out);
                }
            }
        }
        _ => {}
    }
}
