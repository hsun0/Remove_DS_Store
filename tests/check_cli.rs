use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use tempfile::tempdir;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

fn run(arguments: &[&str], current_dir: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rmds"))
        .args(arguments)
        .current_dir(current_dir)
        .env_remove("NO_COLOR")
        .stdin(Stdio::null())
        .output()
        .unwrap()
}

fn git(root: &Path, arguments: &[&str]) -> Output {
    Command::new("git")
        .current_dir(root)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .args(arguments)
        .output()
        .unwrap()
}

fn git_ok(root: &Path, arguments: &[&str]) {
    let output = git(root, arguments);
    assert!(
        output.status.success(),
        "git {arguments:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn init_repo(root: &Path) {
    fs::create_dir_all(root).unwrap();
    git_ok(root, &["init", "--quiet"]);
    git_ok(root, &["config", "user.name", "rmds check test"]);
    git_ok(root, &["config", "user.email", "rmds@example.invalid"]);
}

fn write_zip(path: &Path, entries: &[(&str, &[u8])]) {
    let mut writer = ZipWriter::new(File::create(path).unwrap());
    for (name, content) in entries {
        writer
            .start_file(
                *name,
                SimpleFileOptions::default()
                    .compression_method(CompressionMethod::Deflated)
                    .unix_permissions(0o644),
            )
            .unwrap();
        writer.write_all(content).unwrap();
    }
    writer.finish().unwrap();
}

fn snapshot_files(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut snapshot = BTreeMap::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(&directory).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            let file_type = entry.file_type().unwrap();
            if file_type.is_dir() {
                pending.push(path);
            } else if file_type.is_file() {
                snapshot.insert(
                    path.strip_prefix(root).unwrap().to_path_buf(),
                    fs::read(path).unwrap(),
                );
            }
        }
    }
    snapshot
}

fn assert_plain_and_non_interactive(output: &Output) {
    assert!(!output.stdout.contains(&0x1b));
    assert!(!output.stderr.contains(&0x1b));
    assert!(!String::from_utf8_lossy(&output.stdout).contains("Type DELETE"));
    assert!(!String::from_utf8_lossy(&output.stderr).contains("Type DELETE"));
}

#[test]
fn folder_check_uses_unified_exit_codes_and_changes_nothing() {
    let temp = tempdir().unwrap();
    let root = temp.path().join("folder with space");
    fs::create_dir(&root).unwrap();

    let clean = run(&["folder", "--check", root.to_str().unwrap()], temp.path());
    assert_eq!(clean.status.code(), Some(0));
    assert!(
        String::from_utf8_lossy(&clean.stdout).contains("Check passed: no macOS metadata found.")
    );
    assert_plain_and_non_interactive(&clean);

    fs::write(root.join(".DS_Store"), b"finder").unwrap();
    fs::write(root.join("._照片.jpg"), b"sidecar").unwrap();
    fs::write(root.join(".DS_Store.backup"), b"keep").unwrap();
    let before_ds_store = fs::read(root.join(".DS_Store")).unwrap();
    let before_sidecar = fs::read(root.join("._照片.jpg")).unwrap();

    let found = run(&["folder", "--check", root.to_str().unwrap()], temp.path());
    assert_eq!(found.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&found.stdout);
    assert!(stdout.contains(".DS_Store"));
    assert!(stdout.contains("._照片.jpg"));
    assert!(stdout.contains("Check failed: found 2 metadata entries."));
    assert!(!stdout.contains(".DS_Store.backup\n"));
    assert_plain_and_non_interactive(&found);
    assert_eq!(fs::read(root.join(".DS_Store")).unwrap(), before_ds_store);
    assert_eq!(fs::read(root.join("._照片.jpg")).unwrap(), before_sidecar);
    assert_eq!(fs::read(root.join(".DS_Store.backup")).unwrap(), b"keep");

    let missing = run(&["folder", "--check", "missing"], temp.path());
    assert_eq!(missing.status.code(), Some(2));

    fs::remove_dir_all(&root).unwrap();
    fs::write(temp.path().join(".DS_Store"), b"default").unwrap();
    let default_path = run(&["folder", "--check"], temp.path());
    assert_eq!(default_path.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&default_path.stdout)
            .contains("Check failed: found 1 metadata entry.")
    );

    let preview = run(&["folder"], temp.path());
    assert_eq!(preview.status.code(), Some(0));
}

