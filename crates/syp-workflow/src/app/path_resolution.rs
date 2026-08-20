use std::{
    env,
    path::{Component, Path, PathBuf},
};

use crate::{config::AppConfig, error::Result};

pub(crate) fn absolutize_config(mut config: AppConfig) -> Result<AppConfig> {
    let cwd = env::current_dir()?;
    config.input = absolutize_path(&cwd, &config.input);
    config.output = absolutize_path(&cwd, &config.output);
    Ok(config)
}

/// Resolves `path` against the current directory when it is relative.
///
/// # Errors
/// Returns an error when the current directory cannot be read.
pub fn absolutize(path: &Path) -> Result<PathBuf> {
    Ok(absolutize_path(&env::current_dir()?, path))
}

fn absolutize_path(cwd: &Path, path: &Path) -> PathBuf {
    let joined = if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    };
    normalize(&joined)
}

/// Drops `.` and resolves `..` textually, so configured paths stay readable
/// when they are written to a config file or printed in a log.
///
/// Symlinks are left alone: this never touches the filesystem, so it works for
/// folders that do not exist yet.
fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir if out.components().next_back().is_some_and(is_normal) => {
                out.pop();
            }
            component => out.push(component),
        }
    }
    out
}

fn is_normal(component: Component<'_>) -> bool {
    matches!(component, Component::Normal(_))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absolutize_path_cleans_relative_markers() {
        let cwd = Path::new("/home/papers");

        assert_eq!(absolutize_path(cwd, Path::new(".")), PathBuf::from(cwd));
        assert_eq!(
            absolutize_path(cwd, Path::new("./sorted")),
            PathBuf::from("/home/papers/sorted")
        );
        assert_eq!(
            absolutize_path(cwd, Path::new("../library")),
            PathBuf::from("/home/library")
        );
        assert_eq!(
            absolutize_path(cwd, Path::new("/tmp/./inbox")),
            PathBuf::from("/tmp/inbox")
        );
    }
}
