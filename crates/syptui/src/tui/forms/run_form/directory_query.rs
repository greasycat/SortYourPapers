use std::{
    fs,
    path::{Path, PathBuf},
};

pub(crate) fn list_relative_directories(cwd: &Path, value: &str) -> Vec<String> {
    let query = DirectoryQuery::from_input(cwd, value);
    let Ok(entries) = fs::read_dir(&query.search_dir) else {
        return Vec::new();
    };

    let mut directories = entries
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let metadata = entry.metadata().ok()?;
            metadata.is_dir().then_some(entry.file_name())
        })
        .filter_map(|name| {
            let name = name.to_str()?.to_string();
            if !query.prefix.is_empty() && !name.starts_with(&query.prefix) {
                return None;
            }

            let suggestion = if query.display_dir.as_os_str().is_empty() {
                PathBuf::from(&name)
            } else {
                query.display_dir.join(&name)
            };
            Some(suggestion.display().to_string())
        })
        .collect::<Vec<_>>();

    directories.sort_by_cached_key(|path| path.to_ascii_lowercase());
    directories
}

struct DirectoryQuery {
    search_dir: PathBuf,
    display_dir: PathBuf,
    prefix: String,
}

impl DirectoryQuery {
    fn from_input(cwd: &Path, value: &str) -> Self {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return Self {
                search_dir: cwd.to_path_buf(),
                display_dir: PathBuf::new(),
                prefix: String::new(),
            };
        }

        let typed = PathBuf::from(trimmed);
        let resolved = if typed.is_absolute() {
            typed.clone()
        } else {
            cwd.join(&typed)
        };
        let ends_with_separator = trimmed.chars().last().is_some_and(std::path::is_separator);
        if ends_with_separator || resolved.is_dir() {
            return Self {
                search_dir: resolved,
                display_dir: typed,
                prefix: String::new(),
            };
        }

        Self {
            search_dir: resolved
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| cwd.to_path_buf()),
            display_dir: typed.parent().map(Path::to_path_buf).unwrap_or_default(),
            prefix: typed
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default()
                .to_string(),
        }
    }
}
