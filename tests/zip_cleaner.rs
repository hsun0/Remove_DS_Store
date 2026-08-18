use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

use rmds::zip_cleaner::clean_zip;
use tempfile::tempdir;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

fn options(method: CompressionMethod, mode: u32) -> SimpleFileOptions {
    SimpleFileOptions::default()
        .compression_method(method)
        .unix_permissions(mode)
}

fn fixture(path: &Path) {
    let file = File::create(path).unwrap();
    let mut writer = ZipWriter::new(file);
    writer
        .add_directory("project/", options(CompressionMethod::Stored, 0o755))
        .unwrap();
    writer
        .start_file(
            "project/README.md",
            options(CompressionMethod::Deflated, 0o644),
        )
        .unwrap();
    writer.write_all(b"hello").unwrap();
    writer
        .start_file(
            "project/bin/tool",
            options(CompressionMethod::Deflated, 0o755),
        )
        .unwrap();
    writer.write_all(b"#!/bin/sh\n").unwrap();
    writer
        .add_directory("project/empty/", options(CompressionMethod::Stored, 0o755))
        .unwrap();
    writer
        .start_file("照片/台北.jpg", options(CompressionMethod::Deflated, 0o644))
        .unwrap();
    writer.write_all(b"photo").unwrap();
    writer
        .start_file(
            "project/.gitignore",
            options(CompressionMethod::Deflated, 0o644),
        )
        .unwrap();
    writer.write_all(b"target\n").unwrap();
    writer
        .add_symlink(
            "project/readme-link",
            "README.md",
            options(CompressionMethod::Stored, 0o777),
        )
        .unwrap();

    for name in [
        ".DS_Store",
        "project/.DS_Store",
        "._root",
        "project/._README.md",
        "__MACOSX/",
        "__MACOSX/project/._README.md",
        "nested/__MACOSX/._item",
    ] {
        if name.ends_with('/') {
            writer
                .add_directory(name, options(CompressionMethod::Stored, 0o755))
                .unwrap();
        } else {
            writer
                .start_file(name, options(CompressionMethod::Stored, 0o644))
                .unwrap();
            writer.write_all(b"metadata").unwrap();
        }
    }

    writer.set_comment("archive comment").unwrap();
    writer.finish().unwrap();
}

fn names(path: &Path) -> Vec<String> {
    let mut archive = ZipArchive::new(File::open(path).unwrap()).unwrap();
    (0..archive.len())
        .map(|index| archive.by_index(index).unwrap().name().to_owned())
        .collect()
}

fn write_single(path: &Path, name: &str, content: &[u8], method: CompressionMethod) {
    let mut writer = ZipWriter::new(File::create(path).unwrap());
    writer.start_file(name, options(method, 0o644)).unwrap();
    writer.write_all(content).unwrap();
    writer.finish().unwrap();
}

#[test]
fn cleans_metadata_and_preserves_normal_entry_properties() {
    let directory = tempdir().unwrap();
    let input = directory.path().join("input.zip");
    let output = directory.path().join("output.zip");
    fixture(&input);
    let original = fs::read(&input).unwrap();

    let report = clean_zip(&input, &output).unwrap();

    assert_eq!(report.removed_entries.len(), 7);
    assert_eq!(fs::read(&input).unwrap(), original);
    assert_eq!(
        names(&output),
        [
            "project/",
            "project/README.md",
            "project/bin/tool",
            "project/empty/",
            "照片/台北.jpg",
            "project/.gitignore",
            "project/readme-link",
        ]
    );

    let mut archive = ZipArchive::new(File::open(&output).unwrap()).unwrap();
    assert_eq!(archive.comment(), b"archive comment");
    let executable = archive.by_name("project/bin/tool").unwrap();
    assert_ne!(executable.unix_mode().unwrap() & 0o111, 0);
    assert_eq!(executable.compression(), CompressionMethod::Deflated);
    drop(executable);
    let mut symlink = archive.by_name("project/readme-link").unwrap();
    assert!(symlink.is_symlink());
    let mut symlink_target = String::new();
    symlink.read_to_string(&mut symlink_target).unwrap();
    assert_eq!(symlink_target, "README.md");
}

