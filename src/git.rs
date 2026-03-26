use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq)]
pub struct GitContext {
    pub project_name: String,
    pub branch: Option<String>,
    pub is_worktree: bool,
    pub worktree_name: Option<String>,
    pub repo_root: PathBuf,
}

pub fn detect_git(path: &Path) -> Option<GitContext> {
    let repo = git2::Repository::discover(path).ok()?;

    let workdir = repo.workdir()?;

    let branch = repo
        .head()
        .ok()
        .and_then(|head| head.shorthand().map(|s| s.to_string()));

    // Use git2's built-in worktree detection
    let is_worktree = repo.is_worktree();

    let (project_name, worktree_name) = if is_worktree {
        let main_name = find_main_repo_name(&repo);
        // The worktree name is the workdir's own directory name
        let wt_name = workdir
            .file_name()
            .and_then(|n: &std::ffi::OsStr| n.to_str())
            .unwrap_or("")
            .to_string();
        (main_name, Some(wt_name))
    } else {
        let name = workdir
            .file_name()
            .and_then(|n: &std::ffi::OsStr| n.to_str())
            .unwrap_or("")
            .to_string();
        let name = strip_git_suffix(&name);
        (name, None)
    };

    let repo_root = workdir.to_path_buf();

    Some(GitContext {
        project_name,
        branch,
        is_worktree,
        worktree_name,
        repo_root,
    })
}

fn find_main_repo_name(repo: &git2::Repository) -> String {
    // For a worktree, repo.path() returns something like:
    // /path/to/main-repo/.git/worktrees/wt-name/
    // Navigate: wt-name -> worktrees -> .git -> main-repo
    let git_path = repo.path();
    // go up: worktrees/<name>/ -> worktrees/ -> .git/ -> main-repo/
    let main_git_dir = git_path.parent().and_then(|p| p.parent());
    let main_repo_dir = main_git_dir.and_then(|p| p.parent());

    if let Some(dir) = main_repo_dir {
        let name = dir
            .file_name()
            .and_then(|n: &std::ffi::OsStr| n.to_str())
            .unwrap_or("")
            .to_string();
        strip_git_suffix(&name)
    } else {
        String::new()
    }
}

fn strip_git_suffix(name: &str) -> String {
    if let Some(stripped) = name.strip_suffix(".git") {
        stripped.to_string()
    } else {
        name.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn make_initial_commit(repo: &git2::Repository) -> git2::Oid {
        let sig = git2::Signature::now("Test", "test@test.com").unwrap();
        let tree_id = repo.treebuilder(None).unwrap().write().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "init", &tree, &[])
            .unwrap()
    }

    #[test]
    fn test_detect_normal_git_repo() {
        let dir = TempDir::new().unwrap();
        let repo = git2::Repository::init(dir.path()).unwrap();
        make_initial_commit(&repo);

        let ctx = detect_git(dir.path()).unwrap();

        let expected_name = dir
            .path()
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        assert_eq!(ctx.project_name, expected_name);
        assert_eq!(ctx.branch, Some("master".to_string()));
        assert!(!ctx.is_worktree);
        assert_eq!(ctx.worktree_name, None);
    }

    #[test]
    fn test_detect_subdirectory() {
        let dir = TempDir::new().unwrap();
        let repo = git2::Repository::init(dir.path()).unwrap();
        make_initial_commit(&repo);

        let subdir = dir.path().join("src").join("nested");
        std::fs::create_dir_all(&subdir).unwrap();

        let ctx = detect_git(&subdir).unwrap();

        let expected_name = dir
            .path()
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        assert_eq!(ctx.project_name, expected_name);
        assert!(!ctx.is_worktree);
    }

    #[test]
    fn test_detect_non_git_directory() {
        let dir = TempDir::new().unwrap();
        // No git init, so discover should fail
        let result = detect_git(dir.path());
        assert!(result.is_none());
    }

    #[test]
    fn test_detect_worktree() {
        let main_dir = TempDir::new().unwrap();
        let repo = git2::Repository::init(main_dir.path()).unwrap();
        let commit_oid = make_initial_commit(&repo);
        let commit = repo.find_commit(commit_oid).unwrap();

        // Create a branch for the worktree
        repo.branch("wt-branch", &commit, false).unwrap();

        // Create worktree in a separate temp directory
        // Note: git2's worktree() creates the directory itself, so we just provide the path
        let wt_parent = TempDir::new().unwrap();
        let wt_dir = wt_parent.path().join("my_worktree");

        repo.worktree(
            "my_worktree",
            &wt_dir,
            Some(
                git2::WorktreeAddOptions::new().reference(Some(
                    &repo
                        .find_branch("wt-branch", git2::BranchType::Local)
                        .unwrap()
                        .into_reference(),
                )),
            ),
        )
        .unwrap();

        let ctx = detect_git(&wt_dir).unwrap();

        let expected_main_name = main_dir
            .path()
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        assert_eq!(ctx.project_name, expected_main_name);
        assert!(ctx.is_worktree);
        assert_eq!(ctx.worktree_name, Some("my_worktree".to_string()));
        assert_eq!(ctx.branch, Some("wt-branch".to_string()));
    }

    #[test]
    fn test_detect_branch_name() {
        let dir = TempDir::new().unwrap();
        let repo = git2::Repository::init(dir.path()).unwrap();
        let commit_oid = make_initial_commit(&repo);
        let commit = repo.find_commit(commit_oid).unwrap();

        // Create and checkout a feature branch
        let branch = repo.branch("feature/my-feature", &commit, false).unwrap();
        repo.set_head(branch.into_reference().name().unwrap())
            .unwrap();

        let ctx = detect_git(dir.path()).unwrap();

        assert_eq!(ctx.branch, Some("feature/my-feature".to_string()));
    }
}
