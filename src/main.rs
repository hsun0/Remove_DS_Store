#![forbid(unsafe_code)]

use std::env;
use std::ffi::{OsStr, OsString};
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use rmds::folder_cleaner::{FolderCandidate, FolderScan, apply_folder_cleanup, scan_folder};
use rmds::repo_cleaner::{RepoCandidate, apply_repo_cleanup, scan_repo};
use rmds::zip_cleaner::{clean_zip, default_output_path, scan_zip};

const HELP: &str =
    "rmds — Remove unwanted macOS metadata from ZIP archives, folders, and Git repositories.

Usage:
  rmds zip <INPUT.zip> [-o <OUTPUT.zip>]
  rmds zip --check <INPUT.zip>
  rmds folder [PATH]
  rmds folder --check [PATH]
  rmds folder --apply <PATH>
  rmds repo [PATH]
  rmds repo --check [PATH]
  rmds --help
  rmds --version

Commands:
  zip       Check a ZIP, or create a cleaned copy
  folder    Check or preview a folder, or explicitly apply deletion
  repo      Check a Git working tree, or ask before in-place deletion

Options:
  -h, --help  Print help
  --version   Print version
";

const REPO_HELP: &str = "Safely find and remove macOS metadata from a Git working tree.

Usage:
  rmds repo [PATH]
  rmds repo --check [PATH]

PATH defaults to the current directory. A path inside a repository resolves to
the full working-tree root. Candidates are displayed before deletion, and an
interactive terminal must enter exactly DELETE to continue. There is no
--apply mode.

Check mode is read-only and non-interactive. It exits 0 when clean, 1 when
metadata is found, and 2 when the check cannot be completed.

rmds never traverses or modifies .git, nested repositories, or submodules. It
does not edit .gitignore, stage changes, commit, push, or rewrite history.
Untracked, ignored, and uncommitted content might not be recoverable, and
filesystem deletion cannot provide automatic rollback.
";

const ZIP_HELP: &str = "Clean macOS metadata from a ZIP archive without modifying the original.

Usage:
  rmds zip <INPUT.zip> [-o <OUTPUT.zip>]
  rmds zip --check <INPUT.zip>

Options:
  --check                    Validate and check without creating an output
  -o, --output <OUTPUT.zip>  Choose the destination path
  -h, --help                 Print help

The default destination is <INPUT>-clean.zip. Existing files are never overwritten.
Check mode is read-only and creates no output or temporary file. It exits 0
when clean, 1 when metadata is found, and 2 when validation cannot complete.
";

const FOLDER_HELP: &str = "Recursively find macOS metadata in a folder.

Usage:
  rmds folder [PATH]
  rmds folder --check [PATH]
  rmds folder --apply <PATH>

Modes:
  rmds folder [PATH]          Preview only; defaults to the current folder
  rmds folder --check [PATH]  Read-only CI check; defaults to the current folder
  rmds folder --apply <PATH>  Delete in place after an interactive confirmation

Check mode exits 0 when clean, 1 when metadata is found, and 2 when the check
cannot be completed. It never requires an interactive terminal.

