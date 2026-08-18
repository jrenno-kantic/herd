//! Startup checks for the external programs HERD delegates work to.
//!
//! Presence means more than finding a filename on `PATH`: executing the
//! version command also catches a non-executable file or a broken binary.

use std::io::ErrorKind;
use std::time::Duration;
use tokio::process::Command;

const PROBE_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tool {
    pub name: &'static str,
    pub version: Result<String, String>,
}

impl Tool {
    pub fn available(&self) -> bool {
        self.version.is_ok()
    }

    pub fn label(&self) -> &str {
        match &self.version {
            Ok(version) => version,
            Err(error) => error,
        }
    }

    fn assumed(name: &'static str) -> Self {
        Self {
            name,
            version: Ok("available (not probed)".to_string()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tools {
    pub llama_server: Tool,
    pub hf: Tool,
}

impl Tools {
    /// Used by pure UI constructors and tests. Production startup replaces
    /// this with [`check`], so no test outcome depends on the host's PATH.
    pub fn assumed() -> Self {
        Self {
            llama_server: Tool::assumed("llama-server"),
            hf: Tool::assumed("hf"),
        }
    }

    pub fn required_error(&self) -> Option<&str> {
        self.llama_server.version.as_ref().err().map(String::as_str)
    }
}

pub async fn check() -> Tools {
    let (llama_server, hf) = tokio::join!(check_tool("llama-server"), check_tool("hf"));
    Tools { llama_server, hf }
}

async fn check_tool(name: &'static str) -> Tool {
    let probe =
        tokio::time::timeout(PROBE_TIMEOUT, Command::new(name).arg("--version").output()).await;
    let version = match probe {
        Ok(Ok(output)) if output.status.success() => {
            let text = format!(
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            Ok(text
                .lines()
                .map(str::trim)
                .find(|line| !line.is_empty())
                .unwrap_or("version unknown")
                .to_string())
        }
        Ok(Ok(output)) => Err(format!("`{name} --version` exited with {}", output.status)),
        Ok(Err(error)) if error.kind() == ErrorKind::NotFound => {
            Err(format!("`{name}` was not found on PATH"))
        }
        Ok(Err(error)) => Err(format!("could not execute `{name}`: {error}")),
        Err(_) => Err(format!("`{name} --version` timed out after 3s")),
    };

    Tool { name, version }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_missing_tool_is_reported_clearly() {
        let tool = check_tool("herd-tool-that-does-not-exist").await;

        assert!(!tool.available());
        assert!(tool.label().contains("not found on PATH"), "{tool:?}");
    }

    #[test]
    fn assumed_tools_keep_pure_ui_tests_independent_of_path() {
        let tools = Tools::assumed();

        assert!(tools.llama_server.available());
        assert!(tools.hf.available());
    }

    #[test]
    fn only_llama_server_is_required() {
        let mut tools = Tools::assumed();
        tools.hf.version = Err("missing hf".into());
        assert_eq!(tools.required_error(), None);

        tools.llama_server.version = Err("missing llama-server".into());
        assert_eq!(tools.required_error(), Some("missing llama-server"));
    }
}
