//! External programs a task can run on its file ("Perform actions", `ExternalProgram`).
//!
//! The arguments are split into a vector first and the placeholders filled in after, so a
//! file name can never become more than the one argument it is (PORT.md section 5).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// `ExternalProgram`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct ExternalProgram {
    pub is_active: bool,
    pub name: Option<String>,
    pub path: Option<String>,
    pub args: Option<String>,
    pub output_extension: Option<String>,
    pub extensions: Option<String>,
    pub hidden_window: bool,
    pub delete_input_file: bool,
}

impl Default for ExternalProgram {
    fn default() -> Self {
        ExternalProgram {
            is_active: false,
            name: None,
            path: None,
            // `'"' + $input + '"'`.
            args: Some("\"$input\"".into()),
            output_extension: None,
            extensions: None,
            hidden_window: false,
            delete_input_file: false,
        }
    }
}

/// What a run produced.
#[derive(Clone, Debug, PartialEq)]
pub struct ActionResult {
    /// The file the task goes on with (the output when the program wrote one).
    pub path: PathBuf,
    /// The input, to delete once the task is done with it (`DeleteInputFile`).
    pub pending_delete: Option<PathBuf>,
}

impl ExternalProgram {
    /// `GetFullPath`.
    pub fn full_path(&self) -> String {
        crate::names::file_helpers::expand_folder_variables(
            self.path.as_deref().unwrap_or(""),
            false,
        )
    }

    /// `CheckExtension`: letters and digits make an extension, anything else separates.
    pub fn check_extension(&self, path: &str) -> bool {
        check_extension(path, self.extensions.as_deref().unwrap_or(""))
    }

    /// The output path for an input (`OutputExtension`).
    pub fn output_path(&self, input: &Path) -> PathBuf {
        match self
            .output_extension
            .as_deref()
            .map(str::trim)
            .filter(|e| !e.is_empty())
        {
            Some(extension) => {
                let stem = input
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let extension = extension.strip_prefix('.').unwrap_or(extension);
                input.with_file_name(format!("{stem}.{extension}"))
            }
            None => input.to_path_buf(),
        }
    }

    /// The argument vector for an input and output (`CodeMenuEntryActions.Parse`).
    pub fn arguments(&self, input: &Path, output: &Path) -> Vec<String> {
        let input = input.to_string_lossy();
        let output = output.to_string_lossy();
        match self.args.as_deref().filter(|a| !a.is_empty()) {
            None => vec![input.into_owned()],
            Some(args) => crate::recording::args::split_command_line(args)
                .into_iter()
                .map(|arg| {
                    // Inside one argument the quoted forms are the plain ones: quoting is
                    // what the split already did.
                    arg.replace("%input", &input)
                        .replace("$input", &input)
                        .replace("%output", &output)
                        .replace("$output", &output)
                })
                .collect(),
        }
    }

    /// `Run`: the program on the file, waiting for it to finish.
    pub fn run(&self, input: &Path) -> Option<ActionResult> {
        let program = self.full_path();
        if program.is_empty() || !Path::new(&program).is_file() || !input.is_file() {
            return None;
        }
        if !self.check_extension(&input.to_string_lossy()) {
            return None;
        }
        let output = if self.args.as_deref().is_some_and(|a| !a.is_empty()) {
            self.output_path(input)
        } else {
            input.to_path_buf()
        };
        let args = self.arguments(input, &output);
        log::info!("Action run: \"{program}\" {}", args.join(" "));
        let mut command = std::process::Command::new(&program);
        command.args(&args);
        // `CreateNoWindow = HiddenWindow`: on Windows a console program shows its
        // console unless the action asks for a hidden window, as upstream.
        if self.hidden_window {
            crate::components::process::no_window(&mut command);
        }
        let status = command.status();
        if let Err(error) = status {
            log::warn!("Action failed: {error}");
            return None;
        }
        if output.is_file() {
            let pending_delete =
                (self.delete_input_file && !same_path(&output, input)).then(|| input.to_path_buf());
            return Some(ActionResult {
                path: output,
                pending_delete,
            });
        }
        Some(ActionResult {
            path: input.to_path_buf(),
            pending_delete: None,
        })
    }
}

