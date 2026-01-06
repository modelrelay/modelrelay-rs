//! Lightweight bash tokenizer used for policy evaluation.

use std::path::Path;

/// Tokenized representation of a bash command string.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BashTokens {
    /// Primary command (normalized, e.g., "/usr/bin/git" -> "git").
    pub primary_command: String,
    /// All commands in chain (for pipes, ;, &&, ||).
    pub all_commands: Vec<String>,
    /// Structural flags.
    pub has_pipe: bool,
    pub has_chain: bool,
    pub has_redirect: bool,
    pub has_subshell: bool,
    pub has_eval: bool,
    pub has_dangerous_redirect: bool,
    /// Raw arguments for the primary command.
    pub primary_args: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Operator {
    Pipe,
    Chain,
    Redirect,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Token {
    Word(String),
    Operator(Operator),
    RedirectTarget(String),
}

#[derive(Default)]
struct LexFlags {
    has_pipe: bool,
    has_chain: bool,
    has_redirect: bool,
    has_subshell: bool,
    has_dangerous_redirect: bool,
}

/// Tokenize a bash command string with lightweight detection and normalization.
pub fn tokenize_bash(command: &str) -> BashTokens {
    let (tokens, mut flags) = lex_tokens(command);

    let mut all_commands = Vec::new();
    let mut primary_command = String::new();
    let mut primary_args = Vec::new();
    let mut current_command: Option<String> = None;
    let mut current_args: Vec<String> = Vec::new();

    for token in tokens.iter() {
        match token {
            Token::Word(word) => {
                let normalized = normalize_word(word);
                if current_command.is_none() {
                    if is_env_assignment(&normalized) {
                        continue;
                    }
                    let cmd = normalize_command(&normalized);
                    if !cmd.is_empty() {
                        current_command = Some(cmd);
                    }
                } else {
                    current_args.push(normalized);
                }
            }
            Token::Operator(op) => match op {
                Operator::Pipe | Operator::Chain => {
                    if let Some(cmd) = current_command.take() {
                        if primary_command.is_empty() {
                            primary_command = cmd.clone();
                            primary_args = current_args.clone();
                        }
                        all_commands.push(cmd);
                    }
                    current_args = Vec::new();
                }
                Operator::Redirect => {}
            },
            Token::RedirectTarget(_) => {}
        }
    }

    if let Some(cmd) = current_command.take() {
        if primary_command.is_empty() {
            primary_command = cmd.clone();
            primary_args = current_args.clone();
        }
        all_commands.push(cmd);
    }

    let mut has_eval = false;
    for cmd in all_commands.iter() {
        if is_eval_command(cmd) {
            has_eval = true;
            break;
        }
    }

    // Detect bash -c subshell usage.
    if !flags.has_subshell && detect_shell_subshell(command) {
        flags.has_subshell = true;
    }

    BashTokens {
        primary_command,
        all_commands,
        has_pipe: flags.has_pipe,
        has_chain: flags.has_chain,
        has_redirect: flags.has_redirect,
        has_subshell: flags.has_subshell,
        has_eval,
        has_dangerous_redirect: flags.has_dangerous_redirect,
        primary_args,
    }
}

fn lex_tokens(input: &str) -> (Vec<Token>, LexFlags) {
    let mut tokens = Vec::new();
    let mut flags = LexFlags::default();

    let mut buf = String::new();
    let mut in_single = false;
    let mut in_double = false;
    let mut escape = false;
    let mut expect_redirect_target = false;

    let mut chars = input.chars().peekable();
    while let Some(c) = chars.next() {
        if escape {
            buf.push(c);
            escape = false;
            continue;
        }

        if in_single {
            if c == '\'' {
                in_single = false;
            } else {
                buf.push(c);
            }
            continue;
        }

        if in_double {
            if c == '"' {
                in_double = false;
                continue;
            }
            if c == '$' {
                if let Some('(') = chars.peek() {
                    flags.has_subshell = true;
                }
            } else if c == '`' {
                flags.has_subshell = true;
            }
            if c == '\\' {
                if let Some(next) = chars.peek() {
                    if matches!(next, '"' | '\\' | '$' | '`') {
                        buf.push(*next);
                        chars.next();
                        continue;
                    }
                }
            }
            buf.push(c);
            continue;
        }

        // Not in quotes
        if c == '\\' {
            escape = true;
            continue;
        }

        if c == '\'' {
            in_single = true;
            continue;
        }

        if c == '"' {
            in_double = true;
            continue;
        }

        if c == '#' {
            if buf.is_empty() {
                break;
            }
            buf.push(c);
            continue;
        }

        if c.is_ascii_whitespace() {
            expect_redirect_target =
                flush_word(&mut buf, &mut tokens, expect_redirect_target, &mut flags);
            if c == '\n' {
                flags.has_chain = true;
                tokens.push(Token::Operator(Operator::Chain));
            }
            continue;
        }

        if c == '$' {
            if let Some('(') = chars.peek() {
                flags.has_subshell = true;
                expect_redirect_target =
                    flush_word(&mut buf, &mut tokens, expect_redirect_target, &mut flags);
                chars.next();
                continue;
            }
            buf.push(c);
            continue;
        }

        if c == '`' {
            flags.has_subshell = true;
            expect_redirect_target =
                flush_word(&mut buf, &mut tokens, expect_redirect_target, &mut flags);
            continue;
        }

        if c.is_ascii_digit() && buf.is_empty() {
            if let Some(next) = chars.peek() {
                if *next == '>' || *next == '<' {
                    flags.has_redirect = true;
                    let _ = flush_word(&mut buf, &mut tokens, expect_redirect_target, &mut flags);
                    let op = *next;
                    chars.next();
                    if op == '>' {
                        if let Some('>') = chars.peek() {
                            chars.next();
                        }
                    }
                    expect_redirect_target = true;
                    tokens.push(Token::Operator(Operator::Redirect));
                    continue;
                }
            }
        }

        match c {
            '|' => {
                expect_redirect_target =
                    flush_word(&mut buf, &mut tokens, expect_redirect_target, &mut flags);
                if let Some('|') = chars.peek() {
                    flags.has_chain = true;
                    chars.next();
                    tokens.push(Token::Operator(Operator::Chain));
                } else {
                    flags.has_pipe = true;
                    if let Some('&') = chars.peek() {
                        chars.next();
                    }
                    tokens.push(Token::Operator(Operator::Pipe));
                }
            }
            ';' => {
                expect_redirect_target =
                    flush_word(&mut buf, &mut tokens, expect_redirect_target, &mut flags);
                flags.has_chain = true;
                tokens.push(Token::Operator(Operator::Chain));
            }
            '&' => {
                if let Some('>') = chars.peek() {
                    let _ = flush_word(&mut buf, &mut tokens, expect_redirect_target, &mut flags);
                    flags.has_redirect = true;
                    chars.next();
                    if let Some('>') = chars.peek() {
                        chars.next();
                    }
                    expect_redirect_target = true;
                    tokens.push(Token::Operator(Operator::Redirect));
                } else if let Some('&') = chars.peek() {
                    expect_redirect_target =
                        flush_word(&mut buf, &mut tokens, expect_redirect_target, &mut flags);
                    flags.has_chain = true;
                    chars.next();
                    tokens.push(Token::Operator(Operator::Chain));
                } else {
                    expect_redirect_target =
                        flush_word(&mut buf, &mut tokens, expect_redirect_target, &mut flags);
                    flags.has_chain = true;
                    tokens.push(Token::Operator(Operator::Chain));
                }
            }
            '>' | '<' => {
                let _ = flush_word(&mut buf, &mut tokens, expect_redirect_target, &mut flags);
                flags.has_redirect = true;
                if c == '>' {
                    if let Some('>') = chars.peek() {
                        chars.next();
                    }
                }
                expect_redirect_target = true;
                tokens.push(Token::Operator(Operator::Redirect));
            }
            _ => buf.push(c),
        }
    }

    flush_word(&mut buf, &mut tokens, expect_redirect_target, &mut flags);

    (tokens, flags)
}

fn flush_word(
    buf: &mut String,
    tokens: &mut Vec<Token>,
    mut expect_redirect_target: bool,
    flags: &mut LexFlags,
) -> bool {
    if buf.is_empty() {
        return expect_redirect_target;
    }
    let word = std::mem::take(buf);
    if expect_redirect_target {
        if is_dangerous_redirect_target(&word) {
            flags.has_dangerous_redirect = true;
        }
        tokens.push(Token::RedirectTarget(word));
        expect_redirect_target = false;
    } else {
        tokens.push(Token::Word(word));
    }
    expect_redirect_target
}

fn normalize_word(word: &str) -> String {
    word.trim().to_string()
}

fn normalize_command(word: &str) -> String {
    let trimmed = word.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let path = Path::new(trimmed);
    if let Some(name) = path.file_name() {
        name.to_string_lossy().to_string()
    } else {
        trimmed.to_string()
    }
}

fn is_env_assignment(word: &str) -> bool {
    let (name, _) = match word.split_once('=') {
        Some((name, value)) if !value.is_empty() || word.ends_with('=') => (name, value),
        Some((name, _)) if !name.is_empty() => (name, ""),
        _ => return false,
    };

    if name.is_empty() {
        return false;
    }

    let base = match name.strip_suffix('+') {
        Some(stripped) => stripped,
        None => name,
    };

    if base.is_empty() {
        return false;
    }

    let mut chars = base.chars();
    let first = match chars.next() {
        Some(c) => c,
        None => return false,
    };
    if !(first == '_' || first.is_ascii_alphabetic()) {
        return false;
    }
    for c in chars {
        if !(c == '_' || c.is_ascii_alphanumeric()) {
            return false;
        }
    }

    true
}

fn is_eval_command(cmd: &str) -> bool {
    matches!(cmd, "eval" | "exec")
}

fn is_shell_command(cmd: &str) -> bool {
    matches!(cmd, "sh" | "bash" | "zsh" | "dash" | "ksh")
}

fn detect_shell_subshell(command: &str) -> bool {
    // Lightweight detection for patterns like: bash -c "..."
    let tokens = lex_simple_words(command);
    let mut iter = tokens.iter();
    while let Some(token) = iter.next() {
        let normalized = normalize_command(token);
        if is_shell_command(&normalized) {
            if let Some(arg) = iter.next() {
                if arg == "-c" {
                    return true;
                }
            }
        }
    }
    false
}

fn lex_simple_words(input: &str) -> Vec<String> {
    // Minimal word splitter for detecting "bash -c" patterns.
    let mut words = Vec::new();
    let mut buf = String::new();
    let mut in_single = false;
    let mut in_double = false;
    let mut escape = false;

    for c in input.chars() {
        if escape {
            buf.push(c);
            escape = false;
            continue;
        }

        if in_single {
            if c == '\'' {
                in_single = false;
            } else {
                buf.push(c);
            }
            continue;
        }

        if in_double {
            if c == '"' {
                in_double = false;
            } else if c == '\\' {
                escape = true;
            } else {
                buf.push(c);
            }
            continue;
        }

        match c {
            '\\' => escape = true,
            '\'' => in_single = true,
            '"' => in_double = true,
            c if c.is_ascii_whitespace() => {
                if !buf.is_empty() {
                    words.push(buf.clone());
                    buf.clear();
                }
            }
            _ => buf.push(c),
        }
    }

    if !buf.is_empty() {
        words.push(buf);
    }

    words
}

fn is_dangerous_redirect_target(target: &str) -> bool {
    let trimmed = target.trim();
    trimmed.starts_with("/etc/") || trimmed.starts_with("/root/") || trimmed == "/etc/passwd"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokenize_normalizes_primary_command() {
        let tokens = tokenize_bash("/usr/bin/git status");
        assert_eq!(tokens.primary_command, "git");
        assert_eq!(tokens.all_commands, vec!["git"]);
    }

    #[test]
    fn tokenize_handles_quoted_command() {
        let tokens = tokenize_bash("g'i't status");
        assert_eq!(tokens.primary_command, "git");
        assert_eq!(tokens.primary_args, vec!["status"]);
    }

    #[test]
    fn tokenize_detects_pipe_chain() {
        let tokens = tokenize_bash("cmd1 | cmd2; cmd3");
        assert!(tokens.has_pipe);
        assert!(tokens.has_chain);
        assert_eq!(tokens.all_commands, vec!["cmd1", "cmd2", "cmd3"]);
    }

    #[test]
    fn tokenize_detects_subshell_and_eval() {
        let tokens = tokenize_bash("$(echo rm) file");
        assert!(tokens.has_subshell);

        let tokens = tokenize_bash("eval \"rm file\"");
        assert!(tokens.has_eval);
        assert_eq!(tokens.primary_command, "eval");
    }

    #[test]
    fn tokenize_detects_dangerous_redirect() {
        let tokens = tokenize_bash("echo hi > /etc/passwd");
        assert!(tokens.has_redirect);
        assert!(tokens.has_dangerous_redirect);
    }

    #[test]
    fn tokenize_detects_bash_dash_c_subshell() {
        let tokens = tokenize_bash("bash -c 'echo hi'");
        assert!(tokens.has_subshell);
    }

    #[test]
    fn tokenize_skips_leading_env_assignments() {
        let tokens = tokenize_bash("FOO=bar npm run build");
        assert_eq!(tokens.primary_command, "npm");
        assert_eq!(tokens.primary_args, vec!["run", "build"]);
        assert_eq!(tokens.all_commands, vec!["npm"]);
    }

    #[test]
    fn tokenize_detects_subshell_in_double_quotes() {
        let tokens = tokenize_bash("echo \"$(rm -rf /)\"");
        assert!(tokens.has_subshell);
    }
}
