//! tools.v0 local bash tool pack for the Rust SDK.
//!
//! Provides safe-by-default bash command execution with configurable policy.

use std::collections::HashSet;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::bash_policy::BashPolicy;
use crate::local_tools_common::{filter_env, resolve_root, EnvMode, LocalToolError};
use crate::tools::{parse_and_validate_tool_args, sync_handler, ToolRegistry, ValidateArgs};
use crate::types::{Tool, ToolCall};

/// Result of draining a pipe with size limits.
#[derive(Default)]
struct DrainResult {
    /// Collected output bytes.
    output: Vec<u8>,
    /// True if output was truncated due to size limits.
    truncated: bool,
    /// Error message if reading failed (other than EOF).
    read_error: Option<String>,
}

/// Drains a pipe with size limits, continuing to read (and discard) after limit to prevent deadlock.
fn drain_pipe<R: Read>(mut reader: R, max_bytes: usize) -> DrainResult {
    let mut output = Vec::new();
    let mut truncated = false;
    let mut read_error = None;
    let mut buf = [0u8; 8192];

    loop {
        match reader.read(&mut buf) {
            Ok(0) => break, // EOF
            Ok(n) => {
                let remaining = max_bytes.saturating_sub(output.len());
                if remaining == 0 {
                    truncated = true;
                    // Keep reading to drain the pipe, but don't store
                    continue;
                }
                let to_take = n.min(remaining);
                output.extend_from_slice(&buf[..to_take]);
                if to_take < n {
                    truncated = true;
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => {
                // Record the error but still return what we collected
                read_error = Some(format!("read error: {e}"));
                break;
            }
        }
    }

    DrainResult {
        output,
        truncated,
        read_error,
    }
}

/// Spawns a thread to drain a pipe with size limits (generic over Read types).
fn spawn_pipe_reader<R: Read + Send + 'static>(
    reader: Option<R>,
    max_bytes: usize,
) -> JoinHandle<DrainResult> {
    std::thread::spawn(move || match reader {
        Some(r) => drain_pipe(r, max_bytes),
        None => DrainResult::default(),
    })
}

// Default configuration values (matching Go SDK)
const LOCAL_BASH_DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);
const LOCAL_BASH_DEFAULT_MAX_OUTPUT_BYTES: u64 = 32_000;
const LOCAL_BASH_HARD_MAX_OUTPUT_BYTES: u64 = 256_000;

const TOOL_NAME_BASH: &str = "bash";

/// Configuration options for the local bash tool pack.
pub type LocalBashOption = Box<dyn Fn(&mut LocalBashConfig) + Send + Sync + 'static>;

/// Environment source function type for dependency injection.
pub type EnvSource = Box<dyn Fn() -> Vec<(String, String)> + Send + Sync>;

/// Configuration for the local bash tool pack.
#[derive(Clone)]
pub struct LocalBashConfig {
    root_abs: PathBuf,
    timeout: Duration,
    max_output_bytes: u64,
    hard_max_output_bytes: u64,
    policy: BashPolicy,
    env_mode: EnvMode,
    env_allow: HashSet<String>,
    /// Injected environment source for testability. Defaults to std::env::vars().
    env_source: Arc<EnvSource>,
}

impl std::fmt::Debug for LocalBashConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LocalBashConfig")
            .field("root_abs", &self.root_abs)
            .field("timeout", &self.timeout)
            .field("max_output_bytes", &self.max_output_bytes)
            .field("hard_max_output_bytes", &self.hard_max_output_bytes)
            .field("policy", &self.policy)
            .field("env_mode", &self.env_mode)
            .field("env_allow", &self.env_allow)
            .field("env_source", &"<fn>")
            .finish()
    }
}

impl Default for LocalBashConfig {
    fn default() -> Self {
        Self {
            root_abs: PathBuf::new(),
            timeout: LOCAL_BASH_DEFAULT_TIMEOUT,
            max_output_bytes: LOCAL_BASH_DEFAULT_MAX_OUTPUT_BYTES,
            hard_max_output_bytes: LOCAL_BASH_HARD_MAX_OUTPUT_BYTES,
            policy: BashPolicy::new(),
            env_mode: EnvMode::Empty,
            env_allow: HashSet::new(),
            env_source: Arc::new(Box::new(|| std::env::vars().collect())),
        }
    }
}

