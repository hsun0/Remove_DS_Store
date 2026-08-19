use std::fs;
use std::path::Path;
use std::process::{Command, Output, Stdio};

use tempfile::tempdir;

fn run(arguments: &[&str], current_dir: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rmds"))
        .args(arguments)
        .current_dir(current_dir)
        .stdin(Stdio::piped())
        .output()
        .unwrap()
}

#[test]
fn folder_defaults_to_a_read_only_current_directory_preview() {
    let temp = tempdir().unwrap();
    fs::write(temp.path().join(".DS_Store"), b"finder").unwrap();

    let output = run(&["folder"], temp.path());
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Found macOS metadata:"));
    assert!(stdout.contains(".DS_Store"));
    assert!(stdout.contains("No files were removed."));
    assert!(stdout.contains("rmds folder --apply ."));
    assert!(temp.path().join(".DS_Store").is_file());
}

#[test]
fn folder_preview_with_an_explicit_path_is_read_only() {
    let temp = tempdir().unwrap();
    let target = temp.path().join("folder");
    fs::create_dir(&target).unwrap();
    fs::write(target.join("._photo.jpg"), b"sidecar").unwrap();

    let output = run(&["folder", target.to_str().unwrap()], temp.path());
    assert!(output.status.success());
    assert!(target.join("._photo.jpg").is_file());
}

#[test]
fn apply_lists_the_plan_but_refuses_non_interactive_input() {
    let temp = tempdir().unwrap();
    let target = temp.path().join("folder");
    fs::create_dir(&target).unwrap();
    fs::write(target.join(".DS_Store"), b"finder").unwrap();

    let output = run(
        &["folder", "--apply", target.to_str().unwrap()],
        temp.path(),
    );
    assert!(!output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stdout.contains("will be permanently removed:"));
    assert!(stdout.contains("WARNING: This is an in-place operation."));
    assert!(stderr.contains("requires an interactive terminal"));
    assert!(target.join(".DS_Store").is_file());
}

#[test]
fn apply_with_no_candidates_succeeds_without_a_prompt() {
    let temp = tempdir().unwrap();
    let target = temp.path().join("folder");
    fs::create_dir(&target).unwrap();

    let output = run(
        &["folder", "--apply", target.to_str().unwrap()],
        temp.path(),
    );
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("No macOS metadata found"));
}

#[test]
fn apply_requires_an_explicit_path_and_canonical_argument_order() {
    let temp = tempdir().unwrap();
    let missing = run(&["folder", "--apply"], temp.path());
    assert_eq!(missing.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&missing.stderr).contains("apply mode requires an explicit path")
    );

    let reversed = run(&["folder", ".", "--apply"], temp.path());
    assert_eq!(reversed.status.code(), Some(2));
}

#[test]
fn folder_help_documents_both_modes() {
    let temp = tempdir().unwrap();
    let output = run(&["folder", "--help"], temp.path());
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("rmds folder [PATH]"));
    assert!(stdout.contains("rmds folder --apply <PATH>"));
    assert!(stdout.contains("exactly DELETE"));
}

#[test]
fn main_help_lists_zip_and_folder() {
    let temp = tempdir().unwrap();
    let output = run(&["--help"], temp.path());
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("zip"));
    assert!(stdout.contains("folder"));
}

#[test]
fn invalid_folder_target_returns_failure_without_deleting() {
    let temp = tempdir().unwrap();
    let output = run(&["folder", "missing"], temp.path());
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("No files were removed."));
}
