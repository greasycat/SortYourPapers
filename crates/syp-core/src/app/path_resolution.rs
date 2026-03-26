use std::{
    env,
    path::{Path, PathBuf},
};

use crate::{config::AppConfig, error::Result};

pub(crate) fn absolutize_config(mut config: AppConfig) -> Result<AppConfig> {
    let cwd = env::current_dir()?;
    config.input = absolutize_path(&cwd, &config.input);
    config.output = absolutize_path(&cwd, &config.output);
    Ok(config)
}

fn absolutize_path(cwd: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    }
}
