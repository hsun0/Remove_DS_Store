#![forbid(unsafe_code)]

use std::env;
use std::ffi::{OsStr, OsString};
use std::path::PathBuf;
use std::process::ExitCode;

use rmds::zip_cleaner::{clean_zip, default_output_path};

const HELP: &str = "rmds — Remove unwanted macOS metadata from ZIP archives.

Usage:
  rmds zip <INPUT.zip> [-o <OUTPUT.zip>]
  rmds --help

Commands:
  zip    Clean .DS_Store, AppleDouble, and __MACOSX entries from a ZIP
";

const ZIP_HELP: &str = "Clean macOS metadata from a ZIP archive without modifying the original.

Usage:
  rmds zip <INPUT.zip> [-o <OUTPUT.zip>]

Options:
  -o, --output <OUTPUT.zip>  Choose the destination path
  -h, --help                 Print help

The default destination is <INPUT>-clean.zip. Existing files are never overwritten.
";

enum CliAction {
    Help(&'static str),
    Zip { input: PathBuf, output: PathBuf },
}

fn main() -> ExitCode {
    match parse_args(env::args_os().skip(1)) {
        Ok(CliAction::Help(help)) => {
            print!("{help}");
            ExitCode::SUCCESS
        }
        Ok(CliAction::Zip { input, output }) => run_zip(input, output),
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
                    if report.removed_entries.len() == 1 {
                        "entry"
                    } else {
                        "entries"
                    }
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
    if !is(&command, "zip") {
        return Err(format!("unknown command: {}", command.to_string_lossy()));
    }

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

fn is(value: &OsStr, expected: &str) -> bool {
    value == OsStr::new(expected)
}

#[cfg(test)]
mod tests {
    use super::{CliAction, parse_args};
    use std::ffi::OsString;
    use std::path::Path;

    fn args(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
    }

    #[test]
    fn parses_default_and_custom_outputs() {
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
    fn rejects_invalid_arguments() {
        assert!(parse_args(args(&["repo"])).is_err());
        assert!(parse_args(args(&["zip"])).is_err());
        assert!(parse_args(args(&["zip", "a.zip", "--output"])).is_err());
        assert!(parse_args(args(&["zip", "a.zip", "extra"])).is_err());
    }
}
