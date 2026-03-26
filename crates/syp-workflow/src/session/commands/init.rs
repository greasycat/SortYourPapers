use std::path::PathBuf;

use crate::{config, error::Result};

/// Initializes the default XDG config file for the application.
///
/// # Errors
/// Returns an error when the config path cannot be resolved or written.
pub fn init_config(force: bool) -> Result<PathBuf> {
    config::init_xdg_config(force)
}
