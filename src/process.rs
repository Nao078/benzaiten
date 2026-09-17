//! Small, cancellation-aware wrapper around external command line tools.

use std::{
    io::Read,
    path::Path,
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
    time::Duration,
};

const MAX_CAPTURE_BYTES: usize = 64 * 1024;

/// Receives chunks written by a child process while it is still running.
pub type OutputCallback = Arc<dyn Fn(&str) + Send + Sync>;

/// A filename such as `ffmpeg` is deliberately allowed to resolve through
/// `PATH`; a path containing a directory is validated by callers first.
pub fn is_bare_command(path: &Path) -> bool {
    let value = path.as_os_str().to_string_lossy();
    path.file_name().is_some() && !value.contains('/') && !value.contains('\\')
}

#[derive(Debug)]
pub struct Output {
    pub stdout: String,
    pub stderr: String,
}

/// Run an executable without invoking a shell.  On cancellation the child is
/// killed and waited for before this function returns.
pub fn run(program: &Path, args: &[&Path], cancel: Arc<AtomicBool>) -> Result<Output, String> {
    run_with_callback(program, args, cancel, None)
}

/// Run an executable and optionally report its output before it exits.
pub fn run_with_callback(
    program: &Path,
    args: &[&Path],
    cancel: Arc<AtomicBool>,
    callback: Option<OutputCallback>,
) -> Result<Output, String> {
    let mut command = Command::new(program);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW. External tools must not flash a console for GUI users.
        command.creation_flags(0x0800_0000);
    }

    let mut child = command
        .spawn()
        .map_err(|error| format!("could not start {}: {error}", program.display()))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "could not capture tool stdout".to_owned())?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "could not capture tool stderr".to_owned())?;
    let stdout_callback = callback.clone();
    let stderr_callback = callback;
    let stdout_reader = thread::spawn(move || read_bounded(stdout, stdout_callback));
    let stderr_reader = thread::spawn(move || read_bounded(stderr, stderr_callback));

    let status = loop {
        if cancel.load(Ordering::Acquire) {
            let _ = child.kill();
            let _ = child.wait();
            let stdout = join_reader(stdout_reader);
            let stderr = join_reader(stderr_reader);
            return Err(format!(
                "processing cancelled{}",
                format_tool_output(&stdout, &stderr)
            ));
        }
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => thread::sleep(Duration::from_millis(20)),
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                let stdout = join_reader(stdout_reader);
                let stderr = join_reader(stderr_reader);
                return Err(format!(
                    "could not wait for {}: {error}{}",
                    program.display(),
                    format_tool_output(&stdout, &stderr),
                ));
            }
        }
    };

    let stdout = join_reader(stdout_reader);
    let stderr = join_reader(stderr_reader);
    if !status.success() {
        return Err(format!(
            "{} exited with {status}{}",
            program.display(),
            format_tool_output(&stdout, &stderr),
        ));
    }
    Ok(Output { stdout, stderr })
}

fn read_bounded(mut reader: impl Read, callback: Option<OutputCallback>) -> String {
    let mut captured = Vec::new();
    let mut buffer = [0_u8; 4096];
    loop {
        match reader.read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(count) if captured.len() < MAX_CAPTURE_BYTES => {
                if let Some(callback) = &callback {
                    callback(&String::from_utf8_lossy(&buffer[..count]));
                }
                let remaining = MAX_CAPTURE_BYTES - captured.len();
                captured.extend_from_slice(&buffer[..count.min(remaining)]);
            }
            Ok(count) => {
                if let Some(callback) = &callback {
                    callback(&String::from_utf8_lossy(&buffer[..count]));
                }
            }
        }
    }
    String::from_utf8_lossy(&captured).into_owned()
}

fn join_reader(reader: thread::JoinHandle<String>) -> String {
    reader
        .join()
        .unwrap_or_else(|_| "<failed to collect tool output>".to_owned())
}

fn format_tool_output(stdout: &str, stderr: &str) -> String {
    let output = if !stderr.trim().is_empty() {
        stderr
    } else {
        stdout
    };
    if output.trim().is_empty() {
        String::new()
    } else {
        format!(": {}", output.trim())
    }
}
