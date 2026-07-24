use std::collections::HashSet;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

pub(super) fn resolve_git_directory(repository_root: &Path) -> Option<PathBuf> {
    let dot_git = repository_root.join(".git");
    if dot_git.is_dir() {
        return Some(dot_git);
    }
    let contents = fs::read_to_string(&dot_git).ok()?;
    let value = contents.trim().strip_prefix("gitdir:")?.trim();
    let configured = PathBuf::from(value);
    Some(if configured.is_absolute() {
        configured
    } else {
        repository_root.join(configured)
    })
}

pub(super) fn gitconfig_excludes_path(repository_root: Option<&Path>) -> Option<PathBuf> {
    if let Some(path) = env::var_os("GIT_CONFIG_GLOBAL").map(PathBuf::from)
        && !path.as_os_str().is_empty()
    {
        return read_excludes_setting_for(&path, repository_root).or_else(default_global_ignore);
    }
    let home = home_directory();
    if let Some(path) = home.as_ref().map(|home| home.join(".gitconfig"))
        && let Some(excludes) = read_excludes_setting_for(&path, repository_root)
    {
        return Some(excludes);
    }
    let xdg = env::var_os("XDG_CONFIG_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| home.as_ref().map(|home| home.join(".config")));
    if let Some(path) = xdg.as_ref().map(|xdg| xdg.join("git").join("config"))
        && let Some(excludes) = read_excludes_setting_for(&path, repository_root)
    {
        return Some(excludes);
    }
    if let Some(path) = env::var_os("GIT_CONFIG_SYSTEM")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| (!cfg!(windows)).then(|| PathBuf::from("/etc/gitconfig")))
        && let Some(excludes) = read_excludes_setting_for(&path, repository_root)
    {
        return Some(excludes);
    }
    default_global_ignore()
}

#[cfg(test)]
pub(super) fn read_excludes_setting(path: &Path) -> Option<PathBuf> {
    read_excludes_setting_for(path, None)
}

pub(super) fn read_excludes_setting_for(
    path: &Path,
    repository_root: Option<&Path>,
) -> Option<PathBuf> {
    let mut visited = HashSet::new();
    read_config(path, repository_root, &mut visited, 0)
}

