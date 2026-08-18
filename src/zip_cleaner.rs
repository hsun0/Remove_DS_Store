//! Safe ZIP-to-ZIP cleaning.

use std::fmt;
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use tempfile::Builder;
use zip::{ZipArchive, ZipWriter};

use crate::metadata::is_macos_metadata;

const VERIFY_BUFFER_SIZE: usize = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CleanReport {
    pub output: PathBuf,
    pub removed_entries: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CleanError(String);

impl CleanError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for CleanError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for CleanError {}

pub type Result<T> = std::result::Result<T, CleanError>;

/// Derives `name-clean.zip` in the same directory as `input`.
#[must_use]
pub fn default_output_path(input: &Path) -> PathBuf {
    let stem = input
        .file_stem()
        .or_else(|| input.file_name())
        .unwrap_or_default();
    let mut filename = stem.to_os_string();
    filename.push("-clean.zip");
    input.with_file_name(filename)
}

/// Creates a cleaned ZIP without modifying `input` or overwriting `output`.
pub fn clean_zip(input: &Path, output: &Path) -> Result<CleanReport> {
    validate_input_output(input, output)?;

    let input_file = File::open(input).map_err(|error| {
        if error.kind() == io::ErrorKind::NotFound {
            CleanError::new(format!("file not found:\n{}", input.display()))
        } else {
            CleanError::new(format!("cannot read input {}: {error}", input.display()))
        }
    })?;

    let mut archive = ZipArchive::new(input_file)
        .map_err(|error| CleanError::new(format!("invalid ZIP {}: {error}", input.display())))?;

    if archive.has_overlapping_files().map_err(|error| {
        CleanError::new(format!(
            "cannot validate ZIP structure {}: {error}",
            input.display()
        ))
    })? {
        return Err(CleanError::new(
            "unsafe ZIP structure: archive entries overlap",
        ));
    }

    let archive_comment = archive.comment().to_vec();
    let parent = output_parent(output);
    let mut temporary = Builder::new()
        .prefix(".rmds-")
        .suffix(".tmp")
        .tempfile_in(parent)
        .map_err(|error| {
            CleanError::new(format!(
                "cannot create output in {}: {error}",
                parent.display()
            ))
        })?;

    let removed_entries = {
        let mut writer = ZipWriter::new(temporary.as_file_mut());
        writer
            .set_raw_comment(archive_comment.into())
            .map_err(|error| CleanError::new(format!("cannot preserve ZIP comment: {error}")))?;
        let removed = copy_verified_entries(&mut archive, &mut writer)?;
        writer
            .finish()
            .map_err(|error| CleanError::new(format!("cannot finish output ZIP: {error}")))?;
        removed
    };

    temporary
        .as_file()
        .sync_all()
        .map_err(|error| CleanError::new(format!("cannot flush output ZIP: {error}")))?;

    temporary.persist_noclobber(output).map_err(|error| {
        if error.error.kind() == io::ErrorKind::AlreadyExists {
            CleanError::new(format!("output file already exists:\n{}", output.display()))
        } else {
            CleanError::new(format!(
                "cannot create output {}: {}",
                output.display(),
                error.error
            ))
        }
    })?;

    Ok(CleanReport {
        output: output.to_path_buf(),
        removed_entries,
    })
}

fn copy_verified_entries(
    archive: &mut ZipArchive<File>,
    writer: &mut ZipWriter<&mut File>,
) -> Result<Vec<String>> {
    let mut removed = Vec::new();
    let mut buffer = [0_u8; VERIFY_BUFFER_SIZE];

    for index in 0..archive.len() {
        let (raw_name, display_name, encrypted, symlink) = {
            let entry = archive.by_index_raw(index).map_err(|error| {
                CleanError::new(format!("cannot inspect ZIP entry #{index}: {error}"))
            })?;
            (
                entry.name_raw().to_vec(),
                entry.name().to_owned(),
                entry.encrypted(),
                entry.is_symlink(),
            )
        };

        validate_entry_name(&raw_name).map_err(|reason| {
            CleanError::new(format!("unsafe ZIP entry {display_name:?}: {reason}"))
        })?;

        if encrypted {
            return Err(CleanError::new(format!(
                "encrypted ZIP entry is not supported: {display_name}"
            )));
        }

        verify_entry(archive, index, &display_name, &mut buffer)?;

        if is_macos_metadata(&raw_name) {
            removed.push(display_name);
            continue;
        }

        if symlink {
            copy_symlink(archive, writer, index, &raw_name, &display_name)?;
        } else {
            let entry = archive.by_index_raw(index).map_err(|error| {
                CleanError::new(format!("cannot reopen ZIP entry {display_name:?}: {error}"))
            })?;
            writer.raw_copy_file(entry).map_err(|error| {
                CleanError::new(format!("cannot copy ZIP entry {display_name:?}: {error}"))
            })?;
        }
    }

    Ok(removed)
}

fn copy_symlink(
    archive: &mut ZipArchive<File>,
    writer: &mut ZipWriter<&mut File>,
    index: usize,
    raw_name: &[u8],
    display_name: &str,
) -> Result<()> {
    if raw_name != display_name.as_bytes() {
        return Err(CleanError::new(format!(
            "cannot safely preserve non-UTF-8 symlink name: {display_name:?}"
        )));
    }

    let entry = archive.by_index(index).map_err(|error| {
        CleanError::new(format!(
            "cannot read symlink entry {display_name:?}: {error}"
        ))
    })?;
    if entry.size() > VERIFY_BUFFER_SIZE as u64 {
        return Err(CleanError::new(format!(
            "symlink target is unreasonably large: {display_name:?}"
        )));
    }
    let options = entry.options();
    let mut target = Vec::with_capacity(entry.size() as usize);
    entry
        .take(VERIFY_BUFFER_SIZE as u64 + 1)
        .read_to_end(&mut target)
        .map_err(|error| {
            CleanError::new(format!(
                "cannot read symlink target {display_name:?}: {error}"
            ))
        })?;
    if target.len() > VERIFY_BUFFER_SIZE {
        return Err(CleanError::new(format!(
            "symlink target is unreasonably large: {display_name:?}"
        )));
    }
    let target = String::from_utf8(target).map_err(|_| {
        CleanError::new(format!(
            "cannot safely preserve non-UTF-8 symlink target: {display_name:?}"
        ))
    })?;

    writer
        .add_symlink(display_name, target, options)
        .map_err(|error| {
            CleanError::new(format!(
                "cannot preserve symlink entry {display_name:?}: {error}"
            ))
        })
}

fn verify_entry(
    archive: &mut ZipArchive<File>,
    index: usize,
    display_name: &str,
    buffer: &mut [u8],
) -> Result<()> {
    let mut entry = archive.by_index(index).map_err(|error| {
        CleanError::new(format!("cannot read ZIP entry {display_name:?}: {error}"))
    })?;

    loop {
        match entry.read(buffer) {
            Ok(0) => return Ok(()),
            Ok(_) => {}
            Err(error) => {
                return Err(CleanError::new(format!(
                    "corrupt ZIP entry {display_name:?}: {error}"
                )));
            }
        }
    }
}

fn validate_input_output(input: &Path, output: &Path) -> Result<()> {
    if input == output {
        return Err(CleanError::new("output path must differ from input path"));
    }

    let metadata = fs::metadata(input).map_err(|error| {
        if error.kind() == io::ErrorKind::NotFound {
            CleanError::new(format!("file not found:\n{}", input.display()))
        } else {
            CleanError::new(format!("cannot inspect input {}: {error}", input.display()))
        }
    })?;

    if !metadata.is_file() {
        return Err(CleanError::new(format!(
            "input is not a regular file:\n{}",
            input.display()
        )));
    }

    if output.exists() {
        return Err(CleanError::new(format!(
            "output file already exists:\n{}",
            output.display()
        )));
    }

    let parent = output_parent(output);
    let parent_metadata = fs::metadata(parent).map_err(|error| {
        CleanError::new(format!(
            "output directory does not exist or is inaccessible:\n{}\n{error}",
            parent.display()
        ))
    })?;
    if !parent_metadata.is_dir() {
        return Err(CleanError::new(format!(
            "output parent is not a directory:\n{}",
            parent.display()
        )));
    }

    Ok(())
}

fn output_parent(output: &Path) -> &Path {
    output
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
}

fn validate_entry_name(name: &[u8]) -> std::result::Result<(), &'static str> {
    if name.is_empty() {
        return Err("empty entry name");
    }
    if name.contains(&0) {
        return Err("entry name contains a NUL byte");
    }
    if name.starts_with(b"/") || name.starts_with(b"\\") {
        return Err("absolute entry path");
    }
    if name.contains(&b'\\') {
        return Err("entry name uses a host-specific backslash separator");
    }

    let components: Vec<&[u8]> = name.split(|byte| *byte == b'/').collect();
    for (index, component) in components.iter().enumerate() {
        let is_trailing_directory_marker = index + 1 == components.len() && component.is_empty();
        if is_trailing_directory_marker {
            continue;
        }
        if component.is_empty() {
            return Err("entry path contains an empty component");
        }
        if *component == b"." || *component == b".." {
            return Err("entry path contains a traversal component");
        }
        if index == 0
            && component.len() >= 2
            && component[0].is_ascii_alphabetic()
            && component[1] == b':'
        {
            return Err("entry path contains a Windows drive prefix");
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{default_output_path, validate_entry_name};
    use std::path::Path;

    #[test]
    fn derives_default_output_name() {
        assert_eq!(
            default_output_path(Path::new("somewhere/archive.zip")),
            Path::new("somewhere/archive-clean.zip")
        );
        assert_eq!(
            default_output_path(Path::new("archive")),
            Path::new("archive-clean.zip")
        );
    }

    #[test]
    fn rejects_suspicious_archive_paths() {
        for name in [
            &b"../file"[..],
            &b"foo/../../file"[..],
            &b"/absolute"[..],
            &b"C:\\file"[..],
            &b"foo\\bar"[..],
            &b"foo//bar"[..],
            &b"foo\0bar"[..],
        ] {
            assert!(validate_entry_name(name).is_err(), "{name:?}");
        }
    }

    #[test]
    fn accepts_zip_paths_and_unicode_bytes() {
        for name in [
            &b"foo/bar.txt"[..],
            &b"empty/"[..],
            "照片/台北.jpg".as_bytes(),
        ] {
            assert_eq!(validate_entry_name(name), Ok(()), "{name:?}");
        }
    }
}
