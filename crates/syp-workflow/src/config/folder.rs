//! Config file that lives inside a watched folder and applies only to it.
//!
//! A watch folder is self-describing: the file sits in the folder it
//! configures, so it never sets `input`. Everything else it sets overrides the
//! XDG config while that folder is being watched, and CLI flags and `SYP_*`
//! environment variables still win over it.

use std::{
    fs,
    path::{Path, PathBuf},
};

use toml::Table;

use crate::{
    defaults::{DEFAULT_LLM_PROVIDER, DEFAULT_RECURSIVE, default_llm_model},
    error::{AppError, Result},
    llm::LlmProvider,
};

/// The settings `syp watch init` writes into a watch folder.
///
/// Deliberately the short list a folder actually needs; everything else keeps
/// coming from the XDG config and the built-in defaults.
#[derive(Debug, Clone)]
pub struct WatchSettings {
    pub output: PathBuf,
    pub recursive: bool,
    pub llm_provider: LlmProvider,
    pub llm_model: String,
    /// Environment variable holding the API key, or `None` to leave the key to
    /// the environment and the XDG config.
    pub api_key_env: Option<String>,
}

impl WatchSettings {
    #[must_use]
    pub fn defaults_for(folder: &Path) -> Self {
        Self {
            output: default_watch_output(folder),
            recursive: DEFAULT_RECURSIVE,
            llm_provider: DEFAULT_LLM_PROVIDER,
            llm_model: default_llm_model(DEFAULT_LLM_PROVIDER).to_string(),
            api_key_env: None,
        }
    }
}

/// Conventional environment variable holding each provider's API key.
#[must_use]
pub fn default_api_key_env(provider: LlmProvider) -> Option<&'static str> {
    match provider {
        LlmProvider::Openai => Some("OPENAI_API_KEY"),
        LlmProvider::Gemini => Some("GEMINI_API_KEY"),
        LlmProvider::Ollama => None,
    }
}

/// Name of the per-folder config file, written by `syp watch init`.
pub const WATCH_CONFIG_FILE: &str = "syp.toml";

/// Default library folder created inside a watched folder.
///
/// The watcher ignores PDFs already inside the output folder, so nesting the
/// library in the watched folder is safe and keeps everything in one place.
const DEFAULT_LIBRARY_DIR: &str = "sorted";

#[must_use]
pub fn watch_config_path(folder: &Path) -> PathBuf {
    folder.join(WATCH_CONFIG_FILE)
}

/// Reads the watch config of `folder`, or an empty layer when it has none.
///
/// # Errors
/// Returns an error when the file cannot be read, is not valid TOML, or sets
/// `input`, which the folder itself already determines.
pub(super) fn load_watch_layer(folder: &Path) -> Result<Table> {
    let path = watch_config_path(folder);
    if !path.is_file() {
        return Ok(Table::new());
    }

    let table = fs::read_to_string(&path)?.parse::<Table>()?;
    if table.contains_key("input") {
        return Err(AppError::Config(format!(
            "{} must not set `input`: the watched folder is the one holding this file",
            path.display()
        )));
    }
    Ok(table)
}

/// Writes a starter watch config into `folder`.
///
/// Paths are written absolute so the file keeps meaning the same folders no
/// matter which directory `syp watch` is started from.
///
/// # Errors
/// Returns an error when the folder is missing, the file already exists and
/// `force` is not set, or the file cannot be written.
pub(super) fn write_watch_config(
    folder: &Path,
    settings: &WatchSettings,
    force: bool,
) -> Result<PathBuf> {
    if !folder.is_dir() {
        return Err(AppError::Validation(format!(
            "watch folder does not exist: {}",
            folder.display()
        )));
    }

    let path = watch_config_path(folder);
    if path.exists() && !force {
        return Err(AppError::Config(format!(
            "watch config already exists at {} (use `--force` to overwrite)",
            path.display()
        )));
    }

    fs::write(&path, watch_config_toml(folder, settings))?;
    Ok(path)
}

/// The default library folder for a watched folder.
#[must_use]
pub fn default_watch_output(folder: &Path) -> PathBuf {
    folder.join(DEFAULT_LIBRARY_DIR)
}

