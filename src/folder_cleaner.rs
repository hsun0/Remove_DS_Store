//! Read-only folder scanning and explicitly requested in-place deletion.

use std::fmt;
use std::fs::{self, FileType};
use std::io;
use std::path::{Path, PathBuf};

use crate::metadata::{MetadataKind, classify_filesystem_name};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CandidateType {
    RegularFile,
    Symlink,
    Directory,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FolderCandidate {
    path: PathBuf,
    relative_path: PathBuf,
    metadata_kind: MetadataKind,
    candidate_type: CandidateType,
}

impl FolderCandidate {
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    #[must_use]
    pub fn relative_path(&self) -> &Path {
        &self.relative_path
    }

    #[must_use]
    pub const fn metadata_kind(&self) -> MetadataKind {
        self.metadata_kind
    }

    #[must_use]
    pub const fn candidate_type(&self) -> CandidateType {
        self.candidate_type
    }

    #[must_use]
    pub const fn display_as_directory(&self) -> bool {
        matches!(self.candidate_type, CandidateType::Directory)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FolderScan {
    root: PathBuf,
    candidates: Vec<FolderCandidate>,
}

impl FolderScan {
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    #[must_use]
    pub fn candidates(&self) -> &[FolderCandidate] {
        &self.candidates
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.candidates.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FolderError(String);

impl FolderError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for FolderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for FolderError {}

#[derive(Debug)]
pub struct ApplyError {
    message: String,
    removed: Vec<PathBuf>,
}

impl ApplyError {
    fn new(message: impl Into<String>, removed: Vec<PathBuf>) -> Self {
        Self {
            message: message.into(),
            removed,
        }
    }

    #[must_use]
    pub fn removed(&self) -> &[PathBuf] {
        &self.removed
    }
}

impl fmt::Display for ApplyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ApplyError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplyReport {
    pub removed: Vec<PathBuf>,
}

/// Validates and recursively scans a folder without modifying the filesystem.
pub fn scan_folder(target: &Path) -> Result<FolderScan, FolderError> {
    let root_metadata = fs::symlink_metadata(target).map_err(|error| {
        if error.kind() == io::ErrorKind::NotFound {
            FolderError::new(format!("directory not found:\n{}", target.display()))
        } else {
            FolderError::new(format!(
                "cannot inspect target directory {}: {error}",
                target.display()
            ))
        }
    })?;
    let root_type = root_metadata.file_type();
    if root_type.is_symlink() {
        return Err(FolderError::new(format!(
            "refusing to clean a symbolic-link root:\n{}",
            target.display()
        )));
    }
    if !root_type.is_dir() {
        return Err(FolderError::new(format!(
            "target is not a directory:\n{}",
            target.display()
        )));
    }

    let root = fs::canonicalize(target).map_err(|error| {
        FolderError::new(format!(
            "cannot resolve target directory {}: {error}",
            target.display()
        ))
    })?;
    if root.parent().is_none() {
        return Err(FolderError::new(format!(
            "refusing to scan a filesystem root:\n{}",
            root.display()
        )));
    }

    let mut directories = vec![root.clone()];
    let mut candidates = Vec::new();

    while let Some(directory) = directories.pop() {
        validate_directory_within_root(&directory, &root)?;
        let entries = fs::read_dir(&directory).map_err(|error| {
            FolderError::new(format!(
                "cannot read directory {}: {error}",
                directory.display()
            ))
        })?;

        for entry in entries {
            let entry = entry.map_err(|error| {
                FolderError::new(format!(
                    "cannot read an entry in {}: {error}",
                    directory.display()
                ))
            })?;
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path).map_err(|error| {
                FolderError::new(format!(
                    "cannot inspect filesystem entry {}: {error}",
                    path.display()
                ))
            })?;
            let file_type = metadata.file_type();
            let metadata_kind = classify_filesystem_name(&entry.file_name());

            if file_type.is_symlink() {
                if let Some(kind) = metadata_kind {
                    candidates.push(candidate(&root, path, kind, CandidateType::Symlink)?);
                }
                continue;
            }

            if file_type.is_dir() {
                if metadata_kind == Some(MetadataKind::MacosxDirectory) {
                    candidates.push(candidate(
                        &root,
                        path,
                        MetadataKind::MacosxDirectory,
                        CandidateType::Directory,
                    )?);
                } else {
                    directories.push(path);
                }
                continue;
            }

            if file_type.is_file()
                && matches!(
                    metadata_kind,
                    Some(MetadataKind::DsStore | MetadataKind::AppleDouble)
                )
            {
                let kind = metadata_kind.ok_or_else(|| {
                    FolderError::new("internal error: missing metadata classification")
                })?;
                candidates.push(candidate(&root, path, kind, CandidateType::RegularFile)?);
            }
        }
    }

    candidates.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    Ok(FolderScan { root, candidates })
}

/// Revalidates a completed scan and removes its candidates in place.
pub fn apply_folder_cleanup(scan: &FolderScan) -> Result<ApplyReport, ApplyError> {
    revalidate_root(scan).map_err(|error| ApplyError::new(error.to_string(), Vec::new()))?;

    // A full preflight prevents an already-stale candidate near the end of the
    // plan from causing avoidable partial deletion.
    for candidate in &scan.candidates {
        revalidate_candidate(scan, candidate)
            .map_err(|error| ApplyError::new(error.to_string(), Vec::new()))?;
    }

    let mut removed = Vec::new();
    for candidate in &scan.candidates {
        revalidate_candidate(scan, candidate)
            .map_err(|error| ApplyError::new(error.to_string(), removed.clone()))?;
        remove_candidate(candidate).map_err(|error| {
            ApplyError::new(
                format!(
                    "failed to remove metadata:\n{}\n\nReason:\n{error}",
                    candidate.relative_path.display()
                ),
                removed.clone(),
            )
        })?;
        removed.push(candidate.relative_path.clone());
    }

    Ok(ApplyReport { removed })
}

fn candidate(
    root: &Path,
    path: PathBuf,
    metadata_kind: MetadataKind,
    candidate_type: CandidateType,
) -> Result<FolderCandidate, FolderError> {
    let relative_path = path
        .strip_prefix(root)
        .map_err(|_| {
            FolderError::new(format!(
                "filesystem entry escaped the target directory: {}",
                path.display()
            ))
        })?
        .to_path_buf();
    if relative_path.as_os_str().is_empty() {
        return Err(FolderError::new("refusing to select the target root"));
    }

    Ok(FolderCandidate {
        path,
        relative_path,
        metadata_kind,
        candidate_type,
    })
}

fn validate_directory_within_root(directory: &Path, root: &Path) -> Result<(), FolderError> {
    let metadata = fs::symlink_metadata(directory).map_err(|error| {
        FolderError::new(format!(
            "cannot revalidate directory {}: {error}",
            directory.display()
        ))
    })?;
    if metadata.file_type().is_symlink() || !metadata.file_type().is_dir() {
        return Err(FolderError::new(format!(
            "directory changed while scanning:\n{}",
            directory.display()
        )));
    }
    let canonical = fs::canonicalize(directory).map_err(|error| {
        FolderError::new(format!(
            "cannot resolve directory {}: {error}",
            directory.display()
        ))
    })?;
    if !canonical.starts_with(root) {
        return Err(FolderError::new(format!(
            "directory escaped the target root:\n{}",
            directory.display()
        )));
    }
    Ok(())
}

fn revalidate_root(scan: &FolderScan) -> Result<(), FolderError> {
    let metadata = fs::symlink_metadata(&scan.root).map_err(|error| {
        FolderError::new(format!(
            "cannot revalidate target root {}: {error}",
            scan.root.display()
        ))
    })?;
    if metadata.file_type().is_symlink() || !metadata.file_type().is_dir() {
        return Err(FolderError::new(format!(
            "target root changed after scanning:\n{}",
            scan.root.display()
        )));
    }
    let canonical = fs::canonicalize(&scan.root).map_err(|error| {
        FolderError::new(format!(
            "cannot resolve target root {}: {error}",
            scan.root.display()
        ))
    })?;
    if canonical != scan.root {
        return Err(FolderError::new(format!(
            "target root changed after scanning:\n{}",
            scan.root.display()
        )));
    }
    Ok(())
}

fn revalidate_candidate(scan: &FolderScan, candidate: &FolderCandidate) -> Result<(), FolderError> {
    if !candidate.path.starts_with(&scan.root)
        || candidate.path.strip_prefix(&scan.root).ok() != Some(candidate.relative_path.as_path())
    {
        return Err(FolderError::new(format!(
            "candidate escaped the target root:\n{}",
            candidate.path.display()
        )));
    }
    revalidate_candidate_parents(scan, candidate)?;

    let metadata = fs::symlink_metadata(&candidate.path).map_err(|error| {
        FolderError::new(format!(
            "metadata candidate changed or disappeared:\n{}\n{error}",
            candidate.relative_path.display()
        ))
    })?;
    let actual = classify_type(metadata.file_type());
    if actual != Some(candidate.candidate_type) {
        return Err(FolderError::new(format!(
            "metadata candidate changed type after scanning:\n{}",
            candidate.relative_path.display()
        )));
    }
    if classify_filesystem_name(
        candidate
            .path
            .file_name()
            .ok_or_else(|| FolderError::new("candidate has no filename"))?,
    ) != Some(candidate.metadata_kind)
    {
        return Err(FolderError::new(format!(
            "metadata candidate changed name after scanning:\n{}",
            candidate.relative_path.display()
        )));
    }
    Ok(())
}

fn revalidate_candidate_parents(
    scan: &FolderScan,
    candidate: &FolderCandidate,
) -> Result<(), FolderError> {
    let Some(relative_parent) = candidate.relative_path.parent() else {
        return Ok(());
    };
    let mut parent = scan.root.clone();
    for component in relative_parent.components() {
        parent.push(component);
        let metadata = fs::symlink_metadata(&parent).map_err(|error| {
            FolderError::new(format!(
                "cannot revalidate candidate parent {}: {error}",
                parent.display()
            ))
        })?;
        if metadata.file_type().is_symlink() || !metadata.file_type().is_dir() {
            return Err(FolderError::new(format!(
                "candidate parent changed after scanning:\n{}",
                parent.display()
            )));
        }
    }

    let canonical_parent = fs::canonicalize(&parent).map_err(|error| {
        FolderError::new(format!(
            "cannot resolve candidate parent {}: {error}",
            parent.display()
        ))
    })?;
    if !canonical_parent.starts_with(&scan.root) {
        return Err(FolderError::new(format!(
            "candidate parent escaped the target root:\n{}",
            parent.display()
        )));
    }
    Ok(())
}

fn classify_type(file_type: FileType) -> Option<CandidateType> {
    if file_type.is_symlink() {
        Some(CandidateType::Symlink)
    } else if file_type.is_file() {
        Some(CandidateType::RegularFile)
    } else if file_type.is_dir() {
        Some(CandidateType::Directory)
    } else {
        None
    }
}

fn remove_candidate(candidate: &FolderCandidate) -> io::Result<()> {
    match candidate.candidate_type {
        CandidateType::RegularFile => fs::remove_file(&candidate.path),
        CandidateType::Directory => fs::remove_dir_all(&candidate.path),
        CandidateType::Symlink => remove_symlink(&candidate.path),
    }
}

#[cfg(not(windows))]
fn remove_symlink(path: &Path) -> io::Result<()> {
    fs::remove_file(path)
}

#[cfg(windows)]
fn remove_symlink(path: &Path) -> io::Result<()> {
    use std::os::windows::fs::FileTypeExt;

    let file_type = fs::symlink_metadata(path)?.file_type();
    if file_type.is_symlink_dir() {
        fs::remove_dir(path)
    } else {
        fs::remove_file(path)
    }
}
