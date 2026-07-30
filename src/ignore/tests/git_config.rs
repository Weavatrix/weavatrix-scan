use super::*;
use crate::ignore::git::read_excludes_setting_for;

#[test]
fn git_config_honors_includes_and_repository_conditions() {
    let root =
        std::env::temp_dir().join(format!("weavatrix-ignore-includes-{}", std::process::id()));
    let repository = root.join("repository");
    let git_directory = repository.join(".git");
    std::fs::create_dir_all(&git_directory).unwrap();
    std::fs::write(git_directory.join("HEAD"), "ref: refs/heads/main\n").unwrap();
    std::fs::write(
        root.join("gitdir-config"),
        "[core]\n  excludesFile = gitdir-ignore\n",
    )
    .unwrap();
    std::fs::write(
        root.join("branch-config"),
        "[core]\n  excludesFile = branch-ignore\n",
    )
    .unwrap();

    let git_pattern = git_directory.to_string_lossy().replace('\\', "/");
    std::fs::write(
        root.join("config"),
        format!(
            "[includeIf \"gitdir:{git_pattern}/\"]\n  path = gitdir-config\n\
             [includeIf \"onbranch:main\"]\n  path = branch-config\n"
        ),
    )
    .unwrap();

    assert_eq!(
        read_excludes_setting_for(&root.join("config"), Some(&repository)).as_deref(),
        Some(root.join("branch-ignore").as_path())
    );
    assert_eq!(read_excludes_setting_for(&root.join("config"), None), None);

    let _ = std::fs::remove_dir_all(root);
}