fn watch_config_toml(folder: &Path, settings: &WatchSettings) -> String {
    let api_key = match settings.api_key_env.as_deref() {
        Some(variable) => format!("api_key = {{ source = \"env\", value = \"{variable}\" }}\n"),
        None => "# api_key = { source = \"env\", value = \"GEMINI_API_KEY\" }\n".to_string(),
    };

    format!(
        concat!(
            "# SortYourPapers watch config for {folder}\n",
            "# Applies only while this folder is watched.\n",
            "# Priority: CLI > ENV > this file > XDG config > defaults\n",
            "# `input` belongs to the folder holding this file and cannot be set here.\n",
            "\n",
            "output = \"{output}\"\n",
            "recursive = {recursive}\n",
            "\n",
            "llm_provider = \"{provider}\"\n",
            "llm_model = \"{model}\"\n",
            "{api_key}",
        ),
        folder = folder.display(),
        output = settings.output.display(),
        recursive = settings.recursive,
        provider = provider_key(settings.llm_provider),
        model = settings.llm_model,
        api_key = api_key,
    )
}

/// Stable config-file spelling of a provider.
#[must_use]
pub fn provider_key(provider: LlmProvider) -> &'static str {
    match provider {
        LlmProvider::Openai => "openai",
        LlmProvider::Ollama => "ollama",
        LlmProvider::Gemini => "gemini",
    }
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::*;

    #[test]
    fn load_watch_layer_is_empty_without_a_config_file() {
        let temp = tempdir().expect("tempdir");

        let layer = load_watch_layer(temp.path()).expect("layer");

        assert!(layer.is_empty());
    }

    #[test]
    fn written_config_round_trips_into_a_layer() {
        let temp = tempdir().expect("tempdir");
        let settings = WatchSettings::defaults_for(temp.path());

        let path = write_watch_config(temp.path(), &settings, false).expect("write config");
        let layer = load_watch_layer(temp.path()).expect("layer");

        assert_eq!(path, temp.path().join(WATCH_CONFIG_FILE));
        assert_eq!(
            layer.get("output").and_then(toml::Value::as_str),
            Some(settings.output.display().to_string().as_str())
        );
        assert!(!layer.contains_key("input"));
    }

    #[test]
    fn write_watch_config_refuses_to_overwrite_without_force() {
        let temp = tempdir().expect("tempdir");
        let settings = WatchSettings::defaults_for(temp.path());
        write_watch_config(temp.path(), &settings, false).expect("first write");

        let err = write_watch_config(temp.path(), &settings, false)
            .expect_err("second write should be refused");
        assert!(err.to_string().contains("--force"));

        write_watch_config(temp.path(), &settings, true).expect("forced overwrite");
    }

    #[test]
    fn chosen_settings_reach_the_written_config() {
        let temp = tempdir().expect("tempdir");
        let settings = WatchSettings {
            output: PathBuf::from("/library"),
            recursive: true,
            llm_provider: LlmProvider::Openai,
            llm_model: "gpt-test".to_string(),
            api_key_env: Some("OPENAI_API_KEY".to_string()),
        };

        write_watch_config(temp.path(), &settings, false).expect("write config");
        let layer = load_watch_layer(temp.path()).expect("layer");

        assert_eq!(
            layer.get("output").and_then(toml::Value::as_str),
            Some("/library")
        );
        assert_eq!(
            layer.get("recursive").and_then(toml::Value::as_bool),
            Some(true)
        );
        assert_eq!(
            layer.get("llm_provider").and_then(toml::Value::as_str),
            Some("openai")
        );
        assert_eq!(
            layer.get("api_key").and_then(toml::Value::as_table),
            Some(&toml::Table::from_iter([
                ("source".to_string(), toml::Value::from("env")),
                ("value".to_string(), toml::Value::from("OPENAI_API_KEY")),
            ]))
        );
    }

    #[test]
    fn load_watch_layer_rejects_an_input_key() {
        let temp = tempdir().expect("tempdir");
        fs::write(watch_config_path(temp.path()), "input = \"/elsewhere\"\n")
            .expect("write config");

        let err = load_watch_layer(temp.path()).expect_err("input should be rejected");

        assert!(err.to_string().contains("must not set `input`"));
    }
}
