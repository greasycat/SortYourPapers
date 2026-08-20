//! Interactive setup for `syp watch init`.
//!
//! Prompting lives in the CLI frontend: the workflow crate owns where the file
//! goes and what it contains, this module only collects the answers. With no
//! terminal on stdin — a pipe, a cron job, CI — the prompts are skipped and the
//! defaults are written, so scripted setup never blocks.

use std::{io, path::Path};

use syp_core::{
    app,
    config::{WatchSettings, default_api_key_env, watch_config_path},
    error::Result,
    llm::LlmProvider,
    terminal::current_backend,
};

use crate::WatchInitArgs;

/// Runs `syp watch init`, prompting when there is someone to prompt.
///
/// # Errors
/// Returns an error when the folder is missing, a config already exists there
/// and `--force` is not set, or the file cannot be written.
pub fn run_watch_init(args: WatchInitArgs) -> Result<()> {
    let folder = app::watch_init_folder(args.input)?;
    let mut settings = WatchSettings::defaults_for(&folder);
    let output_from_flag = args.output.is_some();
    if let Some(output) = &args.output {
        settings.output = app::absolute_path(output)?;
    }

    if !current_backend().is_interactive() {
        let path = app::init_watch_config(&folder, &settings, args.force)?;
        println!("Wrote watch config to {}", path.display());
        return Ok(());
    }

    match prompt_settings(&folder, settings, output_from_flag, args.force) {
        Ok(Some((settings, force))) => {
            let path = app::init_watch_config(&folder, &settings, force)?;
            cliclack::outro(format!("Wrote {}", path.display()))?;
            Ok(())
        }
        Ok(None) => {
            cliclack::outro_cancel("Kept the existing config.")?;
            Ok(())
        }
        Err(err) if err.kind() == io::ErrorKind::Interrupted => {
            cliclack::outro_cancel("Cancelled. Nothing was written.")?;
            Ok(())
        }
        Err(err) => Err(err.into()),
    }
}

/// Collects the settings to write, or `None` when the user declines to
/// overwrite an existing config.
///
/// Overwriting is settled first, so nobody answers the whole wizard only to be
/// told the file was already there.
fn prompt_settings(
    folder: &Path,
    mut settings: WatchSettings,
    output_from_flag: bool,
    force: bool,
) -> io::Result<Option<(WatchSettings, bool)>> {
    cliclack::intro(" syp watch init ")?;
    cliclack::log::step(format!("Watching {}", folder.display()))?;

    let existing = watch_config_path(folder);
    let force = force || {
        if !existing.is_file() {
            false
        } else if cliclack::confirm(format!("Overwrite {}?", existing.display()))
            .initial_value(false)
            .interact()?
        {
            true
        } else {
            return Ok(None);
        }
    };

    if !output_from_flag {
        let output: String = cliclack::input("Where should sorted papers go?")
            .default_input(&settings.output.display().to_string())
            .interact()?;
        settings.output = output.trim().into();
    }

    settings.recursive = cliclack::confirm("Scan subfolders of the watched folder?")
        .initial_value(settings.recursive)
        .interact()?;

    settings.llm_provider = cliclack::select("Which model backend?")
        .initial_value(settings.llm_provider)
        .item(LlmProvider::Gemini, "Gemini", "Google")
        .item(
            LlmProvider::Openai,
            "OpenAI",
            "or any OpenAI-compatible API",
        )
        .item(LlmProvider::Ollama, "Ollama", "local, no API key")
        .interact()?;

    settings.llm_model = cliclack::input("Which model?")
        .default_input(&settings.llm_model)
        .required(true)
        .interact()?;

    settings.api_key_env = match default_api_key_env(settings.llm_provider) {
        Some(default_variable) => {
            let variable: String = cliclack::input("Environment variable holding the API key")
                .default_input(default_variable)
                .placeholder("leave empty to configure the key elsewhere")
                .required(false)
                .interact()?;
            let variable = variable.trim().to_string();
            (!variable.is_empty()).then_some(variable)
        }
        None => None,
    };

    Ok(Some((settings, force)))
}