/// Validates configuration values (pure function - no I/O).
fn validate_config(cfg: &LocalBashConfig) -> Result<(), LocalToolError> {
    if cfg.timeout.is_zero() {
        return Err(LocalToolError::InvalidConfig("timeout must be > 0".into()));
    }
    if cfg.max_output_bytes == 0 {
        return Err(LocalToolError::InvalidConfig(
            "max_output_bytes must be > 0".into(),
        ));
    }
    if cfg.hard_max_output_bytes == 0 {
        return Err(LocalToolError::InvalidConfig(
            "hard_max_output_bytes must be > 0".into(),
        ));
    }
    if cfg.max_output_bytes > cfg.hard_max_output_bytes {
        return Err(LocalToolError::InvalidConfig(
            "max_output_bytes exceeds hard_max_output_bytes".into(),
        ));
    }
    Ok(())
}

/// LocalBashToolPack provides safe-by-default bash command execution.
///
/// # Security Model
///
/// The tool is **deny-by-default**: no commands are allowed unless explicitly enabled via
/// `with_bash_policy()` with explicit allow commands or allow-all.
///
/// Policy deny rules are evaluated before allow rules.
///
/// # Example
///
/// ```ignore
/// use modelrelay::{BashPolicy, LocalBashToolPack, ToolRegistry, with_bash_policy};
/// use std::time::Duration;
///
/// // Create tool pack allowing only gh commands
/// let policy = BashPolicy::new().allow_command("gh");
/// let pack = LocalBashToolPack::new(".", vec![
///     with_bash_policy(policy),
///     with_bash_timeout(Duration::from_secs(30)),
/// ])?;
///
/// let mut registry = ToolRegistry::new();
/// pack.register_into(&mut registry);
/// ```
#[derive(Clone, Debug)]
pub struct LocalBashToolPack {
    cfg: Arc<LocalBashConfig>,
}

/// Sets the timeout for command execution.
pub fn with_bash_timeout(d: Duration) -> LocalBashOption {
    Box::new(move |cfg| cfg.timeout = d)
}

/// Sets the soft limit for output bytes.
/// Output beyond this limit will be truncated.
pub fn with_bash_max_output_bytes(n: u64) -> LocalBashOption {
    Box::new(move |cfg| cfg.max_output_bytes = n)
}

/// Sets the hard cap for output bytes.
/// This is the absolute maximum regardless of requested limits.
pub fn with_bash_hard_max_output_bytes(n: u64) -> LocalBashOption {
    Box::new(move |cfg| cfg.hard_max_output_bytes = n)
}

/// Sets the bash policy for command execution.
pub fn with_bash_policy(policy: BashPolicy) -> LocalBashOption {
    Box::new(move |cfg| cfg.policy = policy.clone())
}

/// Inherits all environment variables from the parent process.
pub fn with_bash_inherit_env() -> LocalBashOption {
    Box::new(|cfg| cfg.env_mode = EnvMode::InheritAll)
}

/// Only passes the specified environment variables to the command.
pub fn with_bash_allow_env_vars(names: Vec<String>) -> LocalBashOption {
    Box::new(move |cfg| {
        cfg.env_mode = EnvMode::Allowlist;
        for name in names.iter().map(|n| n.trim()).filter(|n| !n.is_empty()) {
            cfg.env_allow.insert(name.to_string());
        }
    })
}

/// Injects a custom environment source for testing.
/// By default, the tool pack reads from `std::env::vars()`.
#[cfg(test)]
pub fn with_bash_env_source<F>(source: F) -> LocalBashOption
where
    F: Fn() -> Vec<(String, String)> + Send + Sync + 'static,
{
    Box::new(move |cfg| {
        // Clone the Arc, not the function
        cfg.env_source = Arc::new(Box::new({
            let env_snapshot: Vec<(String, String)> = source();
            move || env_snapshot.clone()
        }));
    })
}

