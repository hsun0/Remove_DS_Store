//! Git-aware working-tree scanning without modifying Git state.

use std::collections::BTreeSet;
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::fs::{self, FileType};
use std::io;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

use crate::folder_cleaner::CandidateType;
use crate::metadata::{MetadataKind, classify_filesystem_name};

const GIT_ENTRY: &str = ".git";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepoGitStatus {
    Tracked,
    TrackedModified,
    Untracked,
    Ignored,
    Mixed,
}

impl RepoGitStatus {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Tracked => "tracked",
            Self::TrackedModified => "tracked, modified",
            Self::Untracked => "untracked",
            Self::Ignored => "ignored",
            Self::Mixed => "mixed",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoCandidate {
    path: PathBuf,
    relative_path: PathBuf,
    metadata_kind: MetadataKind,
    candidate_type: CandidateType,
    git_status: RepoGitStatus,
}

impl RepoCandidate {
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
    pub const fn git_status(&self) -> RepoGitStatus {
        self.git_status
    }

    #[must_use]
    pub const fn display_as_directory(&self) -> bool {
        matches!(self.candidate_type, CandidateType::Directory)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoScan {
    root: PathBuf,
    git_dir: PathBuf,
    git_common_dir: PathBuf,
    candidates: Vec<RepoCandidate>,
}

impl RepoScan {
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    #[must_use]
    pub fn git_dir(&self) -> &Path {
        &self.git_dir
    }

    #[must_use]
    pub fn git_common_dir(&self) -> &Path {
        &self.git_common_dir
    }

    #[must_use]
    pub fn candidates(&self) -> &[RepoCandidate] {
        &self.candidates
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.candidates.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoError(String);

impl RepoError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for RepoError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for RepoError {}

#[derive(Debug)]
pub struct RepoApplyError {
    message: String,
    removed: Vec<PathBuf>,
}

impl RepoApplyError {
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

impl fmt::Display for RepoApplyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for RepoApplyError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoApplyReport {
    pub removed: Vec<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RepositoryLayout {
    root: PathBuf,
    git_dir: PathBuf,
    git_common_dir: PathBuf,
}

#[derive(Debug, Default)]
struct GitPathState {
    tracked: BTreeSet<Vec<u8>>,
    modified: BTreeSet<Vec<u8>>,
    untracked: BTreeSet<Vec<u8>>,
    ignored: BTreeSet<Vec<u8>>,
}

#[derive(Debug)]
struct PendingCandidate {
    path: PathBuf,
    relative_path: PathBuf,
    metadata_kind: MetadataKind,
    candidate_type: CandidateType,
}

/// Resolves the repository, scans its working tree, and reads Git state without mutation.
pub fn scan_repo(target: &Path) -> Result<RepoScan, RepoError> {
    let layout = discover_repository(target)?;
    let pending = scan_working_tree(&layout)?;
    let git_state = load_git_path_state(&layout.root)?;

    let mut candidates = Vec::with_capacity(pending.len());
    for candidate in pending {
        let key = git_path_key(&candidate.relative_path)?;
        let git_status = classify_git_status(&git_state, &key, candidate.candidate_type);
        candidates.push(RepoCandidate {
            path: candidate.path,
            relative_path: candidate.relative_path,
            metadata_kind: candidate.metadata_kind,
            candidate_type: candidate.candidate_type,
            git_status,
        });
    }
    candidates.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));

    Ok(RepoScan {
        root: layout.root,
        git_dir: layout.git_dir,
        git_common_dir: layout.git_common_dir,
        candidates,
    })
}

/// Revalidates a repository scan and removes only its working-tree candidates.
pub fn apply_repo_cleanup(scan: &RepoScan) -> Result<RepoApplyReport, RepoApplyError> {
    revalidate_repository(scan)
        .map_err(|error| RepoApplyError::new(error.to_string(), Vec::new()))?;

    for candidate in &scan.candidates {
        revalidate_candidate(scan, candidate)
            .map_err(|error| RepoApplyError::new(error.to_string(), Vec::new()))?;
    }

    let mut removed = Vec::new();
    for candidate in &scan.candidates {
        revalidate_candidate(scan, candidate)
            .map_err(|error| RepoApplyError::new(error.to_string(), removed.clone()))?;
        remove_candidate(candidate).map_err(|error| {
            RepoApplyError::new(
                format!(
                    "failed to remove repository metadata:\n{}\n\nReason:\n{error}",
                    candidate.relative_path.display()
                ),
                removed.clone(),
            )
        })?;
        removed.push(candidate.relative_path.clone());
    }

    Ok(RepoApplyReport { removed })
}

fn discover_repository(target: &Path) -> Result<RepositoryLayout, RepoError> {
    let metadata = fs::symlink_metadata(target).map_err(|error| {
        if error.kind() == io::ErrorKind::NotFound {
            RepoError::new(format!("directory not found:\n{}", target.display()))
        } else {
            RepoError::new(format!(
                "cannot inspect repository path {}: {error}",
                target.display()
            ))
        }
    })?;
    if metadata.file_type().is_symlink() {
        return Err(RepoError::new(format!(
            "refusing to clean a symbolic-link repository path:\n{}",
            target.display()
        )));
    }
    if !metadata.file_type().is_dir() {
        return Err(RepoError::new(format!(
            "repository path is not a directory:\n{}",
            target.display()
        )));
    }

    let canonical_target = fs::canonicalize(target).map_err(|error| {
        RepoError::new(format!(
            "cannot resolve repository path {}: {error}",
            target.display()
        ))
    })?;
    if canonical_target.parent().is_none() {
        return Err(RepoError::new(format!(
            "refusing to scan a filesystem root:\n{}",
            canonical_target.display()
        )));
    }

    let bare = git_single_line(&canonical_target, &["rev-parse", "--is-bare-repository"])?;
    if bare == b"true" {
        return Err(RepoError::new(format!(
            "bare repositories do not have a working tree:\n{}",
            canonical_target.display()
        )));
    }
    if bare != b"false" {
        return Err(RepoError::new(
            "Git returned an invalid bare-repository status",
        ));
    }

    let inside = git_single_line(&canonical_target, &["rev-parse", "--is-inside-work-tree"])?;
    if inside != b"true" {
        return Err(RepoError::new(format!(
            "path is not inside a Git working tree:\n{}",
            canonical_target.display()
        )));
    }

    let cdup = git_single_line(&canonical_target, &["rev-parse", "--show-cdup"])?;
    let root = repository_root_from_cdup(&canonical_target, &cdup)?;
    validate_repository_root(&root)?;

    let verified_cdup = git_single_line(&root, &["rev-parse", "--show-cdup"])?;
    if !verified_cdup.is_empty() {
        return Err(RepoError::new(format!(
            "Git working-tree root validation failed:\n{}",
            root.display()
        )));
    }

    let git_dir = git_absolute_path(&root, &["rev-parse", "--absolute-git-dir"])?;
    let git_common_dir = git_absolute_path(
        &root,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )?;

    Ok(RepositoryLayout {
        root,
        git_dir,
        git_common_dir,
    })
}

fn repository_root_from_cdup(target: &Path, cdup: &[u8]) -> Result<PathBuf, RepoError> {
    if !cdup.is_empty()
        && cdup
            .split(|byte| *byte == b'/')
            .filter(|component| !component.is_empty())
            .any(|component| component != b"..")
    {
        return Err(RepoError::new(
            "Git returned an invalid working-tree relative path",
        ));
    }
    let relative = os_string_from_git_bytes(cdup)?;
    fs::canonicalize(target.join(relative)).map_err(|error| {
        RepoError::new(format!(
            "cannot resolve Git working-tree root from {}: {error}",
            target.display()
        ))
    })
}

fn validate_repository_root(root: &Path) -> Result<(), RepoError> {
    let metadata = fs::symlink_metadata(root).map_err(|error| {
        RepoError::new(format!(
            "cannot inspect Git working-tree root {}: {error}",
            root.display()
        ))
    })?;
    if metadata.file_type().is_symlink() || !metadata.file_type().is_dir() {
        return Err(RepoError::new(format!(
            "Git working-tree root is not a real directory:\n{}",
            root.display()
        )));
    }
    if root.parent().is_none() {
        return Err(RepoError::new(format!(
            "refusing to scan a filesystem root:\n{}",
            root.display()
        )));
    }
    let canonical = fs::canonicalize(root).map_err(|error| {
        RepoError::new(format!(
            "cannot resolve Git working-tree root {}: {error}",
            root.display()
        ))
    })?;
    if canonical != root {
        return Err(RepoError::new(format!(
            "Git working-tree root did not resolve consistently:\n{}",
            root.display()
        )));
    }
    Ok(())
}

fn git_absolute_path(root: &Path, arguments: &[&str]) -> Result<PathBuf, RepoError> {
    let bytes = git_single_line(root, arguments)?;
    if bytes.is_empty() {
        return Err(RepoError::new(
            "Git returned an empty internal directory path",
        ));
    }
    let path = PathBuf::from(os_string_from_git_bytes(&bytes)?);
    let absolute = if path.is_absolute() {
        path
    } else {
        root.join(path)
    };
    fs::canonicalize(&absolute).map_err(|error| {
        RepoError::new(format!(
            "cannot resolve Git internal directory {}: {error}",
            absolute.display()
        ))
    })
}

fn scan_working_tree(layout: &RepositoryLayout) -> Result<Vec<PendingCandidate>, RepoError> {
    let mut directories = vec![layout.root.clone()];
    let mut candidates = Vec::new();

    while let Some(directory) = directories.pop() {
        validate_directory_within_root(&directory, &layout.root)?;
        if directory != layout.root && has_git_boundary(&directory)? {
            continue;
        }

        let entries = fs::read_dir(&directory).map_err(|error| {
            RepoError::new(format!(
                "cannot read repository directory {}: {error}",
                directory.display()
            ))
        })?;

        for entry in entries {
            let entry = entry.map_err(|error| {
                RepoError::new(format!(
                    "cannot read an entry in {}: {error}",
                    directory.display()
                ))
            })?;
            let file_name = entry.file_name();
            if file_name == OsStr::new(GIT_ENTRY) {
                continue;
            }

            let path = entry.path();
            let metadata = fs::symlink_metadata(&path).map_err(|error| {
                RepoError::new(format!(
                    "cannot inspect repository entry {}: {error}",
                    path.display()
                ))
            })?;
            let file_type = metadata.file_type();
            let metadata_kind = classify_filesystem_name(&file_name);

            if file_type.is_symlink() {
                if let Some(kind) = metadata_kind {
                    candidates.push(pending_candidate(
                        &layout.root,
                        path,
                        kind,
                        CandidateType::Symlink,
                    )?);
                }
                continue;
            }

            if file_type.is_dir() {
                if has_git_boundary(&path)? {
                    continue;
                }
                if metadata_kind == Some(MetadataKind::MacosxDirectory) {
                    if !tree_contains_git_boundary(&path)? {
                        candidates.push(pending_candidate(
                            &layout.root,
                            path,
                            MetadataKind::MacosxDirectory,
                            CandidateType::Directory,
                        )?);
                    }
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
                    RepoError::new("internal error: missing metadata classification")
                })?;
                candidates.push(pending_candidate(
                    &layout.root,
                    path,
                    kind,
                    CandidateType::RegularFile,
                )?);
            }
        }
    }

    Ok(candidates)
}

fn pending_candidate(
    root: &Path,
    path: PathBuf,
    metadata_kind: MetadataKind,
    candidate_type: CandidateType,
) -> Result<PendingCandidate, RepoError> {
    let relative_path = path
        .strip_prefix(root)
        .map_err(|_| {
            RepoError::new(format!(
                "repository entry escaped the working tree: {}",
                path.display()
            ))
        })?
        .to_path_buf();
    validate_relative_candidate_path(&relative_path)?;
    Ok(PendingCandidate {
        path,
        relative_path,
        metadata_kind,
        candidate_type,
    })
}

fn load_git_path_state(root: &Path) -> Result<GitPathState, RepoError> {
    let tracked = git_path_set(root, &["ls-files", "-z", "--cached", "--"])?;
    let mut modified = git_path_set(root, &["diff-files", "--name-only", "-z", "--"])?;
    modified.extend(git_path_set(
        root,
        &["diff", "--cached", "--name-only", "-z", "--"],
    )?);
    let untracked = git_path_set(
        root,
        &["ls-files", "-z", "--others", "--exclude-standard", "--"],
    )?;
    let ignored = git_path_set(
        root,
        &[
            "ls-files",
            "-z",
            "--others",
            "--ignored",
            "--exclude-standard",
            "--",
        ],
    )?;
    Ok(GitPathState {
        tracked,
        modified,
        untracked,
        ignored,
    })
}

fn git_path_set(root: &Path, arguments: &[&str]) -> Result<BTreeSet<Vec<u8>>, RepoError> {
    let output = run_git(root, arguments)?;
    let mut paths = BTreeSet::new();
    for path in output.split(|byte| *byte == 0) {
        if path.is_empty() {
            continue;
        }
        let normalized = path.strip_suffix(b"/").unwrap_or(path);
        validate_git_path_record(normalized)?;
        paths.insert(normalized.to_vec());
    }
    Ok(paths)
}

fn validate_git_path_record(path: &[u8]) -> Result<(), RepoError> {
    if path.is_empty() || path.starts_with(b"/") {
        return Err(RepoError::new("Git returned an unsafe path record"));
    }
    for component in path.split(|byte| *byte == b'/') {
        if component.is_empty() || component == b"." || component == b".." {
            return Err(RepoError::new("Git returned an unsafe path component"));
        }
    }
    Ok(())
}

fn classify_git_status(
    state: &GitPathState,
    key: &[u8],
    candidate_type: CandidateType,
) -> RepoGitStatus {
    if candidate_type != CandidateType::Directory {
        if state.tracked.contains(key) {
            return if state.modified.contains(key) {
                RepoGitStatus::TrackedModified
            } else {
                RepoGitStatus::Tracked
            };
        }
        if state.ignored.contains(key) {
            return RepoGitStatus::Ignored;
        }
        return RepoGitStatus::Untracked;
    }

    let tracked = state
        .tracked
        .iter()
        .any(|path| path_is_within_git_key(path, key));
    let modified = state
        .modified
        .iter()
        .any(|path| path_is_within_git_key(path, key));
    let untracked = state
        .untracked
        .iter()
        .any(|path| path_is_within_git_key(path, key));
    let ignored = state
        .ignored
        .iter()
        .any(|path| path_is_within_git_key(path, key));
    let categories = usize::from(tracked) + usize::from(untracked) + usize::from(ignored);

    if categories > 1 {
        RepoGitStatus::Mixed
    } else if tracked {
        if modified {
            RepoGitStatus::TrackedModified
        } else {
            RepoGitStatus::Tracked
        }
    } else if ignored {
        RepoGitStatus::Ignored
    } else {
        RepoGitStatus::Untracked
    }
}

fn path_is_within_git_key(path: &[u8], directory: &[u8]) -> bool {
    path == directory
        || path
            .strip_prefix(directory)
            .is_some_and(|suffix| suffix.starts_with(b"/"))
}

fn git_path_key(path: &Path) -> Result<Vec<u8>, RepoError> {
    let mut key = Vec::new();
    for component in path.components() {
        let Component::Normal(component) = component else {
            return Err(RepoError::new(format!(
                "repository-relative path contains an unsafe component: {}",
                path.display()
            )));
        };
        if component == OsStr::new(GIT_ENTRY) {
            return Err(RepoError::new(format!(
                "refusing a path inside .git: {}",
                path.display()
            )));
        }
        if !key.is_empty() {
            key.push(b'/');
        }
        key.extend_from_slice(component.as_encoded_bytes());
    }
    if key.is_empty() {
        return Err(RepoError::new("refusing to select the repository root"));
    }
    Ok(key)
}

fn validate_relative_candidate_path(path: &Path) -> Result<(), RepoError> {
    let _ = git_path_key(path)?;
    Ok(())
}

fn validate_directory_within_root(directory: &Path, root: &Path) -> Result<(), RepoError> {
    let metadata = fs::symlink_metadata(directory).map_err(|error| {
        RepoError::new(format!(
            "cannot revalidate repository directory {}: {error}",
            directory.display()
        ))
    })?;
    if metadata.file_type().is_symlink() || !metadata.file_type().is_dir() {
        return Err(RepoError::new(format!(
            "repository directory changed while scanning:\n{}",
            directory.display()
        )));
    }
    let canonical = fs::canonicalize(directory).map_err(|error| {
        RepoError::new(format!(
            "cannot resolve repository directory {}: {error}",
            directory.display()
        ))
    })?;
    if !canonical.starts_with(root) {
        return Err(RepoError::new(format!(
            "repository directory escaped the working tree:\n{}",
            directory.display()
        )));
    }
    Ok(())
}

fn has_git_boundary(directory: &Path) -> Result<bool, RepoError> {
    match fs::symlink_metadata(directory.join(GIT_ENTRY)) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(RepoError::new(format!(
            "cannot inspect repository boundary in {}: {error}",
            directory.display()
        ))),
    }
}

fn tree_contains_git_boundary(root: &Path) -> Result<bool, RepoError> {
    let mut directories = vec![root.to_path_buf()];
    while let Some(directory) = directories.pop() {
        let entries = fs::read_dir(&directory).map_err(|error| {
            RepoError::new(format!(
                "cannot inspect metadata tree {} for repository boundaries: {error}",
                directory.display()
            ))
        })?;
        for entry in entries {
            let entry = entry.map_err(|error| {
                RepoError::new(format!(
                    "cannot inspect an entry in metadata tree {}: {error}",
                    directory.display()
                ))
            })?;
            if entry.file_name() == OsStr::new(GIT_ENTRY) {
                return Ok(true);
            }
            let metadata = fs::symlink_metadata(entry.path()).map_err(|error| {
                RepoError::new(format!(
                    "cannot inspect metadata tree entry {}: {error}",
                    entry.path().display()
                ))
            })?;
            if metadata.file_type().is_dir() && !metadata.file_type().is_symlink() {
                directories.push(entry.path());
            }
        }
    }
    Ok(false)
}

fn revalidate_repository(scan: &RepoScan) -> Result<(), RepoError> {
    let layout = discover_repository(&scan.root)?;
    if layout.root != scan.root
        || layout.git_dir != scan.git_dir
        || layout.git_common_dir != scan.git_common_dir
    {
        return Err(RepoError::new(format!(
            "repository layout changed after scanning:\n{}",
            scan.root.display()
        )));
    }
    Ok(())
}

fn revalidate_candidate(scan: &RepoScan, candidate: &RepoCandidate) -> Result<(), RepoError> {
    validate_relative_candidate_path(&candidate.relative_path)?;
    if !candidate.path.starts_with(&scan.root)
        || candidate.path.strip_prefix(&scan.root).ok() != Some(candidate.relative_path.as_path())
    {
        return Err(RepoError::new(format!(
            "candidate escaped the repository working tree:\n{}",
            candidate.path.display()
        )));
    }
    if paths_intersect(&candidate.path, &scan.git_dir)
        || paths_intersect(&candidate.path, &scan.git_common_dir)
    {
        return Err(RepoError::new(format!(
            "refusing to modify a Git internal path:\n{}",
            candidate.path.display()
        )));
    }
    revalidate_candidate_parents(scan, candidate)?;

    let metadata = fs::symlink_metadata(&candidate.path).map_err(|error| {
        RepoError::new(format!(
            "repository metadata candidate changed or disappeared:\n{}\n{error}",
            candidate.relative_path.display()
        ))
    })?;
    if classify_type(metadata.file_type()) != Some(candidate.candidate_type) {
        return Err(RepoError::new(format!(
            "repository metadata candidate changed type after scanning:\n{}",
            candidate.relative_path.display()
        )));
    }
    if classify_filesystem_name(
        candidate
            .path
            .file_name()
            .ok_or_else(|| RepoError::new("candidate has no filename"))?,
    ) != Some(candidate.metadata_kind)
    {
        return Err(RepoError::new(format!(
            "repository metadata candidate changed name after scanning:\n{}",
            candidate.relative_path.display()
        )));
    }
    if candidate.candidate_type == CandidateType::Directory
        && tree_contains_git_boundary(&candidate.path)?
    {
        return Err(RepoError::new(format!(
            "a nested repository boundary appeared inside the candidate:\n{}",
            candidate.relative_path.display()
        )));
    }
    Ok(())
}

fn revalidate_candidate_parents(
    scan: &RepoScan,
    candidate: &RepoCandidate,
) -> Result<(), RepoError> {
    let Some(relative_parent) = candidate.relative_path.parent() else {
        return Ok(());
    };
    let mut parent = scan.root.clone();
    for component in relative_parent.components() {
        let Component::Normal(component) = component else {
            return Err(RepoError::new(
                "candidate parent contains an unsafe component",
            ));
        };
        if component == OsStr::new(GIT_ENTRY) {
            return Err(RepoError::new("refusing a candidate inside .git"));
        }
        parent.push(component);
        let metadata = fs::symlink_metadata(&parent).map_err(|error| {
            RepoError::new(format!(
                "cannot revalidate candidate parent {}: {error}",
                parent.display()
            ))
        })?;
        if metadata.file_type().is_symlink() || !metadata.file_type().is_dir() {
            return Err(RepoError::new(format!(
                "candidate parent changed after scanning:\n{}",
                parent.display()
            )));
        }
        if has_git_boundary(&parent)? {
            return Err(RepoError::new(format!(
                "a nested repository boundary appeared after scanning:\n{}",
                parent.display()
            )));
        }
    }

    let canonical_parent = fs::canonicalize(&parent).map_err(|error| {
        RepoError::new(format!(
            "cannot resolve candidate parent {}: {error}",
            parent.display()
        ))
    })?;
    if !canonical_parent.starts_with(&scan.root) {
        return Err(RepoError::new(format!(
            "candidate parent escaped the repository working tree:\n{}",
            parent.display()
        )));
    }
    Ok(())
}

fn paths_intersect(left: &Path, right: &Path) -> bool {
    left.starts_with(right) || right.starts_with(left)
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

fn remove_candidate(candidate: &RepoCandidate) -> io::Result<()> {
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

fn git_single_line(root: &Path, arguments: &[&str]) -> Result<Vec<u8>, RepoError> {
    let output = run_git(root, arguments)?;
    let line = output
        .strip_suffix(b"\r\n")
        .or_else(|| output.strip_suffix(b"\n"))
        .unwrap_or(&output);
    if line.contains(&b'\n') || line.contains(&b'\r') || line.contains(&0) {
        return Err(RepoError::new(
            "Git returned an invalid single-line response",
        ));
    }
    Ok(line.to_vec())
}

fn run_git(root: &Path, arguments: &[&str]) -> Result<Vec<u8>, RepoError> {
    let output = Command::new("git")
        .current_dir(root)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .args(arguments)
        .output()
        .map_err(|error| {
            RepoError::new(format!("cannot execute Git in {}: {error}", root.display()))
        })?;
    if !output.status.success() {
        let detail = String::from_utf8_lossy(&output.stderr);
        return Err(RepoError::new(format!(
            "Git command failed in {}:\n{}",
            root.display(),
            detail.trim_end()
        )));
    }
    Ok(output.stdout)
}

#[cfg(unix)]
fn os_string_from_git_bytes(bytes: &[u8]) -> Result<OsString, RepoError> {
    use std::os::unix::ffi::OsStringExt;

    Ok(OsString::from_vec(bytes.to_vec()))
}

#[cfg(windows)]
fn os_string_from_git_bytes(bytes: &[u8]) -> Result<OsString, RepoError> {
    String::from_utf8(bytes.to_vec())
        .map(OsString::from)
        .map_err(|_| RepoError::new("Git returned a non-UTF-8 Windows path"))
}

#[cfg(not(any(unix, windows)))]
fn os_string_from_git_bytes(bytes: &[u8]) -> Result<OsString, RepoError> {
    String::from_utf8(bytes.to_vec())
        .map(OsString::from)
        .map_err(|_| RepoError::new("Git returned an unsupported path encoding"))
}
