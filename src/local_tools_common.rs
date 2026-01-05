//! Shared utilities for local tool packs.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

/// Error type for local tool pack operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LocalToolError {
    /// Configuration validation failed.
    InvalidConfig(String),
    /// Root directory path is empty.
    RootRequired,
    /// Root path could not be resolved (doesn't exist or permission denied).
    RootNotFound { path: PathBuf, reason: String },
    /// Root path exists but is not a directory.
    RootNotDirectory(PathBuf),
}

impl std::fmt::Display for LocalToolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LocalToolError::InvalidConfig(msg) => write!(f, "invalid config: {msg}"),
            LocalToolError::RootRequired => write!(f, "root directory required"),
            LocalToolError::RootNotFound { path, reason } => {
                write!(f, "resolve root {}: {reason}", path.display())
            }
            LocalToolError::RootNotDirectory(path) => {
                write!(f, "root is not a directory: {}", path.display())
            }
        }
    }
}

impl std::error::Error for LocalToolError {}

/// Validates and resolves root directory (I/O boundary).
///
/// This is a shared implementation used by both LocalBashToolPack and LocalFSToolPack.
pub fn resolve_root(root: &Path) -> Result<PathBuf, LocalToolError> {
    if root.as_os_str().is_empty() {
        return Err(LocalToolError::RootRequired);
    }

    let abs = fs::canonicalize(root).map_err(|err| LocalToolError::RootNotFound {
        path: root.to_path_buf(),
        reason: err.to_string(),
    })?;

    let metadata = fs::metadata(&abs).map_err(|err| LocalToolError::RootNotFound {
        path: abs.clone(),
        reason: err.to_string(),
    })?;

    if !metadata.is_dir() {
        return Err(LocalToolError::RootNotDirectory(abs));
    }

    Ok(abs)
}

/// Environment mode for command execution.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum EnvMode {
    /// No environment variables passed (default, most secure).
    #[default]
    Empty,
    /// Inherit all environment variables from parent process.
    InheritAll,
    /// Only pass specified environment variables.
    Allowlist,
}

