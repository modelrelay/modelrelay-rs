//! tools.v0 local filesystem tool pack for the Rust SDK.

use std::collections::HashSet;
use std::fs;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::Deserialize;
use serde_json::Value;

use crate::local_tools_common::{resolve_root, LocalToolError};
use crate::tools::{parse_and_validate_tool_args, sync_handler, ToolRegistry, ValidateArgs};
use crate::types::{Tool, ToolCall};

const LOCAL_FS_DEFAULT_MAX_READ_BYTES: u64 = 64_000;
const LOCAL_FS_HARD_MAX_READ_BYTES: u64 = 1_000_000;
const LOCAL_FS_DEFAULT_MAX_LIST_ENTRIES: u64 = 2_000;
const LOCAL_FS_HARD_MAX_LIST_ENTRIES: u64 = 20_000;
const LOCAL_FS_DEFAULT_MAX_SEARCH_MATCHES: u64 = 100;
const LOCAL_FS_HARD_MAX_SEARCH_MATCHES: u64 = 2_000;
const LOCAL_FS_DEFAULT_SEARCH_TIMEOUT: Duration = Duration::from_secs(5);
const LOCAL_FS_DEFAULT_MAX_SEARCH_BYTES: u64 = 1_000_000;

const TOOL_FS_READ_FILE: &str = "fs_read_file";
const TOOL_FS_LIST_FILES: &str = "fs_list_files";
const TOOL_FS_SEARCH: &str = "fs_search";
const TOOL_FS_EDIT: &str = "fs_edit";

/// Configuration options for the local filesystem tool pack.
pub type LocalFSOption = Box<dyn Fn(&mut LocalFSConfig) + Send + Sync + 'static>;

#[derive(Debug, Clone)]
pub struct LocalFSConfig {
    root_abs: PathBuf,

    ignore_dir_names: HashSet<String>,

    max_read_bytes: u64,
    hard_max_read_bytes: u64,

    max_list_entries: u64,
    hard_max_list_entries: u64,

    max_search_matches: u64,
    hard_max_search_matches: u64,

    search_timeout: Duration,
    max_search_bytes: u64,
}

impl Default for LocalFSConfig {
    fn default() -> Self {
        Self {
            root_abs: PathBuf::new(),
            ignore_dir_names: default_ignored_dir_names(),
            max_read_bytes: LOCAL_FS_DEFAULT_MAX_READ_BYTES,
            hard_max_read_bytes: LOCAL_FS_HARD_MAX_READ_BYTES,
            max_list_entries: LOCAL_FS_DEFAULT_MAX_LIST_ENTRIES,
            hard_max_list_entries: LOCAL_FS_HARD_MAX_LIST_ENTRIES,
            max_search_matches: LOCAL_FS_DEFAULT_MAX_SEARCH_MATCHES,
            hard_max_search_matches: LOCAL_FS_HARD_MAX_SEARCH_MATCHES,
            search_timeout: LOCAL_FS_DEFAULT_SEARCH_TIMEOUT,
            max_search_bytes: LOCAL_FS_DEFAULT_MAX_SEARCH_BYTES,
        }
    }
}

/// LocalFSToolPack provides safe-by-default implementations of tools.v0 filesystem tools:
/// - fs.read_file
/// - fs.list_files
/// - fs.search
/// - fs.edit
///
/// The pack enforces a root sandbox, path traversal prevention, ignore lists, and size/time caps.
#[derive(Clone, Debug)]
pub struct LocalFSToolPack {
    cfg: Arc<LocalFSConfig>,
}

#[derive(Debug, Clone)]
struct ReadPlan {
    rel_path: PathBuf,
    max_bytes: u64,
}

#[derive(Debug, Clone)]
struct ListPlan {
    rel_path: PathBuf,
    max_entries: u64,
}

#[derive(Debug, Clone)]
struct SearchPlan {
    rel_path: PathBuf,
    max_matches: u64,
    timeout: Duration,
    max_search_bytes: u64,
    regex: regex::Regex,
}

