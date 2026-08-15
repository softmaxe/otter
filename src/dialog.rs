//! Native file dialogs, hosted in a short-lived child process.
//!
//! On macOS the AppKit panel opened by `rfd` keeps an invisible window alive after
//! the user dismisses it: the panel sits at `CGShieldingWindowLevel` and its
//! fade-out animation only completes while the process keeps pumping the AppKit
//! run loop. A TUI never does that again once it returns to its own event loop, so
//! the window stays at ~0 alpha above every other window and WindowServer shows the
//! spinning wait cursor over that rectangle for the rest of the process lifetime.
//!
//! Re-executing this binary with [`CHILD_FLAG`] keeps AppKit out of the TUI process
//! entirely. The helper shows one dialog, prints the chosen path to stdout and
//! exits, which takes the leftover window down with it.

use std::{
    env,
    ffi::{OsStr, OsString},
    io::{self, Write},
    os::unix::ffi::{OsStrExt, OsStringExt},
    path::PathBuf,
    process::{Command, Stdio},
};

use rfd::FileDialog;
use thiserror::Error;

/// Hidden first argument that turns this executable into a one-shot dialog helper.
pub const CHILD_FLAG: &str = "--file-dialog";

const OPEN_MODE: &str = "open";
const SAVE_MODE: &str = "save";
const DIRECTORY_FLAG: &str = "--directory";
const FILE_NAME_FLAG: &str = "--file-name";