/// Filters environment variables based on mode (pure function).
///
/// Takes an iterator of environment variables and filters according to the mode.
/// This allows injecting test environments for unit testing.
pub fn filter_env<I>(
    env_vars: I,
    mode: &EnvMode,
    allowlist: &HashSet<String>,
) -> Vec<(String, String)>
where
    I: Iterator<Item = (String, String)>,
{
    match mode {
        EnvMode::Empty => Vec::new(),
        EnvMode::InheritAll => env_vars.collect(),
        EnvMode::Allowlist => env_vars.filter(|(k, _)| allowlist.contains(k)).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    // ============================================
    // resolve_root tests
    // ============================================

    #[test]
    fn test_resolve_root_empty_path() {
        let result = resolve_root(Path::new(""));
        assert_eq!(result, Err(LocalToolError::RootRequired));
    }

    #[test]
    fn test_resolve_root_nonexistent_path() {
        let result = resolve_root(Path::new("/nonexistent/path/that/does/not/exist"));
        assert!(matches!(result, Err(LocalToolError::RootNotFound { .. })));
    }

    #[test]
    fn test_resolve_root_file_not_directory() {
        // Create a temp file
        let temp_dir = env::temp_dir();
        let temp_file = temp_dir.join(format!("test_file_{}", fastrand::u64(..)));
        fs::write(&temp_file, "test").expect("write temp file");

        let result = resolve_root(&temp_file);
        assert!(matches!(result, Err(LocalToolError::RootNotDirectory(_))));

        fs::remove_file(&temp_file).ok();
    }

    #[test]
    fn test_resolve_root_valid_directory() {
        let temp_dir = env::temp_dir();
        let result = resolve_root(&temp_dir);
        assert!(result.is_ok());
        // Should return canonicalized path
        let resolved = result.unwrap();
        assert!(resolved.is_absolute());
    }

    #[test]
    fn test_resolve_root_resolves_symlinks() {
        // Create a temp directory and symlink to it
        let temp_dir = env::temp_dir();
        let real_dir = temp_dir.join(format!("real_dir_{}", fastrand::u64(..)));
        let symlink_path = temp_dir.join(format!("symlink_{}", fastrand::u64(..)));

        fs::create_dir_all(&real_dir).expect("create real dir");

        #[cfg(unix)]
        std::os::unix::fs::symlink(&real_dir, &symlink_path).expect("create symlink");
        #[cfg(windows)]
        std::os::windows::fs::symlink_dir(&real_dir, &symlink_path).expect("create symlink");

        let result = resolve_root(&symlink_path);
        assert!(result.is_ok());
        let resolved = result.unwrap();
        // Should resolve to the real directory, not the symlink
        assert!(!resolved.to_string_lossy().contains("symlink"));

        fs::remove_dir(&real_dir).ok();
        fs::remove_file(&symlink_path).ok();
    }

    // ============================================
    // filter_env tests
    // ============================================

    #[test]
    fn test_filter_env_empty_mode() {
        let env_vars = vec![
            ("PATH".to_string(), "/usr/bin".to_string()),
            ("HOME".to_string(), "/home/user".to_string()),
        ];
        let result = filter_env(env_vars.into_iter(), &EnvMode::Empty, &HashSet::new());
        assert!(result.is_empty());
    }

    #[test]
    fn test_filter_env_inherit_all() {
        let env_vars = vec![
            ("PATH".to_string(), "/usr/bin".to_string()),
            ("HOME".to_string(), "/home/user".to_string()),
        ];
        let result = filter_env(env_vars.into_iter(), &EnvMode::InheritAll, &HashSet::new());
        assert_eq!(result.len(), 2);
        assert!(result.iter().any(|(k, _)| k == "PATH"));
        assert!(result.iter().any(|(k, _)| k == "HOME"));
    }

    #[test]
    fn test_filter_env_allowlist_filters_correctly() {
        let env_vars = vec![
            ("PATH".to_string(), "/usr/bin".to_string()),
            ("HOME".to_string(), "/home/user".to_string()),
            ("SECRET".to_string(), "do_not_leak".to_string()),
        ];
        let mut allowlist = HashSet::new();
        allowlist.insert("PATH".to_string());
        allowlist.insert("HOME".to_string());

        let result = filter_env(env_vars.into_iter(), &EnvMode::Allowlist, &allowlist);
        assert_eq!(result.len(), 2);
        assert!(result.iter().any(|(k, _)| k == "PATH"));
        assert!(result.iter().any(|(k, _)| k == "HOME"));
        assert!(!result.iter().any(|(k, _)| k == "SECRET"));
    }

    #[test]
    fn test_filter_env_allowlist_empty_allowlist() {
        let env_vars = vec![
            ("PATH".to_string(), "/usr/bin".to_string()),
            ("HOME".to_string(), "/home/user".to_string()),
        ];
        let result = filter_env(env_vars.into_iter(), &EnvMode::Allowlist, &HashSet::new());
        assert!(result.is_empty());
    }

    #[test]
    fn test_filter_env_allowlist_preserves_values() {
        let env_vars = vec![("MY_VAR".to_string(), "my_value".to_string())];
        let mut allowlist = HashSet::new();
        allowlist.insert("MY_VAR".to_string());

        let result = filter_env(env_vars.into_iter(), &EnvMode::Allowlist, &allowlist);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], ("MY_VAR".to_string(), "my_value".to_string()));
    }

    // ============================================
    // LocalToolError display tests
    // ============================================

    #[test]
    fn test_error_display_invalid_config() {
        let err = LocalToolError::InvalidConfig("timeout must be > 0".to_string());
        assert_eq!(err.to_string(), "invalid config: timeout must be > 0");
    }

    #[test]
    fn test_error_display_root_required() {
        let err = LocalToolError::RootRequired;
        assert_eq!(err.to_string(), "root directory required");
    }

    #[test]
    fn test_error_display_root_not_found() {
        let err = LocalToolError::RootNotFound {
            path: PathBuf::from("/some/path"),
            reason: "No such file or directory".to_string(),
        };
        assert!(err.to_string().contains("/some/path"));
        assert!(err.to_string().contains("No such file or directory"));
    }

    #[test]
    fn test_error_display_root_not_directory() {
        let err = LocalToolError::RootNotDirectory(PathBuf::from("/some/file.txt"));
        assert!(err.to_string().contains("/some/file.txt"));
        assert!(err.to_string().contains("not a directory"));
    }
}
