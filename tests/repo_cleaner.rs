use std::collections::BTreeMap;
#[cfg(target_os = "linux")]
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

#[cfg(unix)]
use rmds::folder_cleaner::CandidateType;
use rmds::repo_cleaner::{RepoGitStatus, apply_repo_cleanup, scan_repo};
use tempfile::tempdir;

fn write(path: &Path, contents: &[u8]) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, contents).unwrap();
}

fn git(root: &Path, arguments: &[&str]) -> Vec<u8> {
    let output = Command::new("git")
        .current_dir(root)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .args(arguments)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {arguments:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

fn init_repo(root: &Path) {
    fs::create_dir_all(root).unwrap();
    git(root, &["init", "--quiet"]);
    git(root, &["config", "user.name", "rmds test"]);
    git(root, &["config", "user.email", "rmds@example.invalid"]);

    // Prevent background Git maintenance from changing .git during tests.
    git(root, &["config", "maintenance.auto", "false"]);
    git(root, &["config", "gc.auto", "0"]);
}

fn commit_all(root: &Path, message: &str) {
    git(root, &["add", "--all"]);
    git(root, &["commit", "--quiet", "-m", message]);
}

fn statuses(root: &Path) -> BTreeMap<PathBuf, RepoGitStatus> {
    scan_repo(root)
        .unwrap()
        .candidates()
        .iter()
        .map(|candidate| {
            (
                candidate.relative_path().to_path_buf(),
                candidate.git_status(),
            )
        })
        .collect()
}

fn git_internal_snapshot(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let git_dir = root.join(".git");
    let mut directories = vec![git_dir.clone()];
    let mut snapshot = BTreeMap::new();
    while let Some(directory) = directories.pop() {
        for entry in fs::read_dir(&directory).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            let relative = path.strip_prefix(&git_dir).unwrap().to_path_buf();
            let metadata = fs::symlink_metadata(&path).unwrap();
            if metadata.is_dir() {
                snapshot.insert(relative.clone(), b"<directory>".to_vec());
                directories.push(path);
            } else {
                snapshot.insert(relative, fs::read(path).unwrap());
            }
        }
    }
    snapshot
}

#[test]
fn resolves_a_subdirectory_and_classifies_all_git_states() {
    let temp = tempdir().unwrap();
    let root = temp.path().join("project with spaces");
    init_repo(&root);
    write(&root.join(".gitignore"), b"images/._*\n__MACOSX/\n");
    write(&root.join("README.md"), b"keep");
    write(&root.join("tracked/.DS_Store"), b"tracked");
    write(&root.join("docs/._manual.pdf"), b"original");
    write(&root.join("__MACOSX/tracked"), b"tracked tree content");
    git(&root, &["add", "--all"]);
    git(&root, &["add", "--force", "__MACOSX/tracked"]);
    git(&root, &["commit", "--quiet", "-m", "fixture"]);

    write(&root.join("docs/._manual.pdf"), b"modified");
    write(&root.join(".DS_Store"), b"untracked");
    write(&root.join("images/._logo.png"), b"ignored");
    write(&root.join("__MACOSX/ignored"), b"ignored tree content");
    fs::create_dir_all(root.join("src/nested")).unwrap();

    let scan = scan_repo(&root.join("src/nested")).unwrap();
    assert_eq!(scan.root(), fs::canonicalize(&root).unwrap());
    let paths: Vec<_> = scan
        .candidates()
        .iter()
        .map(|candidate| candidate.relative_path().to_path_buf())
        .collect();
    let mut sorted = paths.clone();
    sorted.sort();
    assert_eq!(paths, sorted);

    let states = statuses(&root);
    assert_eq!(states[Path::new(".DS_Store")], RepoGitStatus::Untracked);
    assert_eq!(states[Path::new("__MACOSX")], RepoGitStatus::Mixed);
    assert_eq!(
        states[Path::new("docs/._manual.pdf")],
        RepoGitStatus::TrackedModified
    );
    assert_eq!(
        states[Path::new("images/._logo.png")],
        RepoGitStatus::Ignored
    );
    assert_eq!(
        states[Path::new("tracked/.DS_Store")],
        RepoGitStatus::Tracked
    );
    assert_eq!(fs::read(root.join("README.md")).unwrap(), b"keep");
}

#[test]
fn git_directory_and_normal_files_are_unchanged_by_scan_and_cleanup() {
    let temp = tempdir().unwrap();
    let root = temp.path().join("project");
    init_repo(&root);
    write(&root.join("README.md"), b"keep");
    write(&root.join("tracked/.DS_Store"), b"metadata");
    commit_all(&root, "fixture");
    write(&root.join(".DS_Store"), b"untracked metadata");
    write(&root.join(".git/.DS_Store"), b"internal keep");
    write(&root.join(".git/._internal"), b"internal keep");
    write(&root.join(".git/__MACOSX/._internal"), b"internal keep");

    let before = git_internal_snapshot(&root);
    let scan = scan_repo(&root).unwrap();
    assert_eq!(scan.candidates().len(), 2);
    assert!(
        scan.candidates()
            .iter()
            .all(|candidate| !candidate.relative_path().starts_with(".git"))
    );
    assert_eq!(git_internal_snapshot(&root), before);

    let report = apply_repo_cleanup(&scan).unwrap();
    assert_eq!(report.removed.len(), 2);
    assert_eq!(git_internal_snapshot(&root), before);
    assert_eq!(fs::read(root.join("README.md")).unwrap(), b"keep");
    assert_eq!(
        fs::read(root.join(".git/.DS_Store")).unwrap(),
        b"internal keep"
    );
    assert_eq!(
        fs::read(root.join(".git/._internal")).unwrap(),
        b"internal keep"
    );
    assert!(root.join(".git/__MACOSX").is_dir());
}

#[test]
fn nested_repositories_and_git_file_boundaries_are_skipped() {
    let temp = tempdir().unwrap();
    let root = temp.path().join("outer");
    init_repo(&root);
    write(&root.join(".DS_Store"), b"outer metadata");

    let nested = root.join("vendor/nested");
    init_repo(&nested);
    write(&nested.join(".DS_Store"), b"nested metadata");

    let submodule_like = root.join("modules/child");
    write(&submodule_like.join(".git"), b"gitdir: elsewhere");
    write(&submodule_like.join("._metadata"), b"submodule metadata");

    let scan = scan_repo(&root).unwrap();
    assert_eq!(
        scan.candidates()
            .iter()
            .map(|candidate| candidate.relative_path())
            .collect::<Vec<_>>(),
        [Path::new(".DS_Store")]
    );
    apply_repo_cleanup(&scan).unwrap();
    assert!(nested.join(".DS_Store").is_file());
    assert!(submodule_like.join("._metadata").is_file());
    assert_eq!(
        fs::read(submodule_like.join(".git")).unwrap(),
        b"gitdir: elsewhere"
    );
}

#[test]
fn a_git_boundary_anywhere_inside_macosx_preserves_the_entire_tree() {
    let temp = tempdir().unwrap();
    let root = temp.path().join("project");
    init_repo(&root);
    write(&root.join("__MACOSX/content/._metadata"), b"metadata");
    write(
        &root.join("__MACOSX/vendor/nested/.git"),
        b"gitdir: elsewhere",
    );

    assert!(scan_repo(&root).unwrap().is_empty());
    assert!(root.join("__MACOSX/content/._metadata").is_file());
}

#[test]
fn invalid_non_repository_and_bare_targets_are_rejected() {
    let temp = tempdir().unwrap();
    let missing = temp.path().join("missing");
    assert!(
        scan_repo(&missing)
            .unwrap_err()
            .to_string()
            .contains("not found")
    );

    let file = temp.path().join("file.txt");
    write(&file, b"file");
    assert!(
        scan_repo(&file)
            .unwrap_err()
            .to_string()
            .contains("not a directory")
    );

    let ordinary = temp.path().join("ordinary");
    fs::create_dir(&ordinary).unwrap();
    assert!(scan_repo(&ordinary).is_err());

    let bare = temp.path().join("bare.git");
    fs::create_dir(&bare).unwrap();
    git(&bare, &["init", "--bare", "--quiet"]);
    assert!(
        scan_repo(&bare)
            .unwrap_err()
            .to_string()
            .contains("bare repositories")
    );
}

#[cfg(unix)]
#[test]
fn filesystem_root_and_symbolic_link_target_are_rejected() {
    use std::os::unix::fs::symlink;

    assert!(scan_repo(Path::new("/")).is_err());

    let temp = tempdir().unwrap();
    let root = temp.path().join("project");
    init_repo(&root);
    let link = temp.path().join("project-link");
    symlink(&root, &link).unwrap();
    assert!(
        scan_repo(&link)
            .unwrap_err()
            .to_string()
            .contains("symbolic-link")
    );
}

#[cfg(unix)]
#[test]
fn metadata_symlinks_are_removed_without_following_targets() {
    use std::os::unix::fs::symlink;

    let temp = tempdir().unwrap();
    let root = temp.path().join("project");
    let outside = temp.path().join("outside");
    init_repo(&root);
    write(&outside.join("keep.txt"), b"keep");
    symlink(&outside, root.join("__MACOSX")).unwrap();
    symlink(outside.join("keep.txt"), root.join("._outside")).unwrap();

    let scan = scan_repo(&root).unwrap();
    assert!(
        scan.candidates()
            .iter()
            .all(|candidate| candidate.candidate_type() == CandidateType::Symlink)
    );
    apply_repo_cleanup(&scan).unwrap();
    assert_eq!(fs::read(outside.join("keep.txt")).unwrap(), b"keep");
}

#[test]
fn revalidation_rejects_changed_type_before_deleting_anything() {
    let temp = tempdir().unwrap();
    let root = temp.path().join("project");
    init_repo(&root);
    write(&root.join(".DS_Store"), b"metadata");
    write(&root.join("z/._other"), b"metadata");
    let scan = scan_repo(&root).unwrap();

    fs::remove_file(root.join("z/._other")).unwrap();
    fs::create_dir(root.join("z/._other")).unwrap();
    let error = apply_repo_cleanup(&scan).unwrap_err();
    assert!(error.to_string().contains("changed type"));
    assert!(error.removed().is_empty());
    assert!(root.join(".DS_Store").is_file());
}

#[cfg(unix)]
#[test]
fn revalidation_rejects_a_parent_replaced_by_a_symlink() {
    use std::os::unix::fs::symlink;

    let temp = tempdir().unwrap();
    let root = temp.path().join("project");
    let outside = temp.path().join("outside");
    init_repo(&root);
    write(&root.join("nested/._item"), b"inside");
    write(&outside.join("._item"), b"outside");
    let scan = scan_repo(&root).unwrap();

    fs::rename(root.join("nested"), root.join("original-nested")).unwrap();
    symlink(&outside, root.join("nested")).unwrap();
    let error = apply_repo_cleanup(&scan).unwrap_err();
    assert!(error.to_string().contains("candidate parent changed"));
    assert!(error.removed().is_empty());
    assert_eq!(fs::read(outside.join("._item")).unwrap(), b"outside");
}

#[test]
fn revalidation_rejects_a_new_nested_boundary_inside_macosx() {
    let temp = tempdir().unwrap();
    let root = temp.path().join("project");
    init_repo(&root);
    write(&root.join("__MACOSX/._item"), b"metadata");
    let scan = scan_repo(&root).unwrap();
    write(&root.join("__MACOSX/vendor/.git"), b"gitdir: elsewhere");

    let error = apply_repo_cleanup(&scan).unwrap_err();
    assert!(error.to_string().contains("nested repository boundary"));
    assert!(error.removed().is_empty());
    assert!(root.join("__MACOSX/._item").is_file());
}

#[test]
fn tracked_deletion_is_not_staged_and_gitignore_is_unchanged() {
    let temp = tempdir().unwrap();
    let root = temp.path().join("project");
    init_repo(&root);
    write(&root.join(".gitignore"), b"# user rules\n");
    write(&root.join("tracked/.DS_Store"), b"metadata");
    commit_all(&root, "fixture");
    let head_before = git(&root, &["rev-parse", "HEAD"]);
    let ignore_before = fs::read(root.join(".gitignore")).unwrap();

    let scan = scan_repo(&root).unwrap();
    apply_repo_cleanup(&scan).unwrap();

    assert_eq!(git(&root, &["rev-parse", "HEAD"]), head_before);
    assert_eq!(fs::read(root.join(".gitignore")).unwrap(), ignore_before);
    assert!(git(&root, &["diff", "--cached", "--name-only"]).is_empty());
    assert_eq!(
        String::from_utf8(git(&root, &["diff", "--name-only"])).unwrap(),
        "tracked/.DS_Store\n"
    );
}

#[test]
fn linked_worktree_git_file_and_common_directory_are_unchanged() {
    let temp = tempdir().unwrap();
    let main = temp.path().join("main");
    let worktree = temp.path().join("linked-worktree");
    init_repo(&main);
    write(&main.join("README.md"), b"keep");
    commit_all(&main, "fixture");
    git(
        &main,
        &[
            "worktree",
            "add",
            "--quiet",
            "--detach",
            worktree.to_str().unwrap(),
        ],
    );
    write(&worktree.join(".DS_Store"), b"metadata");

    let git_file_before = fs::read(worktree.join(".git")).unwrap();
    let common_before = git_internal_snapshot(&main);
    let scan = scan_repo(&worktree).unwrap();
    assert_ne!(scan.git_dir(), scan.git_common_dir());
    apply_repo_cleanup(&scan).unwrap();

    assert_eq!(fs::read(worktree.join(".git")).unwrap(), git_file_before);
    assert_eq!(git_internal_snapshot(&main), common_before);
    assert!(!worktree.join(".DS_Store").exists());
    assert_eq!(fs::read(worktree.join("README.md")).unwrap(), b"keep");
}

#[test]
fn missing_candidate_is_rejected_before_any_deletion() {
    let temp = tempdir().unwrap();
    let root = temp.path().join("project");
    init_repo(&root);
    write(&root.join(".DS_Store"), b"first");
    write(&root.join("z/._second"), b"second");
    let scan = scan_repo(&root).unwrap();

    fs::remove_file(root.join("z/._second")).unwrap();
    let error = apply_repo_cleanup(&scan).unwrap_err();
    assert!(error.to_string().contains("changed or disappeared"));
    assert!(error.removed().is_empty());
    assert!(root.join(".DS_Store").is_file());
}

#[cfg(unix)]
#[test]
fn scan_failure_never_deletes_candidates_already_seen() {
    use std::os::unix::fs::PermissionsExt;

    let temp = tempdir().unwrap();
    let root = temp.path().join("project");
    let blocked = root.join("blocked");
    init_repo(&root);
    write(&root.join(".DS_Store"), b"must remain");
    write(&blocked.join("normal.txt"), b"blocked");
    fs::set_permissions(&blocked, fs::Permissions::from_mode(0o000)).unwrap();

    let result = scan_repo(&root);
    fs::set_permissions(&blocked, fs::Permissions::from_mode(0o700)).unwrap();
    if result.is_err() {
        assert!(root.join(".DS_Store").is_file());
    }
}

#[cfg(target_os = "linux")]
#[test]
fn non_utf8_appledouble_filename_is_supported() {
    use std::os::unix::ffi::OsStrExt;

    let temp = tempdir().unwrap();
    let root = temp.path().join("project");
    init_repo(&root);
    let name = OsStr::from_bytes(b"._photo-\xff.jpg");
    write(&root.join(name), b"metadata");

    let scan = scan_repo(&root).unwrap();
    assert_eq!(scan.candidates().len(), 1);
    assert_eq!(scan.candidates()[0].git_status(), RepoGitStatus::Untracked);
    apply_repo_cleanup(&scan).unwrap();
    assert!(fs::symlink_metadata(root.join(name)).is_err());
}

#[cfg(unix)]
#[test]
fn partial_failure_reports_items_removed_before_the_error() {
    use std::os::unix::fs::PermissionsExt;

    let temp = tempdir().unwrap();
    let root = temp.path().join("project");
    let locked = root.join("locked");
    init_repo(&root);
    write(&root.join(".DS_Store"), b"first");
    write(&locked.join("._second"), b"second");
    let scan = scan_repo(&root).unwrap();
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o500)).unwrap();

    let result = apply_repo_cleanup(&scan);
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o700)).unwrap();
    if let Err(error) = result {
        assert_eq!(error.removed(), [PathBuf::from(".DS_Store")]);
        assert!(!root.join(".DS_Store").exists());
        assert!(locked.join("._second").exists());
    }
}
