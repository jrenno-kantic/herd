use std::process::{ExitStatus, Stdio};
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::{Child, Command};
use tokio::time::{timeout, Duration};

const SHELL_TIMEOUT: Duration = Duration::from_secs(30);
const TERM_GRACE: Duration = Duration::from_millis(500);
const KILL_GRACE: Duration = Duration::from_secs(2);

pub async fn run_shell(command: &str) -> String {
    run_shell_with_timeout(command, SHELL_TIMEOUT).await
}

async fn run_shell_with_timeout(command: &str, limit: Duration) -> String {
    let mut shell = Command::new("sh");
    shell
        .arg("-c")
        .arg(command)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        // A cancelled task must never detach a shell. Timeout handling
        // below is explicit; this is the last-resort path for runtime
        // shutdown or a panic between spawn and wait.
        .kill_on_drop(true);

    // Give the shell its own process group. Killing only `sh` leaves a
    // background `sleep`, compiler, or downloader alive after herd has
    // already reported a timeout.
    #[cfg(unix)]
    shell.process_group(0);

    let mut child = match shell.spawn() {
        Ok(child) => child,
        Err(error) => return error.to_string(),
    };

    let pid = child.id();
    let stdout = child.stdout.take().map(read_all);
    let stderr = child.stderr.take().map(read_all);

    match timeout(limit, child.wait()).await {
        Ok(Ok(status)) => format_output(status, join(stdout).await, join(stderr).await),
        Ok(Err(error)) => error.to_string(),
        Err(_) => {
            stop_timed_out(&mut child, pid).await;
            // Do not leave pipe readers holding the task alive. Once the
            // whole process group is gone both readers finish promptly.
            let _ = timeout(KILL_GRACE, join(stdout)).await;
            let _ = timeout(KILL_GRACE, join(stderr)).await;
            format!("timeout after {}s", limit.as_secs_f64())
        }
    }
}

fn read_all<R>(mut reader: R) -> tokio::task::JoinHandle<Vec<u8>>
where
    R: AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        let mut bytes = Vec::new();
        let _ = reader.read_to_end(&mut bytes).await;
        bytes
    })
}

async fn join(handle: Option<tokio::task::JoinHandle<Vec<u8>>>) -> Vec<u8> {
    match handle {
        Some(handle) => handle.await.unwrap_or_default(),
        None => Vec::new(),
    }
}

fn format_output(status: ExitStatus, stdout: Vec<u8>, stderr: Vec<u8>) -> String {
    if status.success() {
        String::from_utf8_lossy(&stdout).trim().to_string()
    } else {
        let code = status.code().unwrap_or(-1);
        let stderr = String::from_utf8_lossy(&stderr).trim().to_string();
        format!("exit {code}: {stderr}")
    }
}

async fn stop_timed_out(child: &mut Child, pid: Option<u32>) {
    #[cfg(unix)]
    if let Some(pid) = pid {
        signal_group(pid, libc::SIGTERM);
    }

    #[cfg(not(unix))]
    let _ = child.start_kill();

    let reaped = timeout(TERM_GRACE, child.wait()).await.is_ok();

    #[cfg(unix)]
    if let Some(pid) = pid {
        // The shell may have obeyed TERM while one of its children did
        // not. The process group can therefore outlive the child handle.
        if group_exists(pid) {
            signal_group(pid, libc::SIGKILL);
        }
    }

    if !reaped {
        let _ = child.start_kill();
        let _ = timeout(KILL_GRACE, child.wait()).await;
    }
}

#[cfg(unix)]
fn signal_group(pid: u32, signal: libc::c_int) {
    // SAFETY: `pid` came from the child we spawned as a process-group
    // leader. A negative pid asks kill(2) to address that group.
    unsafe {
        libc::kill(-(pid as libc::pid_t), signal);
    }
}

#[cfg(unix)]
fn group_exists(pid: u32) -> bool {
    // Signal 0 performs existence/permission checking without delivering
    // a signal. EPERM still means the group exists.
    unsafe {
        if libc::kill(-(pid as libc::pid_t), 0) == 0 {
            true
        } else {
            std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn successful_output_is_trimmed() {
        assert_eq!(
            run_shell_with_timeout("printf hello", Duration::from_secs(1)).await,
            "hello"
        );
    }

    #[tokio::test]
    async fn a_nonzero_exit_reports_its_code_and_stderr() {
        assert_eq!(
            run_shell_with_timeout("printf broken >&2; exit 7", Duration::from_secs(1)).await,
            "exit 7: broken"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_timeout_kills_descendants_that_ignore_term() {
        let path = std::env::temp_dir().join(format!(
            "herd-shell-child-{}-{:?}.pid",
            std::process::id(),
            std::thread::current().id()
        ));
        let command = format!(
            "sh -c 'trap \"\" TERM; echo $$ > \"{}\"; while :; do sleep 1; done' & wait",
            path.display()
        );

        let result = run_shell_with_timeout(&command, Duration::from_millis(150)).await;
        assert!(result.starts_with("timeout after "), "{result}");

        let pid: libc::pid_t = std::fs::read_to_string(&path)
            .expect("descendant wrote its pid")
            .trim()
            .parse()
            .expect("numeric pid");
        let _ = std::fs::remove_file(path);

        for _ in 0..40 {
            // SAFETY: signal 0 only checks whether this recorded pid is
            // still present; it does not affect the process.
            let gone = unsafe { libc::kill(pid, 0) } != 0
                && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH);
            if gone {
                return;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }

        panic!("timed-out descendant {pid} is still alive");
    }
}
