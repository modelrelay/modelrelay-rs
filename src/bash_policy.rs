//! Semantic bash policy based on lightweight tokenization.

use std::collections::HashSet;

use crate::bash_tokenizer::{tokenize_bash, BashTokens};

/// Semantic bash policy with safety rails.
#[derive(Clone, Debug)]
pub struct BashPolicy {
    allow_all: bool,
    allow_commands: HashSet<String>,
    deny_commands: HashSet<String>,
    deny_pipe_to_shell: bool,
    deny_command_chains: bool,
    deny_eval: bool,
    deny_subshells: bool,
    deny_dangerous_redirects: bool,
}

impl BashPolicy {
    /// Create a new policy with safety rails enabled by default.
    pub fn new() -> Self {
        Self {
            allow_all: false,
            allow_commands: HashSet::new(),
            deny_commands: HashSet::new(),
            deny_pipe_to_shell: true,
            deny_command_chains: true,
            deny_eval: true,
            deny_subshells: true,
            deny_dangerous_redirects: false,
        }
    }

    /// Allow a specific command by name (normalized).
    pub fn allow_command(mut self, command: impl Into<String>) -> Self {
        self.allow_commands
            .insert(normalize_command_name(&command.into()));
        self
    }

    /// Deny a specific command by name (normalized).
    pub fn deny_command(mut self, command: impl Into<String>) -> Self {
        self.deny_commands
            .insert(normalize_command_name(&command.into()));
        self
    }

    /// Allow all commands (subject to deny rules and safety rails).
    pub fn allow_all(mut self) -> Self {
        self.allow_all = true;
        self
    }

    /// Deny pipes to shells (| bash, | sh, | zsh).
    pub fn deny_pipe_to_shell(mut self) -> Self {
        self.deny_pipe_to_shell = true;
        self
    }

    /// Allow pipes to shells.
    pub fn allow_pipe_to_shell(mut self) -> Self {
        self.deny_pipe_to_shell = false;
        self
    }

    /// Deny command chains (;, &&, ||).
    pub fn deny_command_chains(mut self) -> Self {
        self.deny_command_chains = true;
        self
    }

    /// Allow command chains (;, &&, ||).
    pub fn allow_chains(mut self) -> Self {
        self.deny_command_chains = false;
        self
    }

    /// Deny eval/exec usage.
    pub fn deny_eval(mut self) -> Self {
        self.deny_eval = true;
        self
    }

    /// Allow eval/exec usage.
    pub fn allow_eval(mut self) -> Self {
        self.deny_eval = false;
        self
    }

    /// Deny subshell usage ($(), backticks, bash -c).
    pub fn deny_subshells(mut self) -> Self {
        self.deny_subshells = true;
        self
    }

    /// Allow subshell usage ($(), backticks, bash -c).
    pub fn allow_subshells(mut self) -> Self {
        self.deny_subshells = false;
        self
    }

    /// Deny redirects to sensitive paths (e.g. /etc/*).
    pub fn deny_dangerous_redirects(mut self) -> Self {
        self.deny_dangerous_redirects = true;
        self
    }

    /// Allow redirects to sensitive paths.
    pub fn allow_dangerous_redirects(mut self) -> Self {
        self.deny_dangerous_redirects = false;
        self
    }

    /// Check a raw command string against the policy.
    pub fn check_command(&self, command: &str) -> Result<BashTokens, String> {
        let tokens = tokenize_bash(command);
        self.check_tokens(&tokens)?;
        Ok(tokens)
    }

    /// Check tokenized command data against the policy.
    pub fn check_tokens(&self, tokens: &BashTokens) -> Result<(), String> {
        if tokens.primary_command.is_empty() && !self.allow_all {
            return Err("bash tool denied: unable to determine primary command".to_string());
        }

        if tokens.has_chain && self.deny_command_chains {
            return Err("bash tool denied by policy: command chains are not allowed".to_string());
        }

        if tokens.has_pipe && self.deny_pipe_to_shell && pipe_to_shell(tokens) {
            return Err("bash tool denied by policy: pipe to shell not allowed".to_string());
        }

        if tokens.has_eval && self.deny_eval {
            return Err("bash tool denied by policy: eval/exec not allowed".to_string());
        }

        if tokens.has_subshell && self.deny_subshells {
            return Err("bash tool denied by policy: subshells not allowed".to_string());
        }

        if tokens.has_dangerous_redirect && self.deny_dangerous_redirects {
            return Err("bash tool denied by policy: dangerous redirect not allowed".to_string());
        }

        if !tokens.all_commands.is_empty() {
            for cmd in tokens.all_commands.iter() {
                if self.deny_commands.contains(cmd) {
                    return Err(format!(
                        "bash tool denied by policy: command '{cmd}' is denied"
                    ));
                }
            }
        }

        if self.allow_all {
            return Ok(());
        }

        if self.allow_commands.is_empty() {
            return Err("bash tool disabled by default: configure BashPolicy with allow_command() or allow_all()"
                .to_string());
        }

        let mut missing = Vec::new();
        for cmd in tokens.all_commands.iter() {
            if !self.allow_commands.contains(cmd) {
                missing.push(cmd.clone());
            }
        }

        if !missing.is_empty() {
            return Err(format!(
                "bash tool denied: command not allowed: {}",
                missing.join(", ")
            ));
        }

        Ok(())
    }
}

impl Default for BashPolicy {
    fn default() -> Self {
        Self::new()
    }
}

fn normalize_command_name(command: &str) -> String {
    let trimmed = command.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let path = std::path::Path::new(trimmed);
    if let Some(name) = path.file_name() {
        name.to_string_lossy().to_string()
    } else {
        trimmed.to_string()
    }
}

fn pipe_to_shell(tokens: &BashTokens) -> bool {
    if !tokens.has_pipe {
        return false;
    }
    tokens
        .all_commands
        .iter()
        .any(|cmd| matches!(cmd.as_str(), "sh" | "bash" | "zsh" | "dash" | "ksh"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn policy_blocks_subshell_by_default() {
        let policy = BashPolicy::new().allow_command("echo");
        let err = policy
            .check_command("$(echo rm) file")
            .expect_err("should deny subshells");
        assert!(err.contains("subshells"));
    }

    #[test]
    fn policy_blocks_eval_by_default() {
        let policy = BashPolicy::new().allow_command("eval");
        let err = policy
            .check_command("eval \"rm -rf /\"")
            .expect_err("should deny eval");
        assert!(err.contains("eval"));
    }

    #[test]
    fn policy_blocks_pipe_to_shell() {
        let policy = BashPolicy::new()
            .allow_command("curl")
            .allow_command("bash");
        let err = policy
            .check_command("curl x | bash")
            .expect_err("should deny pipe to shell");
        assert!(err.contains("pipe"));
    }

    #[test]
    fn policy_allows_explicit_opt_out() {
        let policy = BashPolicy::new().allow_command("make").allow_chains();
        policy
            .check_command("make && make install")
            .expect("should allow chains when opted in");
    }

    #[test]
    fn policy_allows_env_prefixed_command() {
        let policy = BashPolicy::new().allow_command("npm");
        policy
            .check_command("FOO=bar npm run build")
            .expect("should allow env-prefixed command");
    }
}
