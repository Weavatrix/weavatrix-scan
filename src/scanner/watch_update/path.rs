use super::{Component, Path, normalized_relative_path};

pub(crate) fn is_safe_relative(relative: &str) -> bool {
    !relative.is_empty()
        && normalized_relative_path(Path::new(relative)) == relative
        && Path::new(relative).components().all(|component| {
            !matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
}
