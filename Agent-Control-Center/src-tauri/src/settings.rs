//! Safe, narrow access to Codex's model defaults. This module never returns a
//! complete config file because it may contain settings unrelated to this UI.

use serde::Serialize;
use serde_json::Value;
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
use toml_edit::{value, DocumentMut, Item, Value as TomlValue};

const ROLE_KEYS: [&str; 5] = ["planner", "coder", "mobile-dev", "tester", "reviewer"];

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelOption {
    pub id: String,
    pub label: String,
    pub efforts: Vec<String>,
    pub default_effort: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoleModelSetting {
    pub key: String,
    pub label: String,
    pub config_path: String,
    pub model: String,
    pub effort: String,
    pub revision: String,
    pub editable: bool,
    pub warning: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelSettings {
    pub source_dir: String,
    pub catalog_source: String,
    pub catalog_fetched_at: String,
    pub warning: String,
    pub models: Vec<ModelOption>,
    pub roles: Vec<RoleModelSetting>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveModelResult {
    pub settings: ModelSettings,
    pub backup_path: String,
    pub changed: bool,
}

/// The service is held behind a Tauri mutex, serializing conflict checks and writes.
pub struct SettingsService {
    codex_home: PathBuf,
}

impl SettingsService {
    pub fn new() -> Result<Self, String> {
        let home = std::env::var_os("CODEX_HOME")
            .map(PathBuf::from)
            .or_else(|| dirs::home_dir().map(|p| p.join(".codex")))
            .ok_or("Cannot locate Codex home")?;
        Self::with_home(home)
    }

    fn with_home(home: PathBuf) -> Result<Self, String> {
        if !home.is_dir() {
            return Err(format!("Codex home is unavailable: {}", home.display()));
        }
        Ok(Self {
            codex_home: fs::canonicalize(home).map_err(|e| e.to_string())?,
        })
    }

    pub fn get(&self) -> ModelSettings {
        let (models, fetched_at, catalog_warning) = self.read_catalog();
        let mut warning = catalog_warning;
        if !warning.is_empty() {
            warning.push(' ');
        }
        warning.push_str("The cached model list is informational and does not confirm account entitlement. Changes apply to new sessions; project and explicit overrides may take precedence.");
        ModelSettings {
            source_dir: self.codex_home.to_string_lossy().into_owned(),
            catalog_source: self
                .codex_home
                .join("models_cache.json")
                .to_string_lossy()
                .into_owned(),
            catalog_fetched_at: fetched_at,
            warning,
            models,
            roles: ROLE_KEYS
                .into_iter()
                .map(|role| self.role_setting(role))
                .collect(),
        }
    }

    pub fn save(
        &self,
        role_key: &str,
        model: &str,
        effort: &str,
        revision: &str,
    ) -> Result<SaveModelResult, String> {
        if !ROLE_KEYS.contains(&role_key) {
            return Err("Unknown agent role".into());
        }
        let settings = self.get();
        let current = settings.roles.iter().find(|r| r.key == role_key).unwrap();
        if !current.editable {
            return Err(if current.warning.is_empty() {
                "This role has no editable configuration".into()
            } else {
                current.warning.clone()
            });
        }
        if current.revision != revision {
            return Err(
                "Configuration changed outside this window. Reload settings before saving.".into(),
            );
        }
        if current.model == model && current.effort == effort {
            return Ok(SaveModelResult {
                settings,
                backup_path: String::new(),
                changed: false,
            });
        }
        let selected = settings
            .models
            .iter()
            .find(|m| m.id == model)
            .ok_or("That model is not in the visible cached Codex catalog")?;
        if !selected.efforts.iter().any(|e| e == effort) {
            return Err("That reasoning effort is not supported by the selected model".into());
        }

        let path = self.role_path(role_key)?;
        let original = fs::read_to_string(&path)
            .map_err(|e| format!("Could not read agent configuration: {e}"))?;
        // Recheck the bytes after discovery to avoid an accidental write after an external edit.
        if revision_for(original.as_bytes()) != revision {
            return Err(
                "Configuration changed outside this window. Reload settings before saving.".into(),
            );
        }
        let mut doc = original
            .parse::<DocumentMut>()
            .map_err(|e| format!("Configuration is malformed TOML: {e}"))?;
        replace_root_string(&mut doc, "model", model);
        replace_root_string(&mut doc, "model_reasoning_effort", effort);
        let replacement = doc.to_string();
        let backup = self.backup_and_replace(&path, original.as_bytes(), replacement.as_bytes())?;
        Ok(SaveModelResult {
            settings: self.get(),
            backup_path: backup.to_string_lossy().into_owned(),
            changed: true,
        })
    }

    fn read_catalog(&self) -> (Vec<ModelOption>, String, String) {
        let path = self.codex_home.join("models_cache.json");
        let raw = match fs::read_to_string(&path) {
            Ok(raw) => raw,
            Err(_) => {
                return (
                    vec![],
                    String::new(),
                    "No local model catalog cache is available; model selection is disabled."
                        .into(),
                )
            }
        };
        let parsed: Value = match serde_json::from_str(&raw) {
            Ok(value) => value,
            Err(_) => {
                return (
                    vec![],
                    String::new(),
                    "The local model catalog cache is malformed; model selection is disabled."
                        .into(),
                )
            }
        };
        let models = parsed
            .get("models")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|m| {
                        if m.get("visibility").and_then(Value::as_str) != Some("list") {
                            return None;
                        }
                        let id = m.get("slug").and_then(Value::as_str)?.to_owned();
                        let efforts: Vec<String> = m
                            .get("supported_reasoning_levels")
                            .and_then(Value::as_array)
                            .into_iter()
                            .flatten()
                            .filter_map(|level| {
                                level
                                    .get("effort")
                                    .and_then(Value::as_str)
                                    .map(str::to_owned)
                            })
                            .collect();
                        if efforts.is_empty() {
                            return None;
                        }
                        let default_effort = m
                            .get("default_reasoning_level")
                            .and_then(|level| {
                                level
                                    .as_str()
                                    .or_else(|| level.get("effort").and_then(Value::as_str))
                            })
                            .filter(|e| efforts.iter().any(|option| option == *e))
                            .unwrap_or(&efforts[0])
                            .to_owned();
                        Some(ModelOption {
                            label: m
                                .get("display_name")
                                .and_then(Value::as_str)
                                .unwrap_or(&id)
                                .to_owned(),
                            id,
                            efforts,
                            default_effort,
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        (
            models,
            parsed
                .get("fetched_at")
                .and_then(Value::as_str)
                .unwrap_or("")
                .into(),
            String::new(),
        )
    }

    fn role_setting(&self, key: &str) -> RoleModelSetting {
        let label = role_label(key).into();
        let path = match self.role_path(key) {
            Ok(path) => path,
            Err(warning) => {
                return RoleModelSetting {
                    key: key.into(),
                    label,
                    config_path: String::new(),
                    model: String::new(),
                    effort: String::new(),
                    revision: String::new(),
                    editable: false,
                    warning,
                }
            }
        };
        let raw = match fs::read_to_string(&path) {
            Ok(raw) => raw,
            Err(e) => {
                return RoleModelSetting {
                    key: key.into(),
                    label,
                    config_path: path.to_string_lossy().into_owned(),
                    model: String::new(),
                    effort: String::new(),
                    revision: String::new(),
                    editable: false,
                    warning: format!("Could not read configuration: {e}"),
                }
            }
        };
        let doc = match raw.parse::<DocumentMut>() {
            Ok(doc) => doc,
            Err(e) => {
                return RoleModelSetting {
                    key: key.into(),
                    label,
                    config_path: path.to_string_lossy().into_owned(),
                    model: String::new(),
                    effort: String::new(),
                    revision: String::new(),
                    editable: false,
                    warning: format!("Configuration is malformed TOML: {e}"),
                }
            }
        };
        let model = item_string(doc.get("model"));
        let effort = item_string(doc.get("model_reasoning_effort"));
        let mut warning = if key == "planner" {
            "Planner is the global default. Project and explicit overrides can win; changes apply to new sessions.".into()
        } else {
            String::new()
        };
        if model.is_empty() || effort.is_empty() {
            if !warning.is_empty() {
                warning.push(' ');
            }
            warning.push_str("This file does not define both model and model_reasoning_effort.");
        }
        RoleModelSetting {
            key: key.into(),
            label,
            config_path: path.to_string_lossy().into_owned(),
            model,
            effort,
            revision: revision_for(raw.as_bytes()),
            editable: true,
            warning,
        }
    }

    fn role_path(&self, key: &str) -> Result<PathBuf, String> {
        if key == "planner" {
            return self.checked_file(self.codex_home.join("config.toml"));
        }
        let agent_dir = self.codex_home.join("agents");
        let entries = fs::read_dir(&agent_dir)
            .map_err(|_| format!("Agent directory is unavailable: {}", agent_dir.display()))?;
        let mut candidates = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|x| x.to_str()) != Some("toml") {
                continue;
            }
            let checked = self.checked_file(path)?;
            let raw = fs::read_to_string(&checked)
                .map_err(|e| format!("Could not read agent configuration: {e}"))?;
            let doc = raw
                .parse::<DocumentMut>()
                .map_err(|e| format!("Agent configuration is malformed TOML: {e}"))?;
            if item_string(doc.get("name")) == key {
                candidates.push(checked);
            }
        }
        match candidates.len() {
            1 => Ok(candidates.remove(0)),
            0 => Err(format!("No agent configuration declares name = \"{key}\".")),
            _ => Err(format!(
                "More than one agent configuration declares name = \"{key}\"."
            )),
        }
    }

    fn checked_file(&self, candidate: PathBuf) -> Result<PathBuf, String> {
        let canonical = fs::canonicalize(&candidate)
            .map_err(|e| format!("Configuration is unavailable: {e}"))?;
        if !canonical.starts_with(&self.codex_home) || !canonical.is_file() {
            return Err("Configuration path escapes Codex home or is not a file".into());
        }
        Ok(canonical)
    }

    fn backup_and_replace(
        &self,
        path: &Path,
        original: &[u8],
        replacement: &[u8],
    ) -> Result<PathBuf, String> {
        let parent = path
            .parent()
            .ok_or("Configuration has no parent directory")?;
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        let backup = unique_file(
            parent,
            &format!(
                "{}.agent-control-center.{stamp}.bak",
                path.file_name()
                    .and_then(|v| v.to_str())
                    .unwrap_or("config")
            ),
        )?;
        write_new(&backup, original)?;
        let temp = unique_file(
            parent,
            &format!(
                ".{}.agent-control-center.{stamp}.tmp",
                path.file_name()
                    .and_then(|v| v.to_str())
                    .unwrap_or("config")
            ),
        )?;
        if let Err(error) = write_new(&temp, replacement)
            .and_then(|_| fs::rename(&temp, path).map_err(|e| e.to_string()))
        {
            let _ = fs::remove_file(&temp);
            return Err(format!(
                "Could not atomically replace configuration; original remains intact: {error}"
            ));
        }
        Ok(backup)
    }
}

fn role_label(key: &str) -> &'static str {
    match key {
        "planner" => "Planner",
        "coder" => "Coder",
        "mobile-dev" => "Mobile Dev",
        "tester" => "Tester",
        "reviewer" => "Reviewer",
        _ => "Unknown",
    }
}
fn item_string(item: Option<&Item>) -> String {
    item.and_then(Item::as_str).unwrap_or("").into()
}
fn replace_root_string(doc: &mut DocumentMut, key: &str, replacement: &str) {
    if let Some(existing) = doc[key].as_value_mut() {
        let decor = existing.decor().clone();
        let mut next = TomlValue::from(replacement);
        *next.decor_mut() = decor;
        *existing = next;
    } else {
        doc[key] = value(replacement);
    }
}
fn revision_for(bytes: &[u8]) -> String {
    let mut state = 0xcbf29ce484222325u64;
    for byte in bytes {
        state ^= *byte as u64;
        state = state.wrapping_mul(0x100000001b3);
    }
    format!("{state:016x}")
}
fn unique_file(parent: &Path, base: &str) -> Result<PathBuf, String> {
    for number in 0..1000 {
        let p = parent.join(if number == 0 {
            base.into()
        } else {
            format!("{base}.{number}")
        });
        if !p.exists() {
            return Ok(p);
        }
    }
    Err("Could not create a unique backup name".into())
}
fn write_new(path: &Path, contents: &[u8]) -> Result<(), String> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| e.to_string())?;
    file.write_all(contents)
        .and_then(|_| file.sync_all())
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (PathBuf, SettingsService) {
        let root = std::env::temp_dir().join(format!(
            "acc-settings-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(root.join("agents")).unwrap();
        fs::write(
            root.join("config.toml"),
            "# preserved\nmodel = \"old\" # primary choice\nmodel_reasoning_effort = \"low\" # reasoning choice\nextra = \"keep\"\n",
        )
        .unwrap();
        for role in ["coder", "mobile-dev", "tester", "reviewer"] {
            fs::write(root.join("agents").join(format!("{role}.toml")), format!("# {role}\nname = \"{role}\"\nmodel = \"old\" # primary choice\nmodel_reasoning_effort = \"low\" # reasoning choice\n" )).unwrap();
        }
        fs::write(root.join("models_cache.json"), r#"{"fetched_at":"2026-10-01T00:00:00Z","models":[{"slug":"new","display_name":"New","visibility":"list","supported_reasoning_levels":[{"effort":"high"},{"effort":"low"}],"default_reasoning_level":"low"},{"slug":"hidden","visibility":"hidden","supported_reasoning_levels":[{"effort":"low"}]}]}"#).unwrap();
        let service = SettingsService::with_home(root.clone()).unwrap();
        (root, service)
    }
    #[test]
    fn save_preserves_toml_and_creates_backup() {
        let (root, service) = fixture();
        let r = service
            .get()
            .roles
            .into_iter()
            .find(|r| r.key == "coder")
            .unwrap();
        let saved = service.save("coder", "new", "high", &r.revision).unwrap();
        assert!(saved.changed);
        assert!(Path::new(&saved.backup_path).is_file());
        assert_eq!(
            fs::read_to_string(&saved.backup_path).unwrap(),
            "# coder\nname = \"coder\"\nmodel = \"old\" # primary choice\nmodel_reasoning_effort = \"low\" # reasoning choice\n"
        );
        let now = fs::read_to_string(root.join("agents/coder.toml")).unwrap();
        assert!(now.contains("# coder"));
        assert!(now.contains("model = \"new\""));
        assert!(now.contains("model_reasoning_effort = \"high\""));
        assert!(now.contains("# primary choice"));
        assert!(now.contains("# reasoning choice"));
        assert_eq!(
            fs::read_to_string(root.join("config.toml")).unwrap(),
            "# preserved\nmodel = \"old\" # primary choice\nmodel_reasoning_effort = \"low\" # reasoning choice\nextra = \"keep\"\n"
        );
        assert_eq!(
            fs::read_to_string(root.join("agents/tester.toml")).unwrap(),
            "# tester\nname = \"tester\"\nmodel = \"old\" # primary choice\nmodel_reasoning_effort = \"low\" # reasoning choice\n"
        );
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn rejects_invalid_choice_without_writing() {
        let (root, service) = fixture();
        let r = service
            .get()
            .roles
            .into_iter()
            .find(|r| r.key == "coder")
            .unwrap();
        assert!(service.save("coder", "hidden", "low", &r.revision).is_err());
        assert!(fs::read_to_string(root.join("agents/coder.toml"))
            .unwrap()
            .contains("model = \"old\""));
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn rejects_revision_conflict_and_is_noop_when_equal() {
        let (root, service) = fixture();
        let r = service
            .get()
            .roles
            .into_iter()
            .find(|r| r.key == "coder")
            .unwrap();
        assert!(
            !service
                .save("coder", "old", "low", &r.revision)
                .unwrap()
                .changed
        );
        fs::write(
            root.join("agents/coder.toml"),
            "name = \"coder\"\nmodel = \"old\"\nmodel_reasoning_effort = \"low\"\n# changed\n",
        )
        .unwrap();
        assert!(service.save("coder", "new", "high", &r.revision).is_err());
        assert!(fs::read_to_string(root.join("agents/coder.toml"))
            .unwrap()
            .contains("# changed"));
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn role_allowlist_and_catalog_are_narrow() {
        let (root, service) = fixture();
        assert!(service.save("../../config", "new", "low", "x").is_err());
        let s = service.get();
        assert_eq!(s.models.len(), 1);
        assert_eq!(s.models[0].id, "new");
        assert_eq!(s.models[0].default_effort, "low");
        assert_eq!(s.roles.len(), 5);
        fs::remove_dir_all(root).unwrap();
    }
}