#[derive(Debug, Clone)]
struct ResolvedPath {
    abs: PathBuf,
}

/// WithLocalFSIgnoreDirs configures directory names to skip during fs.list_files and fs.search.
/// Names are matched by path segment (e.g. ".git" skips any ".git" directory anywhere under root).
pub fn with_local_fs_ignore_dirs(names: Vec<String>) -> LocalFSOption {
    Box::new(move |cfg| {
        for name in names.iter().map(|n| n.trim()).filter(|n| !n.is_empty()) {
            cfg.ignore_dir_names.insert(name.to_string());
        }
    })
}

/// WithLocalFSMaxReadBytes changes the default max_bytes when the tool call does not specify max_bytes.
pub fn with_local_fs_max_read_bytes(n: u64) -> LocalFSOption {
    Box::new(move |cfg| cfg.max_read_bytes = n)
}

/// WithLocalFSHardMaxReadBytes changes the hard cap for fs.read_file max_bytes.
pub fn with_local_fs_hard_max_read_bytes(n: u64) -> LocalFSOption {
    Box::new(move |cfg| cfg.hard_max_read_bytes = n)
}

/// WithLocalFSMaxListEntries changes the default max_entries when the tool call does not specify max_entries.
pub fn with_local_fs_max_list_entries(n: u64) -> LocalFSOption {
    Box::new(move |cfg| cfg.max_list_entries = n)
}

/// WithLocalFSHardMaxListEntries changes the hard cap for fs.list_files max_entries.
pub fn with_local_fs_hard_max_list_entries(n: u64) -> LocalFSOption {
    Box::new(move |cfg| cfg.hard_max_list_entries = n)
}

/// WithLocalFSMaxSearchMatches changes the default max_matches when the tool call does not specify max_matches.
pub fn with_local_fs_max_search_matches(n: u64) -> LocalFSOption {
    Box::new(move |cfg| cfg.max_search_matches = n)
}

/// WithLocalFSHardMaxSearchMatches changes the hard cap for fs.search max_matches.
pub fn with_local_fs_hard_max_search_matches(n: u64) -> LocalFSOption {
    Box::new(move |cfg| cfg.hard_max_search_matches = n)
}

/// WithLocalFSSearchTimeout configures the timeout for fs.search.
pub fn with_local_fs_search_timeout(d: Duration) -> LocalFSOption {
    Box::new(move |cfg| cfg.search_timeout = d)
}

/// WithLocalFSMaxSearchBytes sets a per-file byte limit for the fs.search implementation.
pub fn with_local_fs_max_search_bytes(n: u64) -> LocalFSOption {
    Box::new(move |cfg| cfg.max_search_bytes = n)
}

impl LocalFSToolPack {
    /// Creates a LocalFSToolPack sandboxed to the given root directory.
    ///
    /// Returns an error if root is invalid.
    pub fn new(
        root: impl AsRef<Path>,
        opts: impl IntoIterator<Item = LocalFSOption>,
    ) -> Result<Self, LocalToolError> {
        let mut cfg = LocalFSConfig::default();
        for opt in opts {
            opt(&mut cfg);
        }

        // Resolve root directory (I/O boundary)
        cfg.root_abs = resolve_root(root.as_ref())?;

        Ok(Self { cfg: Arc::new(cfg) })
    }