impl LocalBashToolPack {
    /// Creates a LocalBashToolPack sandboxed to the given root directory.
    ///
    /// Returns an error if root is invalid or configuration is invalid.
    pub fn new(
        root: impl AsRef<Path>,
        opts: impl IntoIterator<Item = LocalBashOption>,
    ) -> Result<Self, LocalToolError> {
        let mut cfg = LocalBashConfig::default();
        for opt in opts {
            opt(&mut cfg);
        }

        // Validate configuration (pure function)
        validate_config(&cfg)?;

        // Resolve root directory (I/O boundary)
        cfg.root_abs = resolve_root(root.as_ref())?;

        Ok(Self { cfg: Arc::new(cfg) })
    }

    /// Registers the bash tool into the provided registry.
    pub fn register_into<'a>(&self, registry: &'a mut ToolRegistry) -> &'a mut ToolRegistry {
        let pack = self.clone();
        registry.register_mut(
            TOOL_NAME_BASH,
            sync_handler(move |_args, call| pack.bash_tool(&call)),
        )
    }

    /// Returns the tool definitions for use in API requests.
    ///
    /// This provides the JSON schema that describes the bash tool to the LLM,
    /// so it knows how to call it.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let pack = LocalBashToolPack::new(".", vec![with_bash_policy(...)]);
    /// let tools = pack.tool_definitions();
    ///
    /// let req = ResponsesRequest {
    ///     model: "claude-sonnet-4-5".into(),
    ///     tools: Some(tools),
    ///     messages: vec![...],
    ///     ..Default::default()
    /// };
    /// ```
    pub fn tool_definitions(&self) -> Vec<Tool> {
        vec![Tool::function(
            TOOL_NAME_BASH,
            Some("Execute a bash command in the sandbox directory".to_string()),
            Some(serde_json::json!({
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string",
                        "description": "The bash command to execute"
                    }
                },
                "required": ["command"]
            })),
        )]
    }

    fn check_policy(&self, command: &str) -> Result<(), String> {
        self.cfg.policy.check_command(command).map(|_| ())
    }

    fn build_env(&self) -> Vec<(String, String)> {
        let env_vars = (self.cfg.env_source)();
        filter_env(
            env_vars.into_iter(),
            &self.cfg.env_mode,
            &self.cfg.env_allow,
        )
    }

    fn bash_tool(&self, call: &ToolCall) -> Result<Value, String> {
        let args: BashArgs = parse_and_validate_tool_args(call).map_err(|err| err.message)?;

        // Check command against policy
        self.check_policy(&args.command)?;

        // Build command
        let mut cmd = Command::new("bash");
        cmd.args(["--noprofile", "--norc", "-c", &args.command])
            .current_dir(&self.cfg.root_abs)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env_clear();

        // Set environment variables based on mode
        for (k, v) in self.build_env() {
            cmd.env(k, v);
        }

        // Spawn process
        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(err) => {
                return BashResult::error(format!("failed to spawn bash: {err}"))
                    .into_tool_result();
            }
        };

        // Take stdout/stderr handles immediately to avoid pipe deadlock.
        // We must drain pipes concurrently while waiting for the process,
        // otherwise commands that write more than the pipe buffer (~64KB)
        // will block and appear to timeout.
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let max_bytes = self.cfg.max_output_bytes as usize;

        // Spawn threads to drain pipes
        let stdout_handle = spawn_pipe_reader(stdout, max_bytes);
        let stderr_handle = spawn_pipe_reader(stderr, max_bytes);

        // Wait with timeout while readers drain pipes
        let timeout = self.cfg.timeout;
        let wait_result = self.wait_with_timeout(&mut child, timeout);

        // Collect reader results, surfacing thread panics
        let (stdout_result, stderr_result, thread_error) =
            collect_reader_results(stdout_handle, stderr_handle);

        // Combine outputs and errors
        let (output_str, output_truncated, io_errors) =
            merge_outputs(stdout_result, stderr_result, max_bytes);

        // Build error message from any issues encountered
        let combined_error = combine_errors(thread_error, io_errors);

        match wait_result {
            WaitResult::Exited => {
                // Get exit code, preserving signal information on Unix
                let exit_code = get_exit_code(&mut child);
                if let Some(err) = combined_error {
                    // Command completed but we had I/O issues - include in result
                    BashResult {
                        output: output_str,
                        exit_code,
                        timed_out: false,
                        output_truncated,
                        error: Some(err),
                    }
                    .into_tool_result()
                } else {
                    BashResult::success(output_str, exit_code, output_truncated).into_tool_result()
                }
            }
            WaitResult::TimedOut => {
                BashResult::timeout(output_str, output_truncated, timeout).into_tool_result()
            }
            WaitResult::WaitFailed(err) => {
                BashResult::error(format!("failed to wait for process: {err}")).into_tool_result()
            }
            WaitResult::KillFailed(err) => {
                // Timeout occurred but we couldn't kill - process may still be running
                BashResult {
                    output: output_str,
                    exit_code: -1,
                    timed_out: true,
                    output_truncated,
                    error: Some(format!(
                        "command timed out after {timeout:?}, but failed to kill process: {err}"
                    )),
                }
                .into_tool_result()
            }
        }
    }

    /// Waits for child process with timeout, returning the result.
    fn wait_with_timeout(&self, child: &mut Child, timeout: Duration) -> WaitResult {
        let start = std::time::Instant::now();

        loop {
            match child.try_wait() {
                Ok(Some(_status)) => return WaitResult::Exited,
                Ok(None) => {
                    if start.elapsed() > timeout {
                        // Kill the process - this will cause readers to get EOF
                        if let Err(e) = child.kill() {
                            return WaitResult::KillFailed(e.to_string());
                        }
                        let _ = child.wait();
                        return WaitResult::TimedOut;
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(err) => {
                    let _ = child.kill();
                    return WaitResult::WaitFailed(err.to_string());
                }
            }
        }
    }
}

/// Result of waiting for a child process.
enum WaitResult {
    /// Process exited normally.
    Exited,
    /// Process was killed due to timeout.
    TimedOut,
    /// wait() syscall failed.
    WaitFailed(String),
    /// Timeout occurred but kill() failed.
    KillFailed(String),
}

/// Collects results from reader threads, returning any thread panic as an error.
fn collect_reader_results(
    stdout_handle: JoinHandle<DrainResult>,
    stderr_handle: JoinHandle<DrainResult>,
) -> (DrainResult, DrainResult, Option<String>) {
    let stdout_result = match stdout_handle.join() {
        Ok(r) => r,
        Err(_) => {
            return (
                DrainResult::default(),
                stderr_handle.join().unwrap_or_default(),
                Some("stdout reader thread panicked".to_string()),
            );
        }
    };

    let stderr_result = match stderr_handle.join() {
        Ok(r) => r,
        Err(_) => {
            return (
                stdout_result,
                DrainResult::default(),
                Some("stderr reader thread panicked".to_string()),
            );
        }
    };

    (stdout_result, stderr_result, None)
}

/// Merges stdout and stderr outputs, respecting max_bytes limit.
/// Returns (combined_output, was_truncated, io_errors).
fn merge_outputs(
    stdout: DrainResult,
    stderr: DrainResult,
    max_bytes: usize,
) -> (String, bool, Vec<String>) {
    let mut output = stdout.output;
    let mut truncated = stdout.truncated;
    let mut errors = Vec::new();

    if let Some(e) = stdout.read_error {
        errors.push(format!("stdout: {e}"));
    }
    if let Some(e) = stderr.read_error {
        errors.push(format!("stderr: {e}"));
    }

    // Append stderr to output, respecting limit
    let remaining = max_bytes.saturating_sub(output.len());
    if remaining > 0 && !stderr.output.is_empty() {
        let to_take = stderr.output.len().min(remaining);
        output.extend_from_slice(&stderr.output[..to_take]);
        if to_take < stderr.output.len() || stderr.truncated {
            truncated = true;
        }
    } else if !stderr.output.is_empty() || stderr.truncated {
        truncated = true;
    }

    let output_str = String::from_utf8_lossy(&output).to_string();
    (output_str, truncated, errors)
}

/// Combines thread and I/O errors into a single error message, if any.
fn combine_errors(thread_error: Option<String>, io_errors: Vec<String>) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(e) = thread_error {
        parts.push(e);
    }
    parts.extend(io_errors);

    if parts.is_empty() {
        None
    } else {
        Some(parts.join("; "))
    }
}