Apply mode requires an explicit path and an interactive terminal. It displays a
fresh deletion plan and proceeds only when you enter exactly DELETE. Folder
scanning is recursive and never follows symbolic links. Folder deletion cannot
be undone, and a failure partway through is not rolled back.
";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FolderMode {
    Preview,
    Check,
    Apply,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RepoMode {
    Clean,
    Check,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ZipMode {
    Clean { output: PathBuf },
    Check,
}

enum CliAction {
    Help(&'static str),
    Version,
    Zip { mode: ZipMode, input: PathBuf },
    Folder { mode: FolderMode, path: PathBuf },
    Repo { mode: RepoMode, path: PathBuf },
}

#[derive(Clone, Copy)]
enum Color {
    Red,
    Green,
    BoldCyan,
}

impl Color {
    const fn ansi(self) -> &'static str {
        match self {
            Self::Red => "\x1b[31m",
            Self::Green => "\x1b[32m",
            Self::BoldCyan => "\x1b[1;36m",
        }
    }
}

fn no_color_is_set() -> bool {
    env::var_os("NO_COLOR").is_some()
}

fn colors_enabled(is_terminal: bool, no_color_is_set: bool) -> bool {
    is_terminal && !no_color_is_set
}

fn stdout_colors() -> bool {
    colors_enabled(io::stdout().is_terminal(), no_color_is_set())
}

fn stderr_colors() -> bool {
    colors_enabled(io::stderr().is_terminal(), no_color_is_set())
}

fn styled(text: &str, color: Color, enabled: bool) -> String {
    if enabled {
        format!("{}{text}\x1b[0m", color.ansi())
    } else {
        text.to_owned()
    }
}

fn stdout_styled(text: &str, color: Color) -> String {
    styled(text, color, stdout_colors())
}

fn stderr_styled(text: &str, color: Color) -> String {
    styled(text, color, stderr_colors())
}

fn stdout_path(path: &Path) -> String {
    stdout_styled(&path.display().to_string(), Color::BoldCyan)
}

fn error_label() -> String {
    stderr_styled("Error", Color::Red)
}

fn main() -> ExitCode {
    match parse_args(env::args_os().skip(1)) {
        Ok(CliAction::Help(help)) => {
            print!("{help}");
            ExitCode::SUCCESS
        }
        Ok(CliAction::Version) => {
            println!("rmds {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        Ok(CliAction::Zip { mode, input }) => match mode {
            ZipMode::Clean { output } => run_zip(input, output),
            ZipMode::Check => run_zip_check(&input),
        },
        Ok(CliAction::Folder { mode, path }) => run_folder(mode, &path),
        Ok(CliAction::Repo { mode, path }) => run_repo(mode, &path),
        Err(message) => {
            eprintln!(
                "{}: {message}\n\nRun {} for usage.",
                error_label(),
                stderr_styled("'rmds --help'", Color::BoldCyan)
            );
            ExitCode::from(2)
        }
    }
}

fn run_repo(mode: RepoMode, path: &Path) -> ExitCode {
    let scan = match scan_repo(path) {
        Ok(scan) => scan,
        Err(error) => {
            eprintln!("{}: {error}", error_label());
            if mode == RepoMode::Check {
                return ExitCode::from(2);
            }
            eprintln!("\nNo files were removed.");
            return ExitCode::FAILURE;
        }
    };

    println!("Repository:\n  {}\n", stdout_path(scan.root()));
    if mode == RepoMode::Check {
        if !scan.is_empty() {
            print_repo_candidates("Found macOS metadata:", scan.candidates());
            println!();
        }
        return check_result(scan.candidates().len());
    }

    if scan.is_empty() {
        println!(
            "{}\n\nNo files were removed.\n",
            stdout_styled("No macOS metadata found.", Color::Green)
        );
        print_gitignore_suggestion();
        return ExitCode::SUCCESS;
    }

    print_repo_candidates("Found macOS metadata:", scan.candidates());
    println!(
        "\n{} metadata {} may be removed.",
        scan.candidates().len(),
        entry_word(scan.candidates().len())
    );
    for warning in [
        "WARNING: This is an in-place operation.",
        "Tracked deletions will appear in the Git working tree.",
        "Untracked and ignored files may not be recoverable through Git.",
        "Uncommitted file contents may not be recoverable.",
        "rmds will not modify the Git index, commits, history, or .git.",
        "This operation cannot provide automatic rollback.",
    ] {
        println!("{}", stdout_styled(warning, Color::Red));
    }
    println!();

    let stdin = io::stdin();
    if !stdin.is_terminal() {
        eprintln!(
            "{}: repository deletion requires an interactive terminal.\n\nNo files were removed.",
            error_label()
        );
        return ExitCode::FAILURE;
    }

    print!(
        "Type {} exactly to continue: ",
        stdout_styled("DELETE", Color::BoldCyan)
    );
    if let Err(error) = io::stdout().flush() {
        eprintln!(
            "\n{}: cannot display confirmation prompt: {error}\n\nNo files were removed.",
            error_label()
        );
        return ExitCode::FAILURE;
    }

    let mut confirmation = String::new();
    if let Err(error) = stdin.read_line(&mut confirmation) {
        eprintln!(
            "\n{}: cannot read confirmation: {error}\n\nNo files were removed.",
            error_label()
        );
        return ExitCode::FAILURE;
    }
    if !is_delete_confirmation(&confirmation) {
        println!("\nCleanup cancelled.\n\nNo files were removed.\n");
        print_gitignore_suggestion();
        return ExitCode::SUCCESS;
    }

    match apply_repo_cleanup(&scan) {
        Ok(report) => {
            println!("\n{}", stdout_styled("Removed:", Color::Green));
            print_repo_candidate_path_lines(scan.candidates());
            println!(
                "\n{}",
                stdout_styled(
                    &format!(
                        "Removed {} metadata {}.",
                        report.removed.len(),
                        entry_word(report.removed.len())
                    ),
                    Color::Green
                )
            );
            println!("\nGit metadata, index, and history were not modified.");
            let review_command = format!("git -C {} status --short", scan.root().display());
            println!(
                "Review the working tree with:\n  {}\n",
                stdout_styled(&review_command, Color::BoldCyan)
            );
            print_gitignore_suggestion();
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("\n{}: {error}", error_label());
            if error.removed().is_empty() {
                eprintln!("\nNo files were removed.");
            } else {
                eprintln!("\nAlready removed before the failure:");
                for path in error.removed() {
                    eprintln!(
                        "  {}",
                        stderr_styled(&path.display().to_string(), Color::BoldCyan)
                    );
                }
                eprintln!(
                    "\nAlready removed {} metadata {} before the failure.",
                    error.removed().len(),
                    entry_word(error.removed().len())
                );
                eprintln!("The operation could not be rolled back.");
            }
            ExitCode::FAILURE
        }
    }
}

fn print_repo_candidates(heading: &str, candidates: &[RepoCandidate]) {
    println!("{heading}");
    for candidate in candidates {
        let suffix = if candidate.display_as_directory() {
            "/"
        } else {
            ""
        };
        println!(
            "  [{:<17}] {}{suffix}",
            candidate.git_status().label(),
            stdout_path(candidate.relative_path())
        );
    }
}

fn print_repo_candidate_path_lines(candidates: &[RepoCandidate]) {
    for candidate in candidates {
        if candidate.display_as_directory() {
            println!("  {}/", stdout_path(candidate.relative_path()));
        } else {
            println!("  {}", stdout_path(candidate.relative_path()));
        }
    }
}

fn print_gitignore_suggestion() {
    println!(
        "Suggested .gitignore entries:\n  {}\n  {}\n  {}\n\nrmds did not modify .gitignore.",
        stdout_styled(".DS_Store", Color::BoldCyan),
        stdout_styled("._*", Color::BoldCyan),
        stdout_styled("__MACOSX/", Color::BoldCyan)
    );
}

fn run_zip_check(input: &Path) -> ExitCode {
    let scan = match scan_zip(input) {
        Ok(scan) => scan,
        Err(error) => {
            eprintln!("{}: {error}", error_label());
            return ExitCode::from(2);
        }
    };

    println!("ZIP archive:\n  {}\n", stdout_path(scan.input()));
    if !scan.is_empty() {
        println!("Found macOS metadata:");
        for entry in scan.candidates() {
            println!("  {}", stdout_styled(entry, Color::BoldCyan));
        }
        println!();
    }

    check_result(scan.candidates().len())
}

fn run_zip(input: PathBuf, output: PathBuf) -> ExitCode {
    println!("Cleaning {}...\n", stdout_path(&input));

    match clean_zip(&input, &output) {
        Ok(report) => {
            if report.removed_entries.is_empty() {
                println!(
                    "{}\n",
                    stdout_styled("No macOS metadata found.", Color::Green)
                );
            } else {
                println!("{}", stdout_styled("Removed:", Color::Green));
                for entry in &report.removed_entries {
                    println!("  {}", stdout_styled(entry, Color::BoldCyan));
                }
                println!();
            }

            println!(
                "{}\n  {}\n",
                stdout_styled("Created:", Color::Green),
                stdout_path(&report.output)
            );
            if !report.removed_entries.is_empty() {
                println!(
                    "{}",
                    stdout_styled(
                        &format!(
                            "Removed {} metadata {}.",
                            report.removed_entries.len(),
                            entry_word(report.removed_entries.len())
                        ),
                        Color::Green
                    )
                );
            }
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("{}: {error}", error_label());
            ExitCode::FAILURE
        }
    }
}

fn run_folder(mode: FolderMode, path: &Path) -> ExitCode {
    let scan = match scan_folder(path) {
        Ok(scan) => scan,
        Err(error) => {
            eprintln!("{}: {error}", error_label());
            if mode == FolderMode::Check {
                return ExitCode::from(2);
            }
            eprintln!("\nNo files were removed.");
            return ExitCode::FAILURE;
        }
    };

    println!("Scanning folder:\n  {}\n", stdout_path(scan.root()));
    if mode == FolderMode::Check {
        if !scan.is_empty() {
            print_candidate_paths("Found macOS metadata:", scan.candidates());
            println!();
        }
        return check_result(scan.candidates().len());
    }

    if scan.is_empty() {
        println!(
            "{}\n\nNo files were removed.",
            stdout_styled("No macOS metadata found.", Color::Green)
        );
        return ExitCode::SUCCESS;
    }

    match mode {
        FolderMode::Preview => run_folder_preview(&scan, path),
        FolderMode::Apply => run_folder_apply(&scan),
        FolderMode::Check => unreachable!("check mode returns before deletion dispatch"),
    }
}

fn run_folder_preview(scan: &FolderScan, original_path: &Path) -> ExitCode {
    print_candidate_paths("Found macOS metadata:", scan.candidates());
    let command = format!("rmds folder --apply {}", original_path.display());
    println!(
        "\nNo files were removed.\n\nTo remove {} metadata {}, run:\n  {}",
        scan.candidates().len(),
        entry_word(scan.candidates().len()),
        stdout_styled(&command, Color::BoldCyan)
    );
    ExitCode::SUCCESS
}

fn run_folder_apply(scan: &FolderScan) -> ExitCode {
    print_candidate_paths(
        "The following macOS metadata will be permanently removed:",
        scan.candidates(),
    );
    println!(
        "\n{} metadata {} will be removed.",
        scan.candidates().len(),
        entry_word(scan.candidates().len())
    );
    for warning in [
        "WARNING: This is an in-place operation.",
        "The listed files and directories will be deleted from the target folder.",
        "This operation cannot be undone.",
        "A failure partway through will not roll back items already removed.",
    ] {
        println!("{}", stdout_styled(warning, Color::Red));
    }
    println!();

    let stdin = io::stdin();
    if !stdin.is_terminal() {
        eprintln!(
            "{}: apply mode requires an interactive terminal. Piped or redirected input is refused.\n\nNo files were removed.",
            error_label()
        );
        return ExitCode::FAILURE;
    }

    print!(
        "Type {} exactly to continue: ",
        stdout_styled("DELETE", Color::BoldCyan)
    );
    if let Err(error) = io::stdout().flush() {
        eprintln!(
            "\n{}: cannot display confirmation prompt: {error}\n\nNo files were removed.",
            error_label()
        );
        return ExitCode::FAILURE;
    }

    let mut confirmation = String::new();
    if let Err(error) = stdin.read_line(&mut confirmation) {
        eprintln!(
            "\n{}: cannot read confirmation: {error}\n\nNo files were removed.",
            error_label()
        );
        return ExitCode::FAILURE;
    }
    if !is_delete_confirmation(&confirmation) {
        println!("\nCleanup cancelled.\n\nNo files were removed.");
        return ExitCode::SUCCESS;
    }

    match apply_folder_cleanup(scan) {
        Ok(report) => {
            println!("\n{}", stdout_styled("Removed:", Color::Green));
            print_candidate_path_lines(scan.candidates());
            println!(
                "\n{}",
                stdout_styled(
                    &format!(
                        "Removed {} metadata {}.",
                        report.removed.len(),
                        entry_word(report.removed.len())
                    ),
                    Color::Green
                )
            );
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("\n{}: {error}", error_label());
            if error.removed().is_empty() {
                eprintln!("\nNo files were removed.");
            } else {
                eprintln!("\nAlready removed before the failure:");
                for path in error.removed() {
                    eprintln!(
                        "  {}",
                        stderr_styled(&path.display().to_string(), Color::BoldCyan)
                    );
                }
                eprintln!(
                    "\nAlready removed {} metadata {} before the failure.",
                    error.removed().len(),
                    entry_word(error.removed().len())
                );
                eprintln!("The operation could not be rolled back.");
            }
            ExitCode::FAILURE
        }
    }
}

fn print_candidate_paths(heading: &str, candidates: &[FolderCandidate]) {
    println!("{heading}");
    print_candidate_path_lines(candidates);
}

fn print_candidate_path_lines(candidates: &[FolderCandidate]) {
    for candidate in candidates {
        if candidate.display_as_directory() {
            println!("  {}/", stdout_path(candidate.relative_path()));
        } else {
            println!("  {}", stdout_path(candidate.relative_path()));
        }
    }
}

fn entry_word(count: usize) -> &'static str {
    if count == 1 { "entry" } else { "entries" }
}

fn check_result(candidate_count: usize) -> ExitCode {
    if candidate_count == 0 {
        println!(
            "{}",
            stdout_styled("Check passed: no macOS metadata found.", Color::Green)
        );
        ExitCode::SUCCESS
    } else {
        println!(
            "{}",
            stdout_styled(
                &format!(
                    "Check failed: found {candidate_count} metadata {}.",
                    entry_word(candidate_count)
                ),
                Color::Red
            )
        );
        ExitCode::FAILURE
    }
}

fn is_delete_confirmation(input: &str) -> bool {
    let without_newline = input
        .strip_suffix("\r\n")
        .or_else(|| input.strip_suffix('\n'))
        .unwrap_or(input);
    without_newline == "DELETE"
}

fn parse_args(args: impl IntoIterator<Item = OsString>) -> Result<CliAction, String> {
    let mut args = args.into_iter();
    let Some(command) = args.next() else {
        return Ok(CliAction::Help(HELP));
    };

    if is(&command, "-h") || is(&command, "--help") {
        if args.next().is_some() {
            return Err("unexpected argument after --help".to_owned());
        }
        return Ok(CliAction::Help(HELP));
    }
    if is(&command, "--version") {
        if args.next().is_some() {
            return Err("unexpected argument after --version".to_owned());
        }
        return Ok(CliAction::Version);
    }

    if is(&command, "zip") {
        parse_zip_args(args)
    } else if is(&command, "folder") {
        parse_folder_args(args)
    } else if is(&command, "repo") {
        parse_repo_args(args)
    } else {
        Err(format!("unknown command: {}", command.to_string_lossy()))
    }
}

fn parse_repo_args(mut args: impl Iterator<Item = OsString>) -> Result<CliAction, String> {
    let Some(first) = args.next() else {
        return Ok(CliAction::Repo {
            mode: RepoMode::Clean,
            path: PathBuf::from("."),
        });
    };

    if is(&first, "-h") || is(&first, "--help") {
        if args.next().is_some() {
            return Err(
                "unexpected argument after repo --help\n\nUsage:\n  rmds repo [PATH]".to_owned(),
            );
        }
        return Ok(CliAction::Help(REPO_HELP));
    }
    if is(&first, "--check") {
        let path = optional_check_path(&mut args, "repo")?;
        return Ok(CliAction::Repo {
            mode: RepoMode::Check,
            path,
        });
    }
    if first.as_encoded_bytes().starts_with(b"-") {
        return Err(format!(
            "unknown repo option: {}\n\nUsage:\n  rmds repo [PATH]",
            first.to_string_lossy()
        ));
    }
    if let Some(extra) = args.next() {
        return Err(format!(
            "unexpected argument: {}\n\nUsage:\n  rmds repo [PATH]",
            extra.to_string_lossy()
        ));
    }

    Ok(CliAction::Repo {
        mode: RepoMode::Clean,
        path: PathBuf::from(first),
    })
}

fn parse_zip_args(mut args: impl Iterator<Item = OsString>) -> Result<CliAction, String> {
    let Some(input) = args.next() else {
        return Err("missing ZIP input path".to_owned());
    };
    if is(&input, "-h") || is(&input, "--help") {
        if args.next().is_some() {
            return Err("unexpected argument after zip --help".to_owned());
        }
        return Ok(CliAction::Help(ZIP_HELP));
    }
    if is(&input, "--check") {
        let Some(input) = args.next() else {
            return Err("check mode requires a ZIP input: rmds zip --check <INPUT.zip>".to_owned());
        };
        if input.as_encoded_bytes().starts_with(b"-") {
            return Err(
                "expected a ZIP input after --check\n\nUsage:\n  rmds zip --check <INPUT.zip>"
                    .to_owned(),
            );
        }
        if let Some(extra) = args.next() {
            return Err(format!(
                "unexpected argument after check input: {}\n\nUsage:\n  rmds zip --check <INPUT.zip>",
                extra.to_string_lossy()
            ));
        }
        return Ok(CliAction::Zip {
            mode: ZipMode::Check,
            input: PathBuf::from(input),
        });
    }
    if is(&input, "-o") || is(&input, "--output") {
        return Err("missing ZIP input path before --output".to_owned());
    }

    let input = PathBuf::from(input);
    let mut output = None;
    while let Some(argument) = args.next() {
        if is(&argument, "-o") || is(&argument, "--output") {
            if output.is_some() {
                return Err("output option may only be specified once".to_owned());
            }
            let Some(value) = args.next() else {
                return Err("missing path after --output".to_owned());
            };
            output = Some(PathBuf::from(value));
        } else {
            return Err(format!(
                "unexpected argument: {}",
                argument.to_string_lossy()
            ));
        }
    }

    let output = output.unwrap_or_else(|| default_output_path(&input));
    Ok(CliAction::Zip {
        mode: ZipMode::Clean { output },
        input,
    })
}

fn parse_folder_args(mut args: impl Iterator<Item = OsString>) -> Result<CliAction, String> {
    let Some(first) = args.next() else {
        return Ok(CliAction::Folder {
            mode: FolderMode::Preview,
            path: PathBuf::from("."),
        });
    };

    if is(&first, "-h") || is(&first, "--help") {
        if args.next().is_some() {
            return Err("unexpected argument after folder --help".to_owned());
        }
        return Ok(CliAction::Help(FOLDER_HELP));
    }

    if is(&first, "--apply") {
        let Some(path) = args.next() else {
            return Err(
                "apply mode requires an explicit path: rmds folder --apply <PATH>".to_owned(),
            );
        };
        if path.as_encoded_bytes().starts_with(b"-") {
            return Err("expected a path after --apply".to_owned());
        }
        if let Some(extra) = args.next() {
            return Err(format!(
                "unexpected argument after apply path: {}",
                extra.to_string_lossy()
            ));
        }
        return Ok(CliAction::Folder {
            mode: FolderMode::Apply,
            path: PathBuf::from(path),
        });
    }

    if is(&first, "--check") {
        let path = optional_check_path(&mut args, "folder")?;
        return Ok(CliAction::Folder {
            mode: FolderMode::Check,
            path,
        });
    }

    if first.as_encoded_bytes().starts_with(b"-") {
        return Err(format!(
            "unknown folder option: {}",
            first.to_string_lossy()
        ));
    }
    if args.next().is_some() {
        return Err("unexpected argument; apply syntax is: rmds folder --apply <PATH>".to_owned());
    }

    Ok(CliAction::Folder {
        mode: FolderMode::Preview,
        path: PathBuf::from(first),
    })
}

fn optional_check_path(
    args: &mut impl Iterator<Item = OsString>,
    command: &str,
) -> Result<PathBuf, String> {
    let Some(path) = args.next() else {
        return Ok(PathBuf::from("."));
    };
    if path.as_encoded_bytes().starts_with(b"-") {
        return Err(format!(
            "expected a path after --check\n\nUsage:\n  rmds {command} --check [PATH]"
        ));
    }
    if let Some(extra) = args.next() {
        return Err(format!(
            "unexpected argument after check path: {}\n\nUsage:\n  rmds {command} --check [PATH]",
            extra.to_string_lossy()
        ));
    }
    Ok(PathBuf::from(path))
}

fn is(value: &OsStr, expected: &str) -> bool {
    value == OsStr::new(expected)
}

#[cfg(test)]
mod tests {
    use super::{
        CliAction, Color, FolderMode, RepoMode, ZipMode, colors_enabled, is_delete_confirmation,
        parse_args, styled,
    };
    use std::ffi::OsString;
    use std::path::Path;

    fn args(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
    }

    #[test]
    fn parses_zip_default_and_custom_outputs() {
        let CliAction::Zip { input, mode } = parse_args(args(&["zip", "a.zip"])).unwrap() else {
            panic!("expected zip action");
        };
        assert_eq!(input, Path::new("a.zip"));
        assert_eq!(
            mode,
            ZipMode::Clean {
                output: Path::new("a-clean.zip").to_path_buf()
            }
        );

        let CliAction::Zip { mode, .. } =
            parse_args(args(&["zip", "a.zip", "--output", "b.zip"])).unwrap()
        else {
            panic!("expected zip action");
        };
        assert_eq!(
            mode,
            ZipMode::Clean {
                output: Path::new("b.zip").to_path_buf()
            }
        );

        let CliAction::Zip { mode, input } =
            parse_args(args(&["zip", "--check", "a.zip"])).unwrap()
        else {
            panic!("expected ZIP check action");
        };
        assert_eq!(mode, ZipMode::Check);
        assert_eq!(input, Path::new("a.zip"));
    }

    #[test]
    fn parses_folder_preview_default_and_path() {
        let CliAction::Folder { mode, path } = parse_args(args(&["folder"])).unwrap() else {
            panic!("expected folder action");
        };
        assert_eq!(mode, FolderMode::Preview);
        assert_eq!(path, Path::new("."));

        let CliAction::Folder { mode, path } = parse_args(args(&["folder", "photos"])).unwrap()
        else {
            panic!("expected folder action");
        };
        assert_eq!(mode, FolderMode::Preview);
        assert_eq!(path, Path::new("photos"));
    }

    #[test]
    fn parses_folder_apply_only_in_canonical_order() {
        let CliAction::Folder { mode, path } =
            parse_args(args(&["folder", "--apply", "photos"])).unwrap()
        else {
            panic!("expected folder action");
        };
        assert_eq!(mode, FolderMode::Apply);
        assert_eq!(path, Path::new("photos"));

        assert!(parse_args(args(&["folder", "--apply"])).is_err());
        assert!(parse_args(args(&["folder", "photos", "--apply"])).is_err());
        assert!(parse_args(args(&["folder", "--apply", "photos", "extra"])).is_err());
    }

    #[test]
    fn parses_folder_check_default_and_path_in_canonical_order() {
        let CliAction::Folder { mode, path } = parse_args(args(&["folder", "--check"])).unwrap()
        else {
            panic!("expected folder action");
        };
        assert_eq!(mode, FolderMode::Check);
        assert_eq!(path, Path::new("."));

        let CliAction::Folder { mode, path } =
            parse_args(args(&["folder", "--check", "photos"])).unwrap()
        else {
            panic!("expected folder action");
        };
        assert_eq!(mode, FolderMode::Check);
        assert_eq!(path, Path::new("photos"));

        assert!(parse_args(args(&["folder", "photos", "--check"])).is_err());
        assert!(parse_args(args(&["folder", "--check", "--apply", "."])).is_err());
        assert!(parse_args(args(&["folder", "--check", "one", "two"])).is_err());
    }

    #[test]
    fn parses_version_without_extra_arguments() {
        assert!(matches!(
            parse_args(args(&["--version"])),
            Ok(CliAction::Version)
        ));
        assert!(parse_args(args(&["--version", "extra"])).is_err());
    }

    #[test]
    fn parses_repo_default_and_path_without_apply_mode() {
        let CliAction::Repo { mode, path } = parse_args(args(&["repo"])).unwrap() else {
            panic!("expected repo action");
        };
        assert_eq!(mode, RepoMode::Clean);
        assert_eq!(path, Path::new("."));

        let CliAction::Repo { mode, path } = parse_args(args(&["repo", "project"])).unwrap() else {
            panic!("expected repo action");
        };
        assert_eq!(mode, RepoMode::Clean);
        assert_eq!(path, Path::new("project"));

        let CliAction::Repo { mode, path } = parse_args(args(&["repo", "--check"])).unwrap() else {
            panic!("expected repo check action");
        };
        assert_eq!(mode, RepoMode::Check);
        assert_eq!(path, Path::new("."));

        let CliAction::Repo { mode, path } =
            parse_args(args(&["repo", "--check", "project"])).unwrap()
        else {
            panic!("expected repo check action");
        };
        assert_eq!(mode, RepoMode::Check);
        assert_eq!(path, Path::new("project"));

        assert!(parse_args(args(&["repo", "--apply"])).is_err());
        assert!(parse_args(args(&["repo", "--apply", "project"])).is_err());
        assert!(parse_args(args(&["repo", "project", "--apply"])).is_err());
        assert!(parse_args(args(&["repo", "one", "two"])).is_err());
        assert!(parse_args(args(&["repo", "project", "--check"])).is_err());
        assert!(parse_args(args(&["repo", "--check", "--apply", "."])).is_err());
        assert!(parse_args(args(&["repo", "--check", "one", "two"])).is_err());
    }

    #[test]
    fn confirmation_is_exact_except_for_line_ending() {
        assert!(is_delete_confirmation("DELETE"));
        assert!(is_delete_confirmation("DELETE\n"));
        assert!(is_delete_confirmation("DELETE\r\n"));
        for rejected in [
            "delete\n",
            " DELETE\n",
            "DELETE \n",
            "DELETE\r",
            "DELETE\n\n",
        ] {
            assert!(!is_delete_confirmation(rejected), "{rejected:?}");
        }
    }

    #[test]
    fn rejects_invalid_arguments() {
        assert!(parse_args(args(&["zip"])).is_err());
        assert!(parse_args(args(&["zip", "a.zip", "--output"])).is_err());
        assert!(parse_args(args(&["zip", "a.zip", "extra"])).is_err());
        assert!(parse_args(args(&["zip", "--check"])).is_err());
        assert!(parse_args(args(&["zip", "a.zip", "--check"])).is_err());
        assert!(parse_args(args(&["zip", "--check", "a.zip", "-o", "b.zip"])).is_err());
        assert!(parse_args(args(&["folder", "--force", "photos"])).is_err());
    }

    #[test]
    fn color_requires_a_terminal_and_respects_no_color() {
        assert!(colors_enabled(true, false));
        assert!(!colors_enabled(false, false));
        assert!(!colors_enabled(true, true));

        assert_eq!(styled("warning", Color::Red, false), "warning");
        assert_eq!(
            styled("warning", Color::Red, true),
            "\x1b[31mwarning\x1b[0m"
        );
    }
}