    /// Registers fs.* tools into the provided registry.
    pub fn register_into<'a>(&self, registry: &'a mut ToolRegistry) -> &'a mut ToolRegistry {
        let pack = self.clone();
        registry.register_mut(
            TOOL_FS_READ_FILE,
            sync_handler(move |_args, call| pack.read_file_tool(&call)),
        );

        let pack = self.clone();
        registry.register_mut(
            TOOL_FS_LIST_FILES,
            sync_handler(move |_args, call| pack.list_files_tool(&call)),
        );

        let pack = self.clone();
        registry.register_mut(
            TOOL_FS_SEARCH,
            sync_handler(move |_args, call| pack.search_tool(&call)),
        );

        let pack = self.clone();
        registry.register_mut(
            TOOL_FS_EDIT,
            sync_handler(move |_args, call| pack.edit_tool(&call)),
        );

        registry
    }

    /// Returns the tool definitions for use in API requests.
    ///
    /// This provides the JSON schemas that describe the filesystem tools to the LLM,
    /// so it knows how to call them.
    pub fn tool_definitions(&self) -> Vec<Tool> {
        vec![
            Tool::function(
                TOOL_FS_READ_FILE,
                Some("Read a file from the workspace".to_string()),
                Some(serde_json::json!({
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "Workspace-relative path to the file"
                        },
                        "max_bytes": {
                            "type": "integer",
                            "description": "Maximum bytes to read (optional)"
                        }
                    },
                    "required": ["path"]
                })),
            ),
            Tool::function(
                TOOL_FS_LIST_FILES,
                Some("List files in a directory recursively".to_string()),
                Some(serde_json::json!({
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "Workspace-relative path to list (defaults to root)"
                        },
                        "max_entries": {
                            "type": "integer",
                            "description": "Maximum entries to return (optional)"
                        }
                    }
                })),
            ),
            Tool::function(
                TOOL_FS_SEARCH,
                Some("Search for a pattern in files".to_string()),
                Some(serde_json::json!({
                    "type": "object",
                    "properties": {
                        "query": {
                            "type": "string",
                            "description": "Regex pattern to search for"
                        },
                        "path": {
                            "type": "string",
                            "description": "Workspace-relative path to search in (defaults to root)"
                        },
                        "max_matches": {
                            "type": "integer",
                            "description": "Maximum matches to return (optional)"
                        }
                    },
                    "required": ["query"]
                })),
            ),
            Tool::function(
                TOOL_FS_EDIT,
                Some("Replace exact string matches in a file".to_string()),
                Some(serde_json::json!({
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "Workspace-relative path to the file"
                        },
                        "old_string": {
                            "type": "string",
                            "description": "Exact string to find and replace"
                        },
                        "new_string": {
                            "type": "string",
                            "description": "Replacement string (may be empty)"
                        },
                        "replace_all": {
                            "type": "boolean",
                            "default": false,
                            "description": "Replace all occurrences (default: first only)"
                        }
                    },
                    "required": ["path", "old_string", "new_string"]
                })),
            ),
        ]
    }

    fn plan_read(&self, args: &FsReadFileArgs) -> Result<ReadPlan, String> {
        let rel_path = parse_rel_path(&args.path)?;
        let max_bytes = resolve_cap(
            args.max_bytes,
            self.cfg.max_read_bytes,
            self.cfg.hard_max_read_bytes,
            "max_bytes",
        )?;
        Ok(ReadPlan {
            rel_path,
            max_bytes,
        })
    }

    fn plan_list(&self, args: &FsListFilesArgs) -> Result<ListPlan, String> {
        let rel_path = parse_optional_rel_path(args.path.as_deref())?;
        let max_entries = resolve_cap(
            args.max_entries,
            self.cfg.max_list_entries,
            self.cfg.hard_max_list_entries,
            "max_entries",
        )?;
        Ok(ListPlan {
            rel_path,
            max_entries,
        })
    }

    fn plan_search(&self, args: &FsSearchArgs) -> Result<SearchPlan, String> {
        let rel_path = parse_optional_rel_path(args.path.as_deref())?;
        let max_matches = resolve_cap(
            args.max_matches,
            self.cfg.max_search_matches,
            self.cfg.hard_max_search_matches,
            "max_matches",
        )?;
        let timeout = if self.cfg.search_timeout.is_zero() {
            LOCAL_FS_DEFAULT_SEARCH_TIMEOUT
        } else {
            self.cfg.search_timeout
        };
        let max_search_bytes = if self.cfg.max_search_bytes == 0 {
            LOCAL_FS_DEFAULT_MAX_SEARCH_BYTES
        } else {
            self.cfg.max_search_bytes
        };
        let regex = regex::Regex::new(&args.query)
            .map_err(|err| invalid_args(format!("invalid query regex: {err}")))?;
        Ok(SearchPlan {
            rel_path,
            max_matches,
            timeout,
            max_search_bytes,
            regex,
        })
    }

    fn resolve_existing_path(&self, rel: &Path) -> Result<ResolvedPath, String> {
        let target = self.cfg.root_abs.join(rel);

        let eval = fs::canonicalize(&target)
            .map_err(|err| format!("local fs tools: resolve path: {err}"))?;

        eval.strip_prefix(&self.cfg.root_abs)
            .map_err(|_| "local fs tools: path escapes root".to_string())?;

        Ok(ResolvedPath { abs: eval })
    }

    fn read_file_tool(&self, call: &ToolCall) -> Result<Value, String> {
        let args: FsReadFileArgs = parse_and_validate_tool_args(call).map_err(|err| err.message)?;

        let plan = self.plan_read(&args)?;
        let resolved = self.resolve_existing_path(&plan.rel_path)?;

        let metadata =
            fs::metadata(&resolved.abs).map_err(|err| format!("fs.read_file: stat: {err}"))?;
        if metadata.is_dir() {
            return Err(format!("fs.read_file: path is a directory: {}", args.path));
        }

        let file =
            fs::File::open(&resolved.abs).map_err(|err| format!("fs.read_file: open: {err}"))?;

        let mut buf = Vec::new();
        let limit = plan.max_bytes.saturating_add(1);
        file.take(limit)
            .read_to_end(&mut buf)
            .map_err(|err| format!("fs.read_file: read: {err}"))?;

        if buf.len() as u64 > plan.max_bytes {
            return Err(format!(
                "fs.read_file: file exceeds max_bytes ({})",
                plan.max_bytes
            ));
        }

        let contents = String::from_utf8(buf)
            .map_err(|_| format!("fs.read_file: file is not valid UTF-8: {}", args.path))?;

        Ok(Value::String(contents))
    }

    fn list_files_tool(&self, call: &ToolCall) -> Result<Value, String> {
        let args: FsListFilesArgs =
            parse_and_validate_tool_args(call).map_err(|err| err.message)?;

        let start_display = args
            .path
            .as_deref()
            .map(|p| p.trim())
            .filter(|p| !p.is_empty())
            .unwrap_or(".");
        let plan = self.plan_list(&args)?;

        let resolved = self.resolve_existing_path(&plan.rel_path)?;

        let metadata =
            fs::metadata(&resolved.abs).map_err(|err| format!("fs.list_files: stat: {err}"))?;
        if !metadata.is_dir() {
            return Err(format!(
                "fs.list_files: path is not a directory: {start_display}",
            ));
        }

        let mut out: Vec<String> = Vec::new();
        let mut stack: Vec<PathBuf> = vec![resolved.abs];

        while let Some(dir) = stack.pop() {
            let entries =
                fs::read_dir(&dir).map_err(|err| format!("fs.list_files: walk: {err}"))?;

            for entry in entries {
                let entry = entry.map_err(|err| format!("fs.list_files: walk: {err}"))?;
                let file_type = entry
                    .file_type()
                    .map_err(|err| format!("fs.list_files: walk: {err}"))?;

                let name = entry.file_name();
                if file_type.is_dir() {
                    if self.is_ignored_dir(&name) {
                        continue;
                    }
                    stack.push(entry.path());
                    continue;
                }

                let entry_path = entry.path();
                let rel = entry_path
                    .strip_prefix(&self.cfg.root_abs)
                    .map_err(|_| "fs.list_files: internal error: escaped root".to_string())?;

                if rel.as_os_str().is_empty() {
                    return Err("fs.list_files: internal error: escaped root".to_string());
                }

                out.push(normalize_rel_path(rel));
                if out.len() as u64 >= plan.max_entries {
                    return Ok(Value::String(out.join("\n")));
                }
            }
        }

        Ok(Value::String(out.join("\n")))
    }

    fn search_tool(&self, call: &ToolCall) -> Result<Value, String> {
        let args: FsSearchArgs = parse_and_validate_tool_args(call).map_err(|err| err.message)?;

        let start_display = args
            .path
            .as_deref()
            .map(|p| p.trim())
            .filter(|p| !p.is_empty())
            .unwrap_or(".");
        let plan = self.plan_search(&args)?;

        let resolved = self.resolve_existing_path(&plan.rel_path)?;
        let metadata =
            fs::metadata(&resolved.abs).map_err(|err| format!("fs.search: stat: {err}"))?;
        if !metadata.is_dir() {
            return Err(format!(
                "fs.search: path is not a directory: {start_display}"
            ));
        }

        let timeout = plan.timeout;
        let deadline = Instant::now() + timeout;

        let mut out: Vec<String> = Vec::new();
        let mut stack: Vec<PathBuf> = vec![resolved.abs];

        while let Some(dir) = stack.pop() {
            if Instant::now() > deadline {
                return Err(format!("fs.search: timed out after {timeout:?}"));
            }

            let entries = fs::read_dir(&dir).map_err(|err| format!("fs.search: walk: {err}"))?;
            for entry in entries {
                if Instant::now() > deadline {
                    return Err(format!("fs.search: timed out after {timeout:?}"));
                }

                let entry = entry.map_err(|err| format!("fs.search: walk: {err}"))?;
                let file_type = entry
                    .file_type()
                    .map_err(|err| format!("fs.search: walk: {err}"))?;

                let name = entry.file_name();
                if file_type.is_dir() {
                    if self.is_ignored_dir(&name) {
                        continue;
                    }
                    stack.push(entry.path());
                    continue;
                }

                if file_type.is_symlink() {
                    // Skip symlinks to avoid ambiguous containment.
                    continue;
                }

                let entry_path = entry.path();
                let rel = entry_path
                    .strip_prefix(&self.cfg.root_abs)
                    .map_err(|_| "fs.search: internal error: escaped root".to_string())?;

                if rel.as_os_str().is_empty() {
                    return Err("fs.search: internal error: escaped root".to_string());
                }

                let rel_str = normalize_rel_path(rel);

                let file = match fs::File::open(&entry_path) {
                    Ok(f) => f,
                    Err(_) => continue,
                };

                let mut reader = BufReader::new(file);
                let mut line = String::new();
                let mut line_no = 0usize;
                let mut bytes_read = 0u64;

                loop {
                    line.clear();
                    let read_res = reader.read_line(&mut line);
                    let read = match read_res {
                        Ok(0) => break,
                        Ok(n) => n,
                        Err(err) => {
                            if err.kind() == io::ErrorKind::InvalidData {
                                // Treat as binary; stop scanning this file.
                                break;
                            }
                            return Err(format!("fs.search: read: {err}"));
                        }
                    };

                    bytes_read = bytes_read.saturating_add(read as u64);
                    if bytes_read > plan.max_search_bytes {
                        break;
                    }

                    line_no += 1;
                    let trimmed = line.trim_end_matches(&['\r', '\n'][..]);
                    if plan.regex.is_match(trimmed) {
                        out.push(format!("{rel_str}:{line_no}:{trimmed}"));
                        if out.len() as u64 >= plan.max_matches {
                            return Ok(Value::String(out.join("\n")));
                        }
                    }

                    if Instant::now() > deadline {
                        return Err(format!("fs.search: timed out after {timeout:?}"));
                    }
                }
            }
        }

        Ok(Value::String(out.join("\n")))
    }

    fn edit_tool(&self, call: &ToolCall) -> Result<Value, String> {
        let args: FsEditArgs = parse_and_validate_tool_args(call).map_err(|err| err.message)?;
        let FsEditArgs {
            path,
            old_string,
            new_string,
            replace_all,
        } = args;

        let rel_path = parse_rel_path(&path)?;
        let resolved = self.resolve_existing_path(&rel_path)?;
        let metadata =
            fs::metadata(&resolved.abs).map_err(|err| format!("fs.edit: stat: {err}"))?;
        if metadata.is_dir() {
            return Err(format!("fs.edit: path is a directory: {path}"));
        }

        let data = fs::read(&resolved.abs).map_err(|err| format!("fs.edit: read: {err}"))?;
        let contents = String::from_utf8(data)
            .map_err(|_| format!("fs.edit: file is not valid UTF-8: {path}"))?;

        let old_string = old_string.ok_or_else(|| "old_string is required".to_string())?;
        let new_string = new_string.ok_or_else(|| "new_string is required".to_string())?;
        let replace_all = replace_all.unwrap_or(false);

        let matches = find_occurrences(&contents, &old_string);
        if matches.is_empty() {
            return Err("fs.edit: old_string not found".to_string());
        }
        if !replace_all && matches.len() > 1 {
            return Err("fs.edit: old_string appears multiple times".to_string());
        }

        let applied = if replace_all {
            matches.clone()
        } else {
            vec![matches[0]]
        };

        let updated = if replace_all {
            contents.replace(&old_string, &new_string)
        } else {
            contents.replacen(&old_string, &new_string, 1)
        };

        let mut file = fs::OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(&resolved.abs)
            .map_err(|err| format!("fs.edit: open: {err}"))?;
        file.write_all(updated.as_bytes())
            .map_err(|err| format!("fs.edit: write: {err}"))?;

        let line_spec =
            format_line_ranges(build_line_ranges(&contents, &applied, old_string.len()));
        let rel = normalize_rel_path(&rel_path);
        Ok(Value::String(format!(
            "Edited {rel} (replacements={}, lines {line_spec})",
            applied.len()
        )))
    }

    fn is_ignored_dir(&self, name: &std::ffi::OsStr) -> bool {
        let name = name.to_string_lossy();
        self.cfg.ignore_dir_names.contains(name.as_ref())
    }
}