/// Gets exit code from child, returning -1 for signals or errors.
fn get_exit_code(child: &mut Child) -> i32 {
    match child.wait() {
        Ok(status) => status.code().unwrap_or(-1),
        Err(_) => -1,
    }
}

/// Creates a ToolRegistry with the LocalBashToolPack registered.
pub fn new_local_bash_tools(
    root: impl AsRef<Path>,
    opts: impl IntoIterator<Item = LocalBashOption>,
) -> Result<ToolRegistry, LocalToolError> {
    let pack = LocalBashToolPack::new(root, opts)?;
    let mut registry = ToolRegistry::new();
    pack.register_into(&mut registry);
    Ok(registry)
}

/// Arguments for the bash tool.
#[derive(Debug, Deserialize)]
struct BashArgs {
    command: String,
}

impl ValidateArgs for BashArgs {
    fn validate(&self) -> Result<(), String> {
        if self.command.trim().is_empty() {
            return Err("command cannot be empty".to_string());
        }
        Ok(())
    }
}

/// Result of a bash command execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BashResult {
    /// Combined stdout and stderr output.
    pub output: String,
    /// Command exit code (-1 if process failed to run).
    pub exit_code: i32,
    /// True if the command timed out.
    #[serde(default, skip_serializing_if = "is_false")]
    pub timed_out: bool,
    /// True if output was truncated due to size limits.
    #[serde(default, skip_serializing_if = "is_false")]
    pub output_truncated: bool,
    /// Error message if command failed to execute.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl BashResult {
    /// Creates an error result with the given message.
    fn error(msg: impl Into<String>) -> Self {
        Self {
            output: String::new(),
            exit_code: -1,
            timed_out: false,
            output_truncated: false,
            error: Some(msg.into()),
        }
    }

    /// Creates a timeout result with captured output.
    fn timeout(output: String, output_truncated: bool, duration: Duration) -> Self {
        Self {
            output,
            exit_code: -1,
            timed_out: true,
            output_truncated,
            error: Some(format!("command timed out after {duration:?}")),
        }
    }

    /// Creates a successful result.
    fn success(output: String, exit_code: i32, output_truncated: bool) -> Self {
        Self {
            output,
            exit_code,
            timed_out: false,
            output_truncated,
            error: None,
        }
    }

    /// Converts to JSON Value for tool results.
    /// Returns Err only if serialization fails (should never happen for BashResult).
    fn into_tool_result(self) -> Result<Value, String> {
        serde_json::to_value(self).map_err(|e| format!("failed to serialize result: {e}"))
    }
}