/// Whether the output is the input itself. Windows compares without case, as upstream
/// does (`OrdinalIgnoreCase`): "a.png" with the output extension "PNG" is the same file
/// there, and deleting the input would delete the output.
fn same_path(output: &Path, input: &Path) -> bool {
    if cfg!(windows) {
        output.to_string_lossy().to_lowercase() == input.to_string_lossy().to_lowercase()
    } else {
        output == input
    }
}

/// `ExternalProgram.CheckExtension(path, extensions)`.
pub fn check_extension(path: &str, extensions: &str) -> bool {
    if path.trim().is_empty() {
        return false;
    }
    if extensions.trim().is_empty() {
        return true;
    }
    let lower = path.to_lowercase();
    extensions
        .split(|c: char| !c.is_alphanumeric())
        .filter(|e| !e.is_empty())
        .any(|e| lower.ends_with(&format!(".{}", e.to_lowercase())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extensions_are_any_run_of_letters_and_digits() {
        assert!(check_extension("/a/b.PNG", "png jpg"));
        assert!(check_extension("/a/b.jpg", "png,jpg;gif"));
        assert!(!check_extension("/a/b.webp", "png jpg"));
        assert!(check_extension("/a/b.webp", ""));
        assert!(!check_extension("  ", ""));
    }

    #[test]
    fn placeholders_fill_whole_arguments() {
        let program = ExternalProgram {
            args: Some(r#"-i "%input" -o $output --flag"#.into()),
            output_extension: Some(".webp".into()),
            ..ExternalProgram::default()
        };
        let folder = if cfg!(windows) { "C:\\tmp\\" } else { "/tmp/" };
        let input = PathBuf::from(format!("{folder}a b; rm -rf ~.png"));
        let input = input.as_path();
        let output = program.output_path(input);
        assert_eq!(output, PathBuf::from(format!("{folder}a b; rm -rf ~.webp")));
        assert_eq!(
            program.arguments(input, &output),
            [
                "-i".to_string(),
                format!("{folder}a b; rm -rf ~.png"),
                "-o".to_string(),
                format!("{folder}a b; rm -rf ~.webp"),
                "--flag".to_string()
            ]
        );
        let bare = ExternalProgram {
            args: None,
            ..ExternalProgram::default()
        };
        assert_eq!(
            bare.arguments(input, input),
            [format!("{folder}a b; rm -rf ~.png")]
        );
        assert_eq!(
            ExternalProgram::default().arguments(input, input),
            [format!("{folder}a b; rm -rf ~.png")]
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_program_runs_and_its_output_is_taken() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("in.txt");
        std::fs::write(&input, "x").unwrap();
        let program = ExternalProgram {
            is_active: true,
            path: Some("/bin/cp".into()),
            args: Some("$input $output".into()),
            output_extension: Some("out".into()),
            delete_input_file: true,
            ..ExternalProgram::default()
        };
        let result = program.run(&input).unwrap();
        assert_eq!(result.path, dir.path().join("in.out"));
        assert_eq!(result.pending_delete, Some(input.clone()));
        assert!(result.path.is_file());
    }

    #[cfg(windows)]
    #[test]
    fn a_program_runs_and_its_output_is_taken_on_windows() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("in.txt");
        std::fs::write(&input, "x").unwrap();
        // The program's path may hold environment variables, as upstream's `GetFullPath`.
        let program = ExternalProgram {
            is_active: true,
            path: Some("%SystemRoot%\\System32\\cmd.exe".into()),
            args: Some("/C copy /Y $input $output".into()),
            output_extension: Some("out".into()),
            hidden_window: true,
            delete_input_file: true,
            ..ExternalProgram::default()
        };
        let result = program.run(&input).unwrap();
        assert_eq!(result.path, dir.path().join("in.out"));
        assert_eq!(result.pending_delete, Some(input.clone()));
        assert!(result.path.is_file());

        // An output that differs only in case is the input itself on Windows: it is
        // never handed over for deletion.
        let same = ExternalProgram {
            output_extension: Some("TXT".into()),
            ..program
        };
        let result = same.run(&input).unwrap();
        assert_eq!(result.pending_delete, None);
    }
}