/// Creates a ToolRegistry with the LocalFSToolPack registered.
pub fn new_local_fs_tools(
    root: impl AsRef<Path>,
    opts: impl IntoIterator<Item = LocalFSOption>,
) -> Result<ToolRegistry, LocalToolError> {
    let pack = LocalFSToolPack::new(root, opts)?;
    let mut registry = ToolRegistry::new();
    pack.register_into(&mut registry);
    Ok(registry)
}

fn invalid_args(message: impl Into<String>) -> String {
    format!("invalid arguments: {}", message.into())
}

fn normalize_rel_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn parse_optional_rel_path(raw: Option<&str>) -> Result<PathBuf, String> {
    let start = raw
        .map(|p| p.trim())
        .filter(|p| !p.is_empty())
        .unwrap_or(".");
    parse_rel_path(start)
}

fn parse_rel_path(raw: &str) -> Result<PathBuf, String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err(invalid_args("path is required"));
    }

    let normalized = raw.replace('\\', "/");
    let path = PathBuf::from(normalized);
    if path.is_absolute() {
        return Err(invalid_args(
            "path must be workspace-relative (not absolute)",
        ));
    }

    for comp in path.components() {
        match comp {
            Component::ParentDir => {
                return Err(invalid_args("path must not escape the workspace root"))
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(invalid_args(
                    "path must be workspace-relative (not absolute)",
                ))
            }
            _ => {}
        }
    }

    Ok(path)
}

