use std::fs;
use std::path::Path;
use std::process::{Command, Output, Stdio};

use tempfile::tempdir;

fn git(root: &Path, arguments: &[&str]) {
    let output = Command::new("git")
        .current_dir(root)
        .args(arguments)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {arguments:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn init_repo(root: &Path) {
    fs::create_dir_all(root).unwrap();
    git(root, &["init", "--quiet"]);
}

fn run(arguments: &[&str], current_dir: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rmds"))
        .args(arguments)
        .current_dir(current_dir)
        .stdin(Stdio::piped())
        .output()
        .unwrap()
}

#[test]
fn repo_defaults_to_current_directory_and_skips_confirmation_when_empty() {
    let temp = tempdir().unwrap();
    init_repo(temp.path());

    let output = run(&["repo"], temp.path());
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Repository:"));
    assert!(stdout.contains("No macOS metadata found."));
    assert!(stdout.contains("No files were removed."));
    assert!(stdout.contains("Suggested .gitignore entries:"));
    assert!(!stdout.contains("Type DELETE"));
}

#[test]
fn repo_path_inside_working_tree_resolves_the_repository_root() {
    let temp = tempdir().unwrap();
    let root = temp.path().join("project");
    let child = root.join("src/nested");
    init_repo(&root);
    fs::create_dir_all(&child).unwrap();

    let output = run(&["repo", child.to_str().unwrap()], temp.path());
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains(&fs::canonicalize(&root).unwrap().display().to_string()));
}

#[test]
fn repo_lists_candidates_and_refuses_non_interactive_deletion() {
    let temp = tempdir().unwrap();
    let root = temp.path().join("project");
    init_repo(&root);
    fs::write(root.join(".DS_Store"), b"metadata").unwrap();

    let output = run(&["repo", root.to_str().unwrap()], temp.path());
    assert!(!output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stdout.contains("Found macOS metadata:"));
    assert!(stdout.contains("[untracked"));
    assert!(stdout.contains(".DS_Store"));
    assert!(stdout.contains("WARNING: This is an in-place operation."));
    assert!(stderr.contains("repository deletion requires an interactive terminal"));
    assert!(root.join(".DS_Store").is_file());
}

#[test]
fn repo_apply_syntax_and_extra_paths_are_rejected() {
    let temp = tempdir().unwrap();
    for arguments in [
        &["repo", "--apply"][..],
        &["repo", "--apply", "."][..],
        &["repo", ".", "--apply"][..],
        &["repo", "one", "two"][..],
        &["repo", "--unknown"][..],
    ] {
        let output = run(arguments, temp.path());
        assert_eq!(output.status.code(), Some(2), "{arguments:?}");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("rmds repo [PATH]"),
            "{arguments:?}"
        );
    }
}

#[test]
fn repo_help_documents_the_single_safe_flow() {
    let temp = tempdir().unwrap();
    let output = run(&["repo", "--help"], temp.path());
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("rmds repo [PATH]"));
    assert!(stdout.contains("exactly DELETE"));
    assert!(stdout.contains("There is no\n--apply mode"));
    assert!(stdout.contains("never traverses or modifies .git"));
    assert!(stdout.contains("does not edit .gitignore"));
    assert!(stdout.contains("automatic rollback"));
}

#[test]
fn invalid_and_non_repository_targets_fail_without_removal() {
    let temp = tempdir().unwrap();
    let missing = run(&["repo", "missing"], temp.path());
    assert!(!missing.status.success());
    assert!(String::from_utf8_lossy(&missing.stderr).contains("No files were removed."));

    let ordinary = temp.path().join("ordinary");
    fs::create_dir(&ordinary).unwrap();
    let output = run(&["repo", ordinary.to_str().unwrap()], temp.path());
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("No files were removed."));
}

#[test]
fn missing_git_executable_is_reported_only_by_repo_mode() {
    let temp = tempdir().unwrap();
    fs::create_dir(temp.path().join("project")).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_rmds"))
        .args(["repo", "project"])
        .current_dir(temp.path())
        .env("PATH", "")
        .stdin(Stdio::piped())
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("cannot execute Git"));

    let version = Command::new(env!("CARGO_BIN_EXE_rmds"))
        .arg("--version")
        .current_dir(temp.path())
        .env("PATH", "")
        .output()
        .unwrap();
    assert!(version.status.success());
}
