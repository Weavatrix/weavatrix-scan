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

pub(super) fn gitconfig_excludes_path() -> Option<PathBuf> {
    if let Some(path) = env::var_os("GIT_CONFIG_GLOBAL").map(PathBuf::from)
        && !path.as_os_str().is_empty()
    {
        return read_excludes_setting(&path).or_else(default_global_ignore);
    }
    let home = home_directory();
    if let Some(path) = home.as_ref().map(|home| home.join(".gitconfig"))
        && let Some(excludes) = read_excludes_setting(&path)
    {
        return Some(excludes);
    }
    let xdg = env::var_os("XDG_CONFIG_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| home.as_ref().map(|home| home.join(".config")));
    if let Some(path) = xdg.as_ref().map(|xdg| xdg.join("git").join("config"))
        && let Some(excludes) = read_excludes_setting(&path)
    {
        return Some(excludes);
    }
    if let Some(path) = env::var_os("GIT_CONFIG_SYSTEM")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| (!cfg!(windows)).then(|| PathBuf::from("/etc/gitconfig")))
        && let Some(excludes) = read_excludes_setting(&path)
    {
        return Some(excludes);
    }
    default_global_ignore()
}

pub(super) fn read_excludes_setting(path: &Path) -> Option<PathBuf> {
    let contents = fs::read_to_string(path).ok()?;
    let mut in_core = false;
    for raw in contents.lines() {
        let line = raw.trim();
        if line.starts_with('[') && line.ends_with(']') {
            let section = line[1..line.len() - 1]
                .split_whitespace()
                .next()
                .unwrap_or_default();
            in_core = section.eq_ignore_ascii_case("core");
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if (in_core && key.trim().eq_ignore_ascii_case("excludesfile"))
            || key.trim().eq_ignore_ascii_case("core.excludesfile")
        {
            return Some(expand_home(value.trim().trim_matches('"')));
        }
    }
    None
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