fn read_config(
    path: &Path,
    repository_root: Option<&Path>,
    visited: &mut HashSet<PathBuf>,
    depth: usize,
) -> Option<PathBuf> {
    if depth >= 8 {
        return None;
    }
    let identity = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    if !visited.insert(identity) {
        return None;
    }
    let contents = fs::read_to_string(path).ok()?;
    let mut section = ConfigSection::Other;
    let mut excludes = None;
    for raw in contents.lines() {
        let line = raw.trim();
        if line.starts_with('[') && line.ends_with(']') {
            section = parse_section(&line[1..line.len() - 1]);
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim().trim_matches('"');
        if (section == ConfigSection::Core && key.eq_ignore_ascii_case("excludesfile"))
            || key.trim().eq_ignore_ascii_case("core.excludesfile")
        {
            excludes = Some(resolve_config_path(path, value));
        } else if key.eq_ignore_ascii_case("path")
            && include_enabled(&section, repository_root, path)
        {
            let include = resolve_config_path(path, value);
            if let Some(included) = read_config(&include, repository_root, visited, depth + 1) {
                excludes = Some(included);
            }
        }
    }
    excludes
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ConfigSection {
    Core,
    Include,
    IncludeIf(String),
    Other,
}

fn parse_section(section: &str) -> ConfigSection {
    let mut parts = section.splitn(2, char::is_whitespace);
    let name = parts.next().unwrap_or_default();
    if name.eq_ignore_ascii_case("core") {
        ConfigSection::Core
    } else if name.eq_ignore_ascii_case("include") {
        ConfigSection::Include
    } else if name.eq_ignore_ascii_case("includeif") {
        let condition = parts.next().unwrap_or_default().trim().trim_matches('"');
        ConfigSection::IncludeIf(condition.to_owned())
    } else {
        ConfigSection::Other
    }
}

fn include_enabled(section: &ConfigSection, repository_root: Option<&Path>, config: &Path) -> bool {
    match section {
        ConfigSection::Include => true,
        ConfigSection::IncludeIf(condition) => {
            conditional_include_matches(condition, repository_root, config)
        }
        ConfigSection::Core | ConfigSection::Other => false,
    }
}

fn conditional_include_matches(
    condition: &str,
    repository_root: Option<&Path>,
    config: &Path,
) -> bool {
    if let Some(pattern) = condition.strip_prefix("gitdir:") {
        return gitdir_matches(pattern, repository_root, config, false);
    }
    if let Some(pattern) = condition.strip_prefix("gitdir/i:") {
        return gitdir_matches(pattern, repository_root, config, true);
    }
    if let Some(pattern) = condition.strip_prefix("onbranch:") {
        return branch_matches(pattern, repository_root);
    }
    false
}

fn gitdir_matches(
    pattern: &str,
    repository_root: Option<&Path>,
    config: &Path,
    case_insensitive: bool,
) -> bool {
    let Some(root) = repository_root else {
        return false;
    };
    let Some(git_directory) = resolve_git_directory(root) else {
        return false;
    };
    let mut pattern = conditional_pattern(pattern, config);
    let mut candidate = git_directory.to_string_lossy().replace('\\', "/");
    if case_insensitive {
        pattern.make_ascii_lowercase();
        candidate.make_ascii_lowercase();
    }
    crate::glob::matches(&pattern, &candidate)
}

fn conditional_pattern(pattern: &str, config: &Path) -> String {
    let mut resolved = if pattern.starts_with("~/") || pattern.starts_with("~\\") {
        expand_home(pattern).to_string_lossy().replace('\\', "/")
    } else if let Some(relative) = pattern.strip_prefix("./") {
        config
            .parent()
            .unwrap_or_else(|| Path::new(""))
            .join(relative)
            .to_string_lossy()
            .replace('\\', "/")
    } else if Path::new(pattern).is_absolute() {
        pattern.replace('\\', "/")
    } else {
        format!("**/{pattern}")
    };
    if resolved.ends_with('/') {
        resolved.push_str("**");
    }
    resolved
}

fn branch_matches(pattern: &str, repository_root: Option<&Path>) -> bool {
    let Some(repository_root) = repository_root else {
        return false;
    };
    let Some(git_directory) = resolve_git_directory(repository_root) else {
        return false;
    };
    let Ok(head) = fs::read_to_string(git_directory.join("HEAD")) else {
        return false;
    };
    let Some(branch) = head.trim().strip_prefix("ref: refs/heads/") else {
        return false;
    };
    let pattern = if pattern.ends_with('/') {
        format!("{pattern}**")
    } else {
        pattern.to_owned()
    };
    crate::glob::matches(&pattern, branch)
}

fn resolve_config_path(config: &Path, value: &str) -> PathBuf {
    let expanded = expand_home(value);
    if expanded.is_absolute() || value.starts_with('~') {
        expanded
    } else {
        config
            .parent()
            .unwrap_or_else(|| Path::new(""))
            .join(expanded)
    }
}

fn default_global_ignore() -> Option<PathBuf> {
    env::var_os("XDG_CONFIG_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| home_directory().map(|home| home.join(".config")))
        .map(|directory| directory.join("git").join("ignore"))
}

pub(super) fn expand_home(value: &str) -> PathBuf {
    if value == "~" {
        return home_directory().unwrap_or_else(|| PathBuf::from(value));
    }
    if let Some(suffix) = value
        .strip_prefix("~/")
        .or_else(|| value.strip_prefix("~\\"))
        && let Some(home) = home_directory()
    {
        return home.join(suffix);
    }
    PathBuf::from(value)
}

fn home_directory() -> Option<PathBuf> {
    env::var_os("HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            env::var_os("USERPROFILE")
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
        })
}
