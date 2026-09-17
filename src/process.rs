//! 外部コマンドラインツールをキャンセル可能に実行する小さなラッパー。

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

/// エラーメッセージ用に保持する子プロセスの標準出力・標準エラーの上限。
/// 出力が多いプロセスでも`Output`が無制限に膨らまないようにする。
/// これを超えた分も`callback`へのストリーミングは続けるが、保持はしない。
const MAX_CAPTURE_BYTES: usize = 64 * 1024;

/// 子プロセスの実行中に書き出されたチャンクを受け取るコールバック。
pub type OutputCallback = Arc<dyn Fn(&str) + Send + Sync>;

/// `ffmpeg`のようなファイル名はあえて`PATH`経由での解決を許可する。
/// ディレクトリを含むパスは呼び出し側が先に検証している前提。
pub fn is_bare_command(path: &Path) -> bool {
    let value = path.as_os_str().to_string_lossy();
    path.file_name().is_some() && !value.contains('/') && !value.contains('\\')
}

/// 終了した子プロセスの標準出力・標準エラー（[`MAX_CAPTURE_BYTES`]により
/// 切り詰められている場合がある）。
#[derive(Debug)]
pub struct Output {
    pub stdout: String,
    pub stderr: String,
}

/// シェルを経由せず実行ファイルを実行する。キャンセルされた場合は、
/// この関数が返る前に子プロセスをkillしてwaitする。
pub fn run(program: &Path, args: &[&Path], cancel: Arc<AtomicBool>) -> Result<Output, String> {
    run_with_callback(program, args, cancel, None)
}

/// 実行ファイルを実行し、任意で終了前の出力を逐次報告する。
///
/// 子プロセスの実行中、約20ms間隔で`cancel`をポーリングする。
/// キャンセルされた場合（または`wait`がエラーになった場合）は、
/// 子プロセスをkillしてwaitし、それまでに取得できた出力とともに
/// `Err`を返す。標準出力・標準エラーは別スレッドで読み出しており、
/// 片方のパイプだけを埋めてもう片方が読まれない子プロセスであっても
/// この呼び出しがデッドロックしないようにしている。
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
        // CREATE_NO_WINDOW。外部ツールがGUIユーザーの前でコンソールを
        // 一瞬表示してしまわないようにする。
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

/// パイプから読めるだけ読み、[`MAX_CAPTURE_BYTES`]までを保持しつつ、
/// 読めたバイト列はすべて`callback`へも渡す。
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

/// 読み出しスレッドの結果を回収する。スレッド自体がパニックした場合は
/// プレースホルダ文字列を返し、呼び出し側の処理は継続できるようにする。
fn join_reader(reader: thread::JoinHandle<String>) -> String {
    reader
        .join()
        .unwrap_or_else(|_| "<failed to collect tool output>".to_owned())
}

/// エラーメッセージに付け加える出力を選ぶ。標準エラーに内容があれば
/// それを優先し、なければ標準出力を使う。両方空ならメッセージは追加しない。
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