const INPUT_FILTER: &str = "Media files";
const INPUT_EXTENSIONS: &[&str] = &["mp4", "mkv", "webm", "mov", "avi", "m4v", "ts"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DialogRequest {
    OpenInput,
    SaveOutput {
        directory: Option<PathBuf>,
        file_name: Option<String>,
    },
}

#[derive(Debug, Error)]
pub enum DialogError {
    #[error("Failed to locate the running executable: {0}")]
    CurrentExe(#[source] io::Error),
    #[error("Failed to open the file dialog: {0}")]
    Spawn(#[source] io::Error),
    #[error("The file dialog helper failed: {0}")]
    Helper(String),
    #[error("Unsupported file dialog arguments.")]
    BadArguments,
}

impl DialogRequest {
    /// Arguments that make a fresh process of this binary show exactly this dialog.
    pub fn to_args(&self) -> Vec<OsString> {
        let mut args = vec![OsString::from(CHILD_FLAG)];
        match self {
            Self::OpenInput => args.push(OsString::from(OPEN_MODE)),
            Self::SaveOutput {
                directory,
                file_name,
            } => {
                args.push(OsString::from(SAVE_MODE));
                if let Some(directory) = directory {
                    args.push(OsString::from(DIRECTORY_FLAG));
                    args.push(directory.as_os_str().to_owned());
                }
                if let Some(file_name) = file_name {
                    args.push(OsString::from(FILE_NAME_FLAG));
                    args.push(OsString::from(file_name));
                }
            }
        }
        args
    }

    /// Inverse of [`DialogRequest::to_args`], starting at the [`CHILD_FLAG`].
    pub fn parse_args<I>(args: I) -> Result<Self, DialogError>
    where
        I: IntoIterator<Item = OsString>,
    {
        let mut args = args.into_iter();
        if args.next().as_deref() != Some(OsStr::new(CHILD_FLAG)) {
            return Err(DialogError::BadArguments);
        }
        let mode = args.next().ok_or(DialogError::BadArguments)?;
        if mode == OsStr::new(OPEN_MODE) {
            return match args.next() {
                None => Ok(Self::OpenInput),
                Some(_) => Err(DialogError::BadArguments),
            };
        }
        if mode != OsStr::new(SAVE_MODE) {
            return Err(DialogError::BadArguments);
        }

        let mut directory = None;
        let mut file_name = None;
        while let Some(flag) = args.next() {
            let value = args.next().ok_or(DialogError::BadArguments)?;
            if flag == OsStr::new(DIRECTORY_FLAG) {
                directory = Some(PathBuf::from(value));
            } else if flag == OsStr::new(FILE_NAME_FLAG) {
                file_name = Some(value.into_string().map_err(|_| DialogError::BadArguments)?);
            } else {
                return Err(DialogError::BadArguments);
            }
        }
        Ok(Self::SaveOutput {
            directory,
            file_name,
        })
    }
}

/// Parent side: run the dialog in a helper process and wait for the selection.
pub fn prompt(request: &DialogRequest) -> Result<Option<PathBuf>, DialogError> {
    let program = env::current_exe().map_err(DialogError::CurrentExe)?;
    let output = Command::new(program)
        .args(request.to_args())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(DialogError::Spawn)?;

    if !output.status.success() {
        let message = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        return Err(DialogError::Helper(if message.is_empty() {
            "the helper process exited with an error".to_owned()
        } else {
            message
        }));
    }
    Ok(parse_selection(output.stdout))
}

/// Child side: show one dialog, report the selection on stdout and return.
pub fn run_child(request: &DialogRequest) -> io::Result<()> {
    let Some(path) = show(request) else {
        return Ok(());
    };
    let mut stdout = io::stdout().lock();
    stdout.write_all(path.as_os_str().as_bytes())?;
    stdout.flush()
}

fn show(request: &DialogRequest) -> Option<PathBuf> {
    match request {
        DialogRequest::OpenInput => FileDialog::new()
            .set_title("Choose input media")
            .add_filter(INPUT_FILTER, INPUT_EXTENSIONS)
            .pick_file(),
        DialogRequest::SaveOutput {
            directory,
            file_name,
        } => {
            let mut dialog = FileDialog::new().set_title("Choose output file");
            if let Some(directory) = directory {
                dialog = dialog.set_directory(directory);
            }
            if let Some(file_name) = file_name {
                dialog = dialog.set_file_name(file_name);
            }
            dialog.save_file()
        }
    }
}

/// A cancelled dialog writes nothing, so empty output means "no selection".
fn parse_selection(stdout: Vec<u8>) -> Option<PathBuf> {
    (!stdout.is_empty()).then(|| PathBuf::from(OsString::from_vec(stdout)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip(request: &DialogRequest) -> DialogRequest {
        DialogRequest::parse_args(request.to_args()).expect("arguments should round-trip")
    }

    #[test]
    fn requests_survive_the_argument_round_trip() {
        assert_eq!(
            round_trip(&DialogRequest::OpenInput),
            DialogRequest::OpenInput
        );

        let save = DialogRequest::SaveOutput {
            directory: Some(PathBuf::from("/tmp/a dir with spaces")),
            file_name: Some("clip -v2.transcoded.mp4".to_owned()),
        };
        assert_eq!(round_trip(&save), save);

        let bare = DialogRequest::SaveOutput {
            directory: None,
            file_name: None,
        };
        assert_eq!(round_trip(&bare), bare);
    }

    #[test]
    fn rejects_arguments_that_are_not_a_dialog_request() {
        for args in [
            vec![],
            vec![OsString::from("--help")],
            vec![OsString::from(CHILD_FLAG)],
            vec![OsString::from(CHILD_FLAG), OsString::from("browse")],
            vec![
                OsString::from(CHILD_FLAG),
                OsString::from(OPEN_MODE),
                OsString::from("/etc/passwd"),
            ],
            vec![
                OsString::from(CHILD_FLAG),
                OsString::from(SAVE_MODE),
                OsString::from(DIRECTORY_FLAG),
            ],
        ] {
            assert!(DialogRequest::parse_args(args).is_err());
        }
    }

    #[test]
    fn empty_helper_output_means_cancelled() {
        assert_eq!(parse_selection(Vec::new()), None);
        assert_eq!(
            parse_selection(b"/tmp/my clip.mp4".to_vec()),
            Some(PathBuf::from("/tmp/my clip.mp4"))
        );
    }
}