#[test]
fn creates_valid_output_when_there_is_no_metadata() {
    let directory = tempdir().unwrap();
    let input = directory.path().join("plain.zip");
    let output = directory.path().join("clean.zip");
    write_single(&input, ".hidden", b"keep", CompressionMethod::Deflated);

    let report = clean_zip(&input, &output).unwrap();
    assert!(report.removed_entries.is_empty());
    assert_eq!(names(&output), [".hidden"]);
}

#[test]
fn creates_empty_archive_when_every_entry_is_metadata() {
    let directory = tempdir().unwrap();
    let input = directory.path().join("metadata.zip");
    let output = directory.path().join("clean.zip");
    write_single(&input, ".DS_Store", b"metadata", CompressionMethod::Stored);

    clean_zip(&input, &output).unwrap();
    assert!(names(&output).is_empty());
}

#[test]
fn streams_a_large_entry() {
    let directory = tempdir().unwrap();
    let input = directory.path().join("large.zip");
    let output = directory.path().join("clean.zip");
    let content = vec![0x5a; 4 * 1024 * 1024];
    write_single(&input, "large.bin", &content, CompressionMethod::Deflated);

    clean_zip(&input, &output).unwrap();
    let mut archive = ZipArchive::new(File::open(output).unwrap()).unwrap();
    let mut entry = archive.by_name("large.bin").unwrap();
    let mut actual = Vec::new();
    entry.read_to_end(&mut actual).unwrap();
    assert_eq!(actual, content);
}

#[test]
fn rejects_missing_input_collision_and_same_path() {
    let directory = tempdir().unwrap();
    let missing = directory.path().join("missing.zip");
    let output = directory.path().join("output.zip");
    assert!(clean_zip(&missing, &output).is_err());
    assert!(!output.exists());

    let input = directory.path().join("input.zip");
    write_single(&input, "file.txt", b"safe", CompressionMethod::Stored);
    assert!(clean_zip(&input, &input).is_err());

    fs::write(&output, b"do not overwrite").unwrap();
    assert!(clean_zip(&input, &output).is_err());
    assert_eq!(fs::read(&output).unwrap(), b"do not overwrite");
}

#[test]
fn rejects_corrupt_zip_and_crc_failure_without_final_output() {
    let directory = tempdir().unwrap();
    let malformed = directory.path().join("malformed.zip");
    let malformed_output = directory.path().join("malformed-clean.zip");
    fs::write(&malformed, b"not a zip").unwrap();
    assert!(clean_zip(&malformed, &malformed_output).is_err());
    assert!(!malformed_output.exists());

    let corrupt = directory.path().join("crc.zip");
    let corrupt_output = directory.path().join("crc-clean.zip");
    let payload = b"unique-payload-for-crc";
    write_single(&corrupt, "file.txt", payload, CompressionMethod::Stored);
    let mut bytes = fs::read(&corrupt).unwrap();
    let offset = bytes
        .windows(payload.len())
        .position(|window| window == payload)
        .unwrap();
    bytes[offset] ^= 0xff;
    fs::write(&corrupt, bytes).unwrap();

    assert!(clean_zip(&corrupt, &corrupt_output).is_err());
    assert!(!corrupt_output.exists());
}

#[test]
fn rejects_suspicious_archive_paths() {
    let directory = tempdir().unwrap();
    let input = directory.path().join("suspicious.zip");
    let output = directory.path().join("clean.zip");
    write_single(&input, "../outside.txt", b"nope", CompressionMethod::Stored);

    assert!(clean_zip(&input, &output).is_err());
    assert!(!output.exists());
    assert!(!directory.path().join("outside.txt").exists());
}

#[test]
fn cli_reports_success_and_missing_input() {
    let directory = tempdir().unwrap();
    let input = directory.path().join("cli.zip");
    let output = directory.path().join("result.zip");
    write_single(&input, ".DS_Store", b"metadata", CompressionMethod::Stored);

    let success = Command::new(env!("CARGO_BIN_EXE_rmds"))
        .args([
            "zip",
            input.to_str().unwrap(),
            "-o",
            output.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(success.status.success());
    assert!(String::from_utf8_lossy(&success.stdout).contains("Removed 1 metadata entry."));
    assert!(output.exists());

    let missing_path: PathBuf = directory.path().join("absent.zip");
    let failure = Command::new(env!("CARGO_BIN_EXE_rmds"))
        .args(["zip", missing_path.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(!failure.status.success());
    assert!(String::from_utf8_lossy(&failure.stderr).contains("file not found"));
}
