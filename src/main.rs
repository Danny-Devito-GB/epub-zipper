//! epub-zipper: turn the current folder (loose EPUB files) into an .epub file.
//!
//! Usage:  epub-zipper [output.epub]
//! By default the output is `<current folder name>.epub`, inside the current folder.

mod container;
mod pack;
mod package;
mod paths;
mod report;
mod validate;
mod xml;

use std::env;
use std::error::Error;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

// The `?` operator used below means: "if this failed, return the error from
// this function right now; otherwise unwrap the successful value".
fn run() -> Result<(), Box<dyn Error>> {
    let folder = env::current_dir()?;
    let output_path = choose_output_path(&folder);

    // If the output file lives inside the folder, tell `validate` so it can
    // ignore our own output when looking for stray files.
    let output_file_in_folder = path_relative_to(&folder, &output_path);
    let validation = validate::validate(&folder, output_file_in_folder.as_deref())?;

    for warning in &validation.report.warnings {
        eprintln!("warning: {warning}");
    }

    if validation.report.has_errors() {
        for error in &validation.report.errors {
            eprintln!("error: {error}");
        }
        return Err(format!(
            "{} problem(s) found - no EPUB was written",
            validation.report.errors.len()
        )
        .into());
    }

    pack::build(&folder, &validation.files_to_pack, &output_path)?;
    println!(
        "Created {} ({} files)",
        output_path.display(),
        validation.files_to_pack.len()
    );
    Ok(())
}

/// Use the path given on the command line, or `<folder name>.epub`.
fn choose_output_path(folder: &Path) -> PathBuf {
    // `args_os` (unlike `args`) doesn't panic if the argument isn't valid Unicode.
    if let Some(argument) = env::args_os().nth(1) {
        let given_path = PathBuf::from(argument);
        return if given_path.is_absolute() {
            given_path
        } else {
            folder.join(given_path)
        };
    }

    // A drive root such as F:\ has no folder name, so fall back to "book".
    let folder_name = folder
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("book");
    folder.join(format!("{folder_name}.epub"))
}

/// `path` relative to `folder`, written with `/` separators.
/// Returns `None` if `path` is not inside `folder`.
fn path_relative_to(folder: &Path, path: &Path) -> Option<String> {
    let relative = path.strip_prefix(folder).ok()?;
    let parts: Vec<&str> = relative
        .components()
        .filter_map(|component| component.as_os_str().to_str())
        .collect();
    Some(parts.join("/"))
}