#[test]
fn repo_check_is_non_interactive_and_preserves_git_state() {
    let temp = tempdir().unwrap();
    let root = temp.path().join("project");
    init_repo(&root);
    fs::write(root.join(".gitignore"), b"ignored/\n").unwrap();
    fs::write(root.join(".DS_Store"), b"tracked").unwrap();
    fs::write(root.join("README.md"), b"keep").unwrap();
    git_ok(&root, &["add", ".gitignore", ".DS_Store", "README.md"]);
    git_ok(&root, &["commit", "--quiet", "-m", "fixture"]);
    fs::write(root.join(".DS_Store"), b"tracked modified").unwrap();
    fs::write(root.join("._untracked"), b"untracked").unwrap();
    fs::create_dir(root.join("ignored")).unwrap();
    fs::write(root.join("ignored/.DS_Store"), b"ignored").unwrap();
    fs::create_dir_all(root.join("src/nested")).unwrap();

    let head_before = git(&root, &["rev-parse", "HEAD"]).stdout;
    let status_before = git(&root, &["status", "--porcelain=v1", "--ignored"]).stdout;
    let ignore_before = fs::read(root.join(".gitignore")).unwrap();
    let tracked_before = fs::read(root.join(".DS_Store")).unwrap();
    let git_directory_before = snapshot_files(&root.join(".git"));

    let output = run(
        &["repo", "--check", root.join("src/nested").to_str().unwrap()],
        temp.path(),
    );
    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("[tracked, modified"));
    assert!(stdout.contains("[untracked"));
    assert!(stdout.contains("[ignored"));
    assert!(stdout.contains("Check failed: found 3 metadata entries."));
    assert_plain_and_non_interactive(&output);

    assert_eq!(git(&root, &["rev-parse", "HEAD"]).stdout, head_before);
    assert_eq!(
        git(&root, &["status", "--porcelain=v1", "--ignored"]).stdout,
        status_before
    );
    assert_eq!(fs::read(root.join(".gitignore")).unwrap(), ignore_before);
    assert_eq!(fs::read(root.join(".DS_Store")).unwrap(), tracked_before);
    assert_eq!(snapshot_files(&root.join(".git")), git_directory_before);
    assert!(
        git(&root, &["diff", "--cached", "--quiet"])
            .status
            .success()
    );
}

#[test]
fn repo_check_clean_and_operational_errors_use_zero_and_two() {
    let temp = tempdir().unwrap();
    let root = temp.path().join("clean");
    init_repo(&root);

    let clean = run(&["repo", "--check", root.to_str().unwrap()], temp.path());
    assert_eq!(clean.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&clean.stdout).contains("Check passed"));

    let ordinary = temp.path().join("ordinary");
    fs::create_dir(&ordinary).unwrap();
    let non_repo = run(
        &["repo", "--check", ordinary.to_str().unwrap()],
        temp.path(),
    );
    assert_eq!(non_repo.status.code(), Some(2));

    let missing_git = Command::new(env!("CARGO_BIN_EXE_rmds"))
        .args(["repo", "--check", root.to_str().unwrap()])
        .current_dir(temp.path())
        .env("PATH", "")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_eq!(missing_git.status.code(), Some(2));
}

#[test]
fn zip_check_uses_unified_exit_codes_and_never_creates_output() {
    let temp = tempdir().unwrap();
    let clean_path = temp.path().join("clean.zip");
    write_zip(&clean_path, &[("README.md", b"keep")]);
    let clean_before = fs::read(&clean_path).unwrap();

    let clean = run(
        &["zip", "--check", clean_path.to_str().unwrap()],
        temp.path(),
    );
    assert_eq!(clean.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&clean.stdout).contains("Check passed"));
    assert_eq!(fs::read(&clean_path).unwrap(), clean_before);
    assert!(!temp.path().join("clean-clean.zip").exists());
    assert_plain_and_non_interactive(&clean);

    let metadata_path = temp.path().join("metadata.zip");
    write_zip(
        &metadata_path,
        &[(".DS_Store", b"one"), ("nested/._photo", b"two")],
    );
    let metadata_before = fs::read(&metadata_path).unwrap();
    let found = run(
        &["zip", "--check", metadata_path.to_str().unwrap()],
        temp.path(),
    );
    assert_eq!(found.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&found.stdout);
    assert!(stdout.contains(".DS_Store"));
    assert!(stdout.contains("nested/._photo"));
    assert!(stdout.contains("Check failed: found 2 metadata entries."));
    assert_eq!(fs::read(&metadata_path).unwrap(), metadata_before);
    assert!(!temp.path().join("metadata-clean.zip").exists());
    assert_plain_and_non_interactive(&found);

    let malformed = temp.path().join("malformed.zip");
    fs::write(&malformed, b"not a zip").unwrap();
    let invalid = run(
        &["zip", "--check", malformed.to_str().unwrap()],
        temp.path(),
    );
    assert_eq!(invalid.status.code(), Some(2));
}

#[test]
fn check_syntax_is_canonical_and_help_documents_all_modes() {
    let temp = tempdir().unwrap();
    for arguments in [
        &["folder", ".", "--check"][..],
        &["repo", ".", "--check"][..],
        &["zip", "archive.zip", "--check"][..],
        &["folder", "--check", "one", "two"][..],
        &["repo", "--check", "one", "two"][..],
        &["zip", "--check"][..],
        &["zip", "--check", "archive.zip", "extra.zip"][..],
        &["folder", "--check", "--apply", "."][..],
        &["repo", "--check", "--apply", "."][..],
        &["zip", "--check", "archive.zip", "-o", "output.zip"][..],
    ] {
        let output = run(arguments, temp.path());
        assert_eq!(output.status.code(), Some(2), "{arguments:?}");
    }

    for arguments in [
        &["--help"][..],
        &["folder", "--help"][..],
        &["repo", "--help"][..],
        &["zip", "--help"][..],
    ] {
        let output = run(arguments, temp.path());
        assert_eq!(output.status.code(), Some(0));
        assert!(String::from_utf8_lossy(&output.stdout).contains("--check"));
    }
}

#[test]
fn no_color_keeps_check_output_plain() {
    let temp = tempdir().unwrap();
    fs::write(temp.path().join(".DS_Store"), b"metadata").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_rmds"))
        .args(["folder", "--check"])
        .current_dir(temp.path())
        .env("NO_COLOR", "")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert_plain_and_non_interactive(&output);
}