fn resolve_cap(requested: Option<u64>, default: u64, hard: u64, name: &str) -> Result<u64, String> {
    let mut value = default;
    if let Some(requested) = requested {
        if requested > hard {
            return Err(invalid_args(format!("{name} exceeds hard cap ({hard})")));
        }
        value = requested;
    }
    if value == 0 {
        value = 1;
    }
    Ok(value)
}

#[derive(Debug, Clone, Copy)]
struct LineRange {
    start: usize,
    end: usize,
}

fn find_occurrences(haystack: &str, needle: &str) -> Vec<usize> {
    if needle.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut offset = 0;
    while offset <= haystack.len() {
        let remaining = &haystack[offset..];
        let Some(pos) = remaining.find(needle) else {
            break;
        };
        let idx = offset + pos;
        out.push(idx);
        offset = idx + needle.len();
        if offset >= haystack.len() {
            break;
        }
    }
    out
}

fn build_line_ranges(contents: &str, positions: &[usize], needle_len: usize) -> Vec<LineRange> {
    if positions.is_empty() {
        return Vec::new();
    }
    let mut newlines = Vec::with_capacity(contents.matches('\n').count());
    for (idx, byte) in contents.as_bytes().iter().enumerate() {
        if *byte == b'\n' {
            newlines.push(idx);
        }
    }
    let span = if needle_len == 0 { 1 } else { needle_len };
    positions
        .iter()
        .map(|pos| LineRange {
            start: line_number_at(&newlines, *pos),
            end: line_number_at(&newlines, pos.saturating_add(span - 1)),
        })
        .collect()
}

