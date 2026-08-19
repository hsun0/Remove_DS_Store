#![forbid(unsafe_code)]

use std::env;
use std::ffi::{OsStr, OsString};
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use rmds::folder_cleaner::{FolderCandidate, FolderScan, apply_folder_cleanup, scan_folder};
use rmds::zip_cleaner::{clean_zip, default_output_path};

const HELP: &str = "rmds — Remove unwanted macOS metadata from ZIP archives and folders.

Usage:
  rmds zip <INPUT.zip> [-o <OUTPUT.zip>]
  rmds folder [PATH]
  rmds folder --apply <PATH>
  rmds --help

Commands:
  zip       Create a cleaned copy of a ZIP archive
  folder    Preview folder metadata, or explicitly apply in-place deletion
";

const ZIP_HELP: &str = "Clean macOS metadata from a ZIP archive without modifying the original.

Usage:
  rmds zip <INPUT.zip> [-o <OUTPUT.zip>]

Options:
  -o, --output <OUTPUT.zip>  Choose the destination path
  -h, --help                 Print help

The default destination is <INPUT>-clean.zip. Existing files are never overwritten.
";

const FOLDER_HELP: &str = "Recursively find macOS metadata in a folder.

Usage:
  rmds folder [PATH]
  rmds folder --apply <PATH>

Modes:
  rmds folder [PATH]          Preview only; defaults to the current folder
  rmds folder --apply <PATH>  Delete in place after an interactive confirmation

Apply mode requires an explicit path and an interactive terminal. It displays a
fresh deletion plan and proceeds only when you enter exactly DELETE. Folder
scanning is recursive and never follows symbolic links. Folder deletion cannot
be undone, and a failure partway through is not rolled back.
";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FolderMode {
    Preview,
    Apply,
}

enum CliAction {
    Help(&'static str),
    Zip { input: PathBuf, output: PathBuf },
    Folder { mode: FolderMode, path: PathBuf },
}

fn main() -> ExitCode {
    match parse_args(env::args_os().skip(1)) {
        Ok(CliAction::Help(help)) => {
            print!("{help}");
            ExitCode::SUCCESS
        }
        Ok(CliAction::Zip { input, output }) => run_zip(input, output),
        Ok(CliAction::Folder { mode, path }) => run_folder(mode, &path),
        Err(message) => {
            eprintln!("Error: {message}\n\nRun 'rmds --help' for usage.");
            ExitCode::from(2)
        }
    }
}

fn run_zip(input: PathBuf, output: PathBuf) -> ExitCode {
    println!("Cleaning {}...\n", input.display());

    match clean_zip(&input, &output) {
        Ok(report) => {
            if report.removed_entries.is_empty() {
                println!("No macOS metadata found.\n");
            } else {
                println!("Removed:");
                for entry in &report.removed_entries {
                    println!("  {entry}");
                }
                println!();
            }

            println!("Created:\n  {}\n", report.output.display());
            if !report.removed_entries.is_empty() {
                println!(
                    "Removed {} metadata {}.",
                    report.removed_entries.len(),
                    entry_word(report.removed_entries.len())
                );
            }
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("Error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run_folder(mode: FolderMode, path: &Path) -> ExitCode {
    let scan = match scan_folder(path) {
        Ok(scan) => scan,
        Err(error) => {
            eprintln!("Error: {error}\n\nNo files were removed.");
            return ExitCode::FAILURE;
        }
    };

    println!("Scanning folder:\n  {}\n", scan.root().display());
    if scan.is_empty() {
        println!("No macOS metadata found.\n\nNo files were removed.");
        return ExitCode::SUCCESS;
    }

    match mode {
        FolderMode::Preview => run_folder_preview(&scan, path),
        FolderMode::Apply => run_folder_apply(&scan),
    }
}

fn run_folder_preview(scan: &FolderScan, original_path: &Path) -> ExitCode {
    print_candidate_paths("Found macOS metadata:", scan.candidates());
    println!(
        "\nNo files were removed.\n\nTo remove {} metadata {}, run:\n  rmds folder --apply {}",
        scan.candidates().len(),
        entry_word(scan.candidates().len()),
        original_path.display()
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
    println!("\nWARNING: This is an in-place operation.");
    println!("The listed files and directories will be deleted from the target folder.");
    println!("This operation cannot be undone.");
    println!("A failure partway through will not roll back items already removed.\n");

    let stdin = io::stdin();
    if !stdin.is_terminal() {
        eprintln!(
            "Error: apply mode requires an interactive terminal. Piped or redirected input is refused.\n\nNo files were removed."
        );
        return ExitCode::FAILURE;
    }

    print!("Type DELETE exactly to continue: ");
    if let Err(error) = io::stdout().flush() {
        eprintln!("\nError: cannot display confirmation prompt: {error}\n\nNo files were removed.");
        return ExitCode::FAILURE;
    }

    let mut confirmation = String::new();
    if let Err(error) = stdin.read_line(&mut confirmation) {
        eprintln!("\nError: cannot read confirmation: {error}\n\nNo files were removed.");
        return ExitCode::FAILURE;
    }
    if !is_delete_confirmation(&confirmation) {
        println!("\nCleanup cancelled.\n\nNo files were removed.");
        return ExitCode::SUCCESS;
    }

    match apply_folder_cleanup(scan) {
        Ok(report) => {
            println!("\nRemoved:");
            print_candidate_path_lines(scan.candidates());
            println!(
                "\nRemoved {} metadata {}.",
                report.removed.len(),
                entry_word(report.removed.len())
            );
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("\nError: {error}");
            if error.removed().is_empty() {
                eprintln!("\nNo files were removed.");
            } else {
                eprintln!("\nAlready removed before the failure:");
                for path in error.removed() {
                    eprintln!("  {}", path.display());
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
            println!("  {}/", candidate.relative_path().display());
        } else {
            println!("  {}", candidate.relative_path().display());
        }
    }
}

fn entry_word(count: usize) -> &'static str {
    if count == 1 { "entry" } else { "entries" }
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

    if is(&command, "zip") {
        parse_zip_args(args)
    } else if is(&command, "folder") {
        parse_folder_args(args)
    } else {
        Err(format!("unknown command: {}", command.to_string_lossy()))
    }
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
    Ok(CliAction::Zip { input, output })
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

fn is(value: &OsStr, expected: &str) -> bool {
    value == OsStr::new(expected)
}

#[cfg(test)]
mod tests {
    use super::{CliAction, FolderMode, is_delete_confirmation, parse_args};
    use std::ffi::OsString;
    use std::path::Path;

    fn args(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
    }

    #[test]
    fn parses_zip_default_and_custom_outputs() {
        let CliAction::Zip { input, output } = parse_args(args(&["zip", "a.zip"])).unwrap() else {
            panic!("expected zip action");
        };
        assert_eq!(input, Path::new("a.zip"));
        assert_eq!(output, Path::new("a-clean.zip"));

        let CliAction::Zip { output, .. } =
            parse_args(args(&["zip", "a.zip", "--output", "b.zip"])).unwrap()
        else {
            panic!("expected zip action");
        };
        assert_eq!(output, Path::new("b.zip"));
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
        assert!(parse_args(args(&["repo"])).is_err());
        assert!(parse_args(args(&["zip"])).is_err());
        assert!(parse_args(args(&["zip", "a.zip", "--output"])).is_err());
        assert!(parse_args(args(&["zip", "a.zip", "extra"])).is_err());
        assert!(parse_args(args(&["folder", "--force", "photos"])).is_err());
    }
}
