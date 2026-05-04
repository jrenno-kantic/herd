use tokio::process::Command;
use tokio::time::{timeout, Duration};

const SHELL_TIMEOUT: Duration = Duration::from_secs(30);

pub async fn run_shell(command: &str) -> String {
    let exec = Command::new("sh").arg("-c").arg(command).output();

    match timeout(SHELL_TIMEOUT, exec).await {
        Ok(Ok(output)) if output.status.success() => {
            String::from_utf8_lossy(&output.stdout).trim().to_string()
        }
        Ok(Ok(output)) => {
            let code = output.status.code().unwrap_or(-1);
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            format!("exit {}: {}", code, stderr)
        }
        Ok(Err(error)) => error.to_string(),
        Err(_) => format!("timeout after {}s", SHELL_TIMEOUT.as_secs()),
    }
}