fn line_number_at(newlines: &[usize], index: usize) -> usize {
    let mut lo = 0;
    let mut hi = newlines.len();
    while lo < hi {
        let mid = (lo + hi) / 2;
        if newlines[mid] < index {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    lo + 1
}

fn format_line_ranges(ranges: Vec<LineRange>) -> String {
    if ranges.is_empty() {
        return String::new();
    }
    let mut merged: Vec<LineRange> = Vec::with_capacity(ranges.len());
    for range in ranges {
        if let Some(last) = merged.last_mut() {
            if range.start <= last.end + 1 {
                if range.end > last.end {
                    last.end = range.end;
                }
                continue;
            }
        }
        merged.push(range);
    }
    merged
        .into_iter()
        .map(|range| {
            if range.start == range.end {
                range.start.to_string()
            } else {
                format!("{}-{}", range.start, range.end)
            }
        })
        .collect::<Vec<_>>()
        .join(",")
}

fn default_ignored_dir_names() -> HashSet<String> {
    let ignored = [
        ".git",
        "node_modules",
        "vendor",
        "dist",
        "build",
        ".next",
        "target",
        ".idea",
        ".vscode",
    ];
    ignored.iter().map(|n| n.to_string()).collect()
}

#[derive(Debug, Deserialize)]
struct FsReadFileArgs {
    path: String,
    #[serde(default)]
    max_bytes: Option<u64>,
}

impl ValidateArgs for FsReadFileArgs {
    fn validate(&self) -> Result<(), String> {
        if self.path.trim().is_empty() {
            return Err("path is required".to_string());
        }
        if let Some(max_bytes) = self.max_bytes {
            if max_bytes == 0 {
                return Err("max_bytes must be > 0".to_string());
            }
        }
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
struct FsListFilesArgs {
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    max_entries: Option<u64>,
}

impl ValidateArgs for FsListFilesArgs {
    fn validate(&self) -> Result<(), String> {
        if let Some(max_entries) = self.max_entries {
            if max_entries == 0 {
                return Err("max_entries must be > 0".to_string());
            }
        }
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
struct FsSearchArgs {
    query: String,
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    max_matches: Option<u64>,
}

impl ValidateArgs for FsSearchArgs {
    fn validate(&self) -> Result<(), String> {
        if self.query.trim().is_empty() {
            return Err("query is required".to_string());
        }
        if let Some(max_matches) = self.max_matches {
            if max_matches == 0 {
                return Err("max_matches must be > 0".to_string());
            }
        }
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
struct FsEditArgs {
    path: String,
    #[serde(default)]
    old_string: Option<String>,
    #[serde(default)]
    new_string: Option<String>,
    #[serde(default)]
    replace_all: Option<bool>,
}

impl ValidateArgs for FsEditArgs {
    fn validate(&self) -> Result<(), String> {
        if self.path.trim().is_empty() {
            return Err("path is required".to_string());
        }
        match &self.old_string {
            Some(value) if !value.trim().is_empty() => {}
            _ => return Err("old_string is required".to_string()),
        }
        if self.new_string.is_none() {
            return Err("new_string is required".to_string());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{FunctionCall, ToolCall, ToolType};

    struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        fn new() -> Self {
            let mut path = std::env::temp_dir();
            path.push(format!("modelrelay-rust-fs-{}", fastrand::u64(0..u64::MAX)));
            fs::create_dir_all(&path).expect("create temp dir");
            Self { path }
        }

        fn path(&self) -> &Path {
            &self.path
        }

        fn write_file(&self, rel: &str, contents: &str) {
            let path = self.path.join(rel.replace('\\', "/"));
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).expect("create parent");
            }
            fs::write(path, contents).expect("write file");
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
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
    async fn test_fs_read_file_max_bytes() {
        let temp = TempDir::new();
        temp.write_file("foo.txt", "hello");

        let pack = LocalFSToolPack::new(temp.path(), vec![with_local_fs_max_read_bytes(3)])
            .expect("create pack");
        let mut registry = ToolRegistry::new();
        pack.register_into(&mut registry);

        let call = tool_call(TOOL_FS_READ_FILE, serde_json::json!({"path": "foo.txt"}));
        let result = registry.execute(&call).await;
        assert!(result.is_err());
        assert!(result
            .error
            .unwrap_or_default()
            .contains("file exceeds max_bytes"));
    }

    #[tokio::test]
    async fn test_fs_read_file_hard_cap() {
        let temp = TempDir::new();
        temp.write_file("foo.txt", "hello");

        let pack = LocalFSToolPack::new(temp.path(), vec![with_local_fs_hard_max_read_bytes(5)])
            .expect("create pack");
        let mut registry = ToolRegistry::new();
        pack.register_into(&mut registry);

        let call = tool_call(
            TOOL_FS_READ_FILE,
            serde_json::json!({"path": "foo.txt", "max_bytes": 100}),
        );
        let result = registry.execute(&call).await;
        assert!(result.is_err());
        assert!(result
            .error
            .unwrap_or_default()
            .contains("max_bytes exceeds hard cap"));
    }

    #[tokio::test]
    async fn test_fs_list_files_max_entries() {
        let temp = TempDir::new();
        temp.write_file("a.txt", "a");
        temp.write_file("b.txt", "b");

        let pack = LocalFSToolPack::new(temp.path(), vec![]).expect("create pack");
        let mut registry = ToolRegistry::new();
        pack.register_into(&mut registry);

        let call = tool_call(
            TOOL_FS_LIST_FILES,
            serde_json::json!({"path": ".", "max_entries": 1}),
        );
        let result = registry.execute(&call).await;
        assert!(result.is_ok());
        let output = result.result.unwrap();
        let output = output.as_str().unwrap_or("");
        assert_eq!(output.lines().count(), 1);
    }

    #[tokio::test]
    async fn test_fs_list_files_path_traversal() {
        let temp = TempDir::new();
        temp.write_file("a.txt", "a");

        let pack = LocalFSToolPack::new(temp.path(), vec![]).expect("create pack");
        let mut registry = ToolRegistry::new();
        pack.register_into(&mut registry);

        let call = tool_call(TOOL_FS_LIST_FILES, serde_json::json!({"path": "../"}));
        let result = registry.execute(&call).await;
        assert!(result.is_err());
        assert!(result
            .error
            .unwrap_or_default()
            .contains("path must not escape"));
    }

    #[tokio::test]
    async fn test_fs_search_max_matches_and_ignores() {
        let temp = TempDir::new();
        temp.write_file("src/main.txt", "foo\nfoo\nfoo");
        temp.write_file(".git/secret.txt", "foo\n");

        let pack = LocalFSToolPack::new(temp.path(), vec![]).expect("create pack");
        let mut registry = ToolRegistry::new();
        pack.register_into(&mut registry);

        let call = tool_call(
            TOOL_FS_SEARCH,
            serde_json::json!({"query": "foo", "path": ".", "max_matches": 1}),
        );
        let result = registry.execute(&call).await;
        assert!(result.is_ok());
        let output = result.result.unwrap();
        let output = output.as_str().unwrap_or("");
        assert_eq!(output.lines().count(), 1);
        assert!(output.contains("src/main.txt"));
        assert!(!output.contains(".git"));
    }
}
