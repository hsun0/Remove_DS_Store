use std::fs;
use std::path::{Path, PathBuf};

use rmds::folder_cleaner::{CandidateType, apply_folder_cleanup, scan_folder};
use tempfile::tempdir;

fn write(path: &Path, contents: &[u8]) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, contents).unwrap();
}

fn relative_paths(root: &Path) -> Vec<PathBuf> {
    scan_folder(root)
        .unwrap()
        .candidates()
        .iter()
        .map(|candidate| candidate.relative_path().to_path_buf())
        .collect()
}

#[test]
fn preview_finds_only_supported_metadata_and_changes_nothing() {
    let temp = tempdir().unwrap();
    let root = temp.path().join("project");

    write(&root.join(".DS_Store"), b"finder");
    write(&root.join("._photo.jpg"), b"sidecar");
    write(&root.join("nested/.DS_Store"), b"nested");
    write(&root.join("__MACOSX/ignored/._inside"), b"tree");
    write(&root.join("named/.DS_Store/._inside"), b"descend");
    write(&root.join("named/._cache/.DS_Store"), b"descend");
    write(&root.join("nested/__MACOSX"), b"regular file");

    for safe in [
        "README.md",
        ".gitignore",
        ".env",
        ".hidden",
        "DS_Store.txt",
        ".DS_Store.backup",
        "foo._bar",
        "__MACOSX-file",
    ] {
        write(&root.join(safe), b"keep");
    }

    let scan = scan_folder(&root).unwrap();
    assert_eq!(scan.root(), fs::canonicalize(&root).unwrap());

    let paths: Vec<_> = scan
        .candidates()
        .iter()
        .map(|candidate| candidate.relative_path().to_path_buf())
        .collect();
    let mut sorted = paths.clone();
    sorted.sort();
    assert_eq!(paths, sorted, "candidate output must be deterministic");
    assert_eq!(
        paths,
        [
            PathBuf::from(".DS_Store"),
            PathBuf::from("._photo.jpg"),
            PathBuf::from("__MACOSX"),
            PathBuf::from("named/.DS_Store/._inside"),
            PathBuf::from("named/._cache/.DS_Store"),
            PathBuf::from("nested/.DS_Store"),
        ]
    );
    assert_eq!(
        scan.candidates()
            .iter()
            .find(|candidate| candidate.relative_path() == Path::new("__MACOSX"))
            .unwrap()
            .candidate_type(),
        CandidateType::Directory
    );

    let second_preview = scan_folder(&root).unwrap();
    assert_eq!(scan, second_preview);

    for path in &paths {
        assert!(
            root.join(path).exists(),
            "preview removed {}",
            path.display()
        );
    }
    assert!(root.join("nested/__MACOSX").is_file());
    assert!(root.join("named/.DS_Store").is_dir());
    assert!(root.join("named/._cache").is_dir());
}

#[test]
fn apply_removes_the_plan_and_is_idempotent() {
    let temp = tempdir().unwrap();
    let root = temp.path().join("project");
    write(&root.join(".DS_Store"), b"finder");
    write(&root.join("assets/._logo.png"), b"sidecar");
    write(&root.join("__MACOSX/assets/._logo.png"), b"tree");
    write(&root.join("assets/logo.png"), b"keep");

    let scan = scan_folder(&root).unwrap();
    let expected = relative_paths(&root);
    let report = apply_folder_cleanup(&scan).unwrap();
    assert_eq!(report.removed, expected);
    assert!(!root.join(".DS_Store").exists());
    assert!(!root.join("assets/._logo.png").exists());
    assert!(!root.join("__MACOSX").exists());
    assert_eq!(fs::read(root.join("assets/logo.png")).unwrap(), b"keep");

    let second = scan_folder(&root).unwrap();
    assert!(second.is_empty());
    assert!(apply_folder_cleanup(&second).unwrap().removed.is_empty());
}

#[test]
fn unicode_and_space_paths_are_supported() {
    let temp = tempdir().unwrap();
    let root = temp.path().join("有空白 的專案");
    write(&root.join("圖片 資料/._照片.png"), b"sidecar");
    write(&root.join("圖片 資料/照片.png"), b"keep");

    let scan = scan_folder(&root).unwrap();
    assert_eq!(
        scan.candidates()[0].relative_path(),
        Path::new("圖片 資料/._照片.png")
    );
    apply_folder_cleanup(&scan).unwrap();
    assert_eq!(fs::read(root.join("圖片 資料/照片.png")).unwrap(), b"keep");
}

#[test]
fn a_folder_containing_only_metadata_is_cleaned_but_the_root_remains() {
    let temp = tempdir().unwrap();
    let root = temp.path().join("only-metadata");
    write(&root.join(".DS_Store"), b"finder");
    write(&root.join("._file"), b"sidecar");
    write(&root.join("__MACOSX/._file"), b"tree");

    let scan = scan_folder(&root).unwrap();
    assert_eq!(scan.candidates().len(), 3);
    apply_folder_cleanup(&scan).unwrap();
    assert!(root.is_dir());
    assert_eq!(fs::read_dir(&root).unwrap().count(), 0);
}

#[test]
fn apply_preflight_rejects_a_candidate_that_changed_type() {
    let temp = tempdir().unwrap();
    let root = temp.path().join("project");
    write(&root.join(".DS_Store"), b"finder");
    let scan = scan_folder(&root).unwrap();

    fs::remove_file(root.join(".DS_Store")).unwrap();
    fs::create_dir(root.join(".DS_Store")).unwrap();

    let error = apply_folder_cleanup(&scan).unwrap_err();
    assert!(error.to_string().contains("changed type"));
    assert!(error.removed().is_empty());
    assert!(root.join(".DS_Store").is_dir());
}