fn is_false(b: &bool) -> bool {
    !*b
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bash_policy::BashPolicy;
    use crate::types::{FunctionCall, ToolCall, ToolType};
    use std::fs;

    struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        fn new() -> Self {
            let mut path = std::env::temp_dir();
            path.push(format!(
                "modelrelay-rust-bash-{}",
                fastrand::u64(0..u64::MAX)
            ));
            fs::create_dir_all(&path).expect("create temp dir");
            Self { path }
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            if let Err(e) = fs::remove_dir_all(&self.path) {
                eprintln!("warning: failed to clean up temp dir {:?}: {e}", self.path);
            }
        }
    }

    fn tool_call(name: &str, args: serde_json::Value) -> ToolCall {
        ToolCall {
            id: "call_1".to_string(),
            kind: ToolType::Function,
            function: Some(FunctionCall {
                name: name.to_string(),
                arguments: args.to_string(),
            }),
        }
    }

    #[tokio::test]
    async fn test_bash_default_deny() {
        let temp = TempDir::new();

        // No allow policy = denied
        let pack = LocalBashToolPack::new(temp.path(), vec![]).expect("create pack");
        let mut registry = ToolRegistry::new();
        pack.register_into(&mut registry);

        let call = tool_call(TOOL_NAME_BASH, serde_json::json!({"command": "echo hello"}));
        let result = registry.execute(&call).await;
        assert!(result.is_err());
        assert!(result
            .error
            .unwrap_or_default()
            .contains("bash tool disabled by default"));
    }

    #[tokio::test]
    async fn test_bash_allow_all() {
        let temp = TempDir::new();

        let pack = LocalBashToolPack::new(
            temp.path(),
            vec![with_bash_policy(BashPolicy::new().allow_all())],
        )
        .expect("create pack");
        let mut registry = ToolRegistry::new();
        pack.register_into(&mut registry);

        let call = tool_call(TOOL_NAME_BASH, serde_json::json!({"command": "echo hello"}));
        let result = registry.execute(&call).await;
        assert!(result.is_ok());
        let output: BashResult =
            serde_json::from_value(result.result.unwrap()).expect("parse result");
        assert_eq!(output.exit_code, 0);
        assert!(output.output.contains("hello"));
    }

    #[tokio::test]
    async fn test_bash_allow_command() {
        let temp = TempDir::new();

        let pack = LocalBashToolPack::new(
            temp.path(),
            vec![with_bash_policy(BashPolicy::new().allow_command("echo"))],
        )
        .expect("create pack");
        let mut registry = ToolRegistry::new();
        pack.register_into(&mut registry);

        // Allowed: echo command
        let call = tool_call(TOOL_NAME_BASH, serde_json::json!({"command": "echo hello"}));
        let result = registry.execute(&call).await;
        assert!(result.is_ok());

        // Denied: ls command
        let call = tool_call(TOOL_NAME_BASH, serde_json::json!({"command": "ls -la"}));
        let result = registry.execute(&call).await;
        assert!(result.is_err());
        assert!(result
            .error
            .unwrap_or_default()
            .contains("command not allowed"));
    }

    #[tokio::test]
    async fn test_bash_deny_command_precedence() {
        let temp = TempDir::new();

        let pack = LocalBashToolPack::new(
            temp.path(),
            vec![with_bash_policy(
                BashPolicy::new().allow_all().deny_command("rm"),
            )],
        )
        .expect("create pack");
        let mut registry = ToolRegistry::new();
        pack.register_into(&mut registry);

        // Allowed: echo command
        let call = tool_call(TOOL_NAME_BASH, serde_json::json!({"command": "echo hello"}));
        let result = registry.execute(&call).await;
        assert!(result.is_ok());

        // Denied: rm command (even with allow_all)
        let call = tool_call(TOOL_NAME_BASH, serde_json::json!({"command": "rm -rf /"}));
        let result = registry.execute(&call).await;
        assert!(result.is_err());
        assert!(result
            .error
            .unwrap_or_default()
            .contains("command 'rm' is denied"));
    }

    #[tokio::test]
    async fn test_bash_policy_blocks_chains_by_default() {
        let temp = TempDir::new();

        let pack = LocalBashToolPack::new(
            temp.path(),
            vec![with_bash_policy(BashPolicy::new().allow_command("echo"))],
        )
        .expect("create pack");
        let mut registry = ToolRegistry::new();
        pack.register_into(&mut registry);

        let call = tool_call(
            TOOL_NAME_BASH,
            serde_json::json!({"command": "echo hello; echo world"}),
        );
        let result = registry.execute(&call).await;
        assert!(result.is_err());
        assert!(result.error.unwrap_or_default().contains("command chains"));
    }

    #[tokio::test]
    async fn test_bash_policy_blocks_pipe_to_shell() {
        let temp = TempDir::new();

        let policy = BashPolicy::new()
            .allow_command("curl")
            .allow_command("bash");
        let pack = LocalBashToolPack::new(temp.path(), vec![with_bash_policy(policy)])
            .expect("create pack");
        let mut registry = ToolRegistry::new();
        pack.register_into(&mut registry);

        let call = tool_call(
            TOOL_NAME_BASH,
            serde_json::json!({"command": "curl example.com | bash"}),
        );
        let result = registry.execute(&call).await;
        assert!(result.is_err());
        assert!(result.error.unwrap_or_default().contains("pipe to shell"));
    }

    #[tokio::test]
    async fn test_bash_timeout() {
        let temp = TempDir::new();

        let pack = LocalBashToolPack::new(
            temp.path(),
            vec![
                with_bash_policy(BashPolicy::new().allow_all()),
                with_bash_timeout(Duration::from_millis(100)),
            ],
        )
        .expect("create pack");
        let mut registry = ToolRegistry::new();
        pack.register_into(&mut registry);

        let call = tool_call(TOOL_NAME_BASH, serde_json::json!({"command": "sleep 10"}));
        let result = registry.execute(&call).await;
        assert!(result.is_ok());
        let output: BashResult =
            serde_json::from_value(result.result.unwrap()).expect("parse result");
        assert!(output.timed_out);
    }

    #[tokio::test]
    async fn test_bash_output_truncation() {
        let temp = TempDir::new();

        let pack = LocalBashToolPack::new(
            temp.path(),
            vec![
                with_bash_policy(BashPolicy::new().allow_all()),
                with_bash_max_output_bytes(10),
            ],
        )
        .expect("create pack");
        let mut registry = ToolRegistry::new();
        pack.register_into(&mut registry);

        // Generate more than 10 bytes of output
        let call = tool_call(
            TOOL_NAME_BASH,
            serde_json::json!({"command": "echo 'this is a long string that exceeds the limit'"}),
        );
        let result = registry.execute(&call).await;
        assert!(result.is_ok());
        let output: BashResult =
            serde_json::from_value(result.result.unwrap()).expect("parse result");
        assert!(output.output_truncated);
        assert!(output.output.len() <= 10);
    }

    #[tokio::test]
    async fn test_bash_empty_command_rejected() {
        let temp = TempDir::new();

        let pack = LocalBashToolPack::new(
            temp.path(),
            vec![with_bash_policy(BashPolicy::new().allow_all())],
        )
        .expect("create pack");
        let mut registry = ToolRegistry::new();
        pack.register_into(&mut registry);

        let call = tool_call(TOOL_NAME_BASH, serde_json::json!({"command": "  "}));
        let result = registry.execute(&call).await;
        assert!(result.is_err());
        assert!(result
            .error
            .unwrap_or_default()
            .contains("command cannot be empty"));
    }

    #[tokio::test]
    async fn test_bash_high_output_no_deadlock() {
        // Regression test: commands producing output > pipe buffer (~64KB) should
        // complete normally, not appear to timeout due to pipe deadlock.
        let temp = TempDir::new();

        let pack = LocalBashToolPack::new(
            temp.path(),
            vec![
                with_bash_policy(BashPolicy::new().allow_all()),
                with_bash_timeout(Duration::from_secs(5)),
                with_bash_max_output_bytes(1000), // Cap output but still drain pipe
            ],
        )
        .expect("create pack");
        let mut registry = ToolRegistry::new();
        pack.register_into(&mut registry);

        // Generate ~100KB of output (larger than typical 64KB pipe buffer)
        let call = tool_call(
            TOOL_NAME_BASH,
            serde_json::json!({"command": "seq 1 20000"}),
        );
        let result = registry.execute(&call).await;
        assert!(result.is_ok());

        let output: BashResult =
            serde_json::from_value(result.result.unwrap()).expect("parse result");

        // Should complete without timeout
        assert!(!output.timed_out, "high-output command should not timeout");
        assert_eq!(output.exit_code, 0);
        // Output should be truncated but present
        assert!(output.output_truncated);
        assert!(!output.output.is_empty());
    }
}
