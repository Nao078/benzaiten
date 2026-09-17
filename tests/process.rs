use std::{
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};

use benzaiten::process;

#[test]
fn bare_command_names_are_left_for_path_resolution() {
    assert!(process::is_bare_command(Path::new("ffmpeg")));
    assert!(!process::is_bare_command(Path::new(".\\tools\\ffmpeg.exe")));
}

/// The current test executable is a portable child process for the integration
/// test below. It writes more than the capture limit and stays alive long
/// enough to be cancelled.
#[test]
fn process_child_mode() {
    if !std::env::args().any(|arg| arg == "process_child_mode")
        || std::env::var_os("LYRIC_SYNC_PROCESS_CHILD").is_none()
    {
        return;
    }
    for _ in 0..20_000 {
        println!("0123456789abcdef");
    }
    thread::sleep(Duration::from_secs(10));
}

#[test]
fn cancellation_reaps_child_while_output_is_bounded() {
    let executable = std::env::current_exe().unwrap();
    let cancel = Arc::new(AtomicBool::new(false));
    let canceller = Arc::clone(&cancel);
    let prior = std::env::var_os("LYRIC_SYNC_PROCESS_CHILD");
    std::env::set_var("LYRIC_SYNC_PROCESS_CHILD", "1");
    let stopper = thread::spawn(move || {
        thread::sleep(Duration::from_millis(100));
        canceller.store(true, Ordering::Release);
    });

    let started = Instant::now();
    let result = process::run(
        &executable,
        &[
            Path::new("--exact"),
            Path::new("process_child_mode"),
            Path::new("--nocapture"),
        ],
        cancel,
    );
    stopper.join().unwrap();
    match prior {
        Some(value) => std::env::set_var("LYRIC_SYNC_PROCESS_CHILD", value),
        None => std::env::remove_var("LYRIC_SYNC_PROCESS_CHILD"),
    }

    let error = result.unwrap_err();
    assert!(error.contains("processing cancelled"));
    assert!(error.len() <= 66 * 1024, "captured output was not bounded");
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "child was not reaped promptly"
    );
}