#[test]
fn apply_preflight_rejects_a_missing_candidate_before_deleting_anything() {
    let temp = tempdir().unwrap();
    let root = temp.path().join("project");
    write(&root.join(".DS_Store"), b"finder");
    write(&root.join("z/._other"), b"sidecar");
    let scan = scan_folder(&root).unwrap();

    fs::remove_file(root.join("z/._other")).unwrap();
    let error = apply_folder_cleanup(&scan).unwrap_err();
    assert!(error.to_string().contains("changed or disappeared"));
    assert!(error.removed().is_empty());
    assert!(root.join(".DS_Store").is_file());
}

#[test]
fn invalid_targets_are_rejected() {
    let temp = tempdir().unwrap();
    let missing = temp.path().join("missing");
    assert!(
        scan_folder(&missing)
            .unwrap_err()
            .to_string()
            .contains("not found")
    );

    let file = temp.path().join("file.txt");
    write(&file, b"not a directory");
    assert!(
        scan_folder(&file)
            .unwrap_err()
            .to_string()
            .contains("not a directory")
    );
}

#[cfg(unix)]
#[test]
fn filesystem_root_is_rejected() {
    assert!(
        scan_folder(Path::new("/"))
            .unwrap_err()
            .to_string()
            .contains("filesystem root")
    );
}

#[cfg(unix)]
#[test]
fn root_and_directory_symlinks_are_never_followed() {
    use std::os::unix::fs::symlink;

    let temp = tempdir().unwrap();
    let root = temp.path().join("project");
    let outside = temp.path().join("outside");
    write(&outside.join(".DS_Store"), b"outside");
    fs::create_dir_all(&root).unwrap();
    symlink(&outside, root.join("linked-directory")).unwrap();
    symlink(&root, temp.path().join("root-link")).unwrap();

    assert!(scan_folder(&temp.path().join("root-link")).is_err());
    assert!(scan_folder(&root).unwrap().is_empty());
    assert!(outside.join(".DS_Store").is_file());
}

#[cfg(unix)]
#[test]
fn metadata_symlinks_are_removed_without_touching_targets() {
    use std::os::unix::fs::symlink;

    let temp = tempdir().unwrap();
    let root = temp.path().join("project");
    let outside_file = temp.path().join("outside-file");
    let outside_dir = temp.path().join("outside-dir");
    write(&outside_file, b"keep");
    write(&outside_dir.join("keep.txt"), b"keep");
    fs::create_dir_all(&root).unwrap();
    symlink(&outside_file, root.join("._external")).unwrap();
    symlink(&outside_dir, root.join("__MACOSX")).unwrap();
    symlink(temp.path().join("missing-target"), root.join(".DS_Store")).unwrap();

    let scan = scan_folder(&root).unwrap();
    assert_eq!(scan.candidates().len(), 3);
    assert!(
        scan.candidates()
            .iter()
            .all(|candidate| candidate.candidate_type() == CandidateType::Symlink)
    );
    apply_folder_cleanup(&scan).unwrap();

    assert!(fs::symlink_metadata(root.join("._external")).is_err());
    assert!(fs::symlink_metadata(root.join("__MACOSX")).is_err());
    assert!(fs::symlink_metadata(root.join(".DS_Store")).is_err());
    assert_eq!(fs::read(&outside_file).unwrap(), b"keep");
    assert_eq!(fs::read(outside_dir.join("keep.txt")).unwrap(), b"keep");
}

#[cfg(unix)]
#[test]
fn apply_rejects_a_parent_replaced_with_a_symlink() {
    use std::os::unix::fs::symlink;

    let temp = tempdir().unwrap();
    let root = temp.path().join("project");
    let nested = root.join("nested");
    let outside = temp.path().join("outside");
    write(&nested.join("._item"), b"original");
    write(&outside.join("._item"), b"outside");
    let scan = scan_folder(&root).unwrap();

    fs::rename(&nested, root.join("original-nested")).unwrap();
    symlink(&outside, &nested).unwrap();

    let error = apply_folder_cleanup(&scan).unwrap_err();
    assert!(error.to_string().contains("candidate parent changed"));
    assert!(error.removed().is_empty());
    assert_eq!(fs::read(outside.join("._item")).unwrap(), b"outside");
}

#[cfg(unix)]
#[test]
fn scan_error_does_not_delete_candidates_already_seen() {
    use std::os::unix::fs::PermissionsExt;

    let temp = tempdir().unwrap();
    let root = temp.path().join("project");
    let blocked = root.join("blocked");
    write(&root.join(".DS_Store"), b"keep on scan failure");
    write(&blocked.join("file.txt"), b"blocked");
    fs::set_permissions(&blocked, fs::Permissions::from_mode(0o000)).unwrap();

    let result = scan_folder(&root);
    fs::set_permissions(&blocked, fs::Permissions::from_mode(0o700)).unwrap();

    if result.is_err() {
        assert!(root.join(".DS_Store").is_file());
    }
}

#[cfg(unix)]
#[test]
fn partial_apply_error_reports_items_already_removed() {
    use std::os::unix::fs::PermissionsExt;

    let temp = tempdir().unwrap();
    let root = temp.path().join("project");
    let locked = root.join("locked");
    write(&root.join(".DS_Store"), b"first");
    write(&locked.join("._second"), b"second");
    let scan = scan_folder(&root).unwrap();
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o500)).unwrap();

    let result = apply_folder_cleanup(&scan);
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o700)).unwrap();

    if let Err(error) = result {
        assert_eq!(error.removed(), [PathBuf::from(".DS_Store")]);
        assert!(!root.join(".DS_Store").exists());
        assert!(locked.join("._second").exists());
    }
}
