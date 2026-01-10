use std::collections::HashMap;
use std::path::PathBuf;

use modelrelay::{new_local_fs_tools, FunctionCall, ToolCall, ToolExecutionResult, ToolType};

#[derive(serde::Deserialize)]
struct ToolsFixtures {
    workspace: Workspace,
    tools: HashMap<String, ToolFixture>,
}

#[derive(serde::Deserialize)]
struct Workspace {
    root: String,
}

#[derive(serde::Deserialize)]
struct ToolFixture {
    #[serde(default)]
    schema_invalid: Vec<ToolCase>,
    #[serde(default)]
    behavior: Vec<ToolBehaviorCase>,
}

#[derive(serde::Deserialize)]
struct ToolCase {
    name: String,
    args: serde_json::Value,
}

#[derive(serde::Deserialize)]
struct ToolBehaviorCase {
    name: String,
    args: serde_json::Value,
    expect: ToolExpect,
}

#[derive(serde::Deserialize, Default)]
struct ToolExpect {
    error: Option<bool>,
    retryable: Option<bool>,
    output_equals: Option<String>,
    #[serde(default)]
    output_contains: Vec<String>,
    #[serde(default)]
    output_contains_any: Vec<String>,
    #[serde(default)]
    output_excludes: Vec<String>,
    #[serde(default)]
    error_contains_any: Vec<String>,
    max_lines: Option<usize>,
    line_regex: Option<String>,
}

fn conformance_tools_dir() -> Option<PathBuf> {
    if let Ok(root) = std::env::var("MODELRELAY_CONFORMANCE_DIR") {
        return Some(PathBuf::from(root).join("tools-v0"));
    }

    // sdk/rust/tests -> repo root
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..");
    let internal = repo_root
        .join("platform")
        .join("workflow")
        .join("testdata")
        .join("conformance")
        .join("tools-v0");
    if internal.join("fixtures.json").exists() {
        return Some(internal);
    }
    if is_monorepo(&repo_root) {
        panic!(
            "tools conformance fixtures missing at {} (set MODELRELAY_CONFORMANCE_DIR)",
            internal.display()
        );
    }

    None
}

fn is_monorepo(repo_root: &PathBuf) -> bool {
    if repo_root.join("go.work").exists() {
        return true;
    }
    if repo_root.join("platform").exists() {
        return true;
    }
    false
}

fn read_fixtures() -> Option<ToolsFixtures> {
    let base = conformance_tools_dir()?;
    let raw = std::fs::read_to_string(base.join("fixtures.json")).ok()?;
    serde_json::from_str(&raw).ok()
}

fn tool_call(name: &str, args: &serde_json::Value) -> ToolCall {
    ToolCall {
        id: "tc_conformance".to_string(),
        kind: ToolType::Function,
        function: Some(FunctionCall {
            name: name.to_string(),
            arguments: args.to_string(),
        }),
    }
}

fn non_empty_lines(output: &str) -> Vec<&str> {
    output
        .lines()
        .map(|line| line.trim())
        .filter(|line| !line.is_empty())
        .collect()
}

fn assert_schema_invalid(tool: &str, c: &ToolCase, result: ToolExecutionResult) {
    assert!(
        result.error.is_some(),
        "{} schema_invalid {}: expected error",
        tool,
        c.name
    );
    assert!(
        result.is_retryable,
        "{} schema_invalid {}: expected retryable error",
        tool, c.name
    );
}

fn assert_behavior(tool: &str, c: &ToolBehaviorCase, result: ToolExecutionResult) {
    if let Some(expect_error) = c.expect.error {
        if expect_error {
            assert!(
                result.error.is_some(),
                "{} behavior {}: expected error",
                tool,
                c.name
            );
        } else {
            assert!(
                result.error.is_none(),
                "{} behavior {}: unexpected error {:?}",
                tool,
                c.name,
                result.error
            );
        }
    }

    if let Some(retryable) = c.expect.retryable {
        assert_eq!(
            result.is_retryable, retryable,
            "{} behavior {}: retryable mismatch",
            tool, c.name
        );
    }

    if let Some(err) = result.error.as_ref() {
        if !c.expect.error_contains_any.is_empty() {
            let matched = c
                .expect
                .error_contains_any
                .iter()
                .any(|frag| err.contains(frag));
            assert!(
                matched,
                "{} behavior {}: error {} missing expected fragments {:?}",
                tool, c.name, err, c.expect.error_contains_any
            );
        }
        return;
    }

    let output = result
        .result
        .as_ref()
        .and_then(|value| value.as_str())
        .unwrap_or_default();

    if let Some(expected) = c.expect.output_equals.as_ref() {
        assert_eq!(
            output, expected,
            "{} behavior {}: output mismatch",
            tool, c.name
        );
    }

    for frag in c.expect.output_contains.iter() {
        assert!(
            output.contains(frag),
            "{} behavior {}: output missing {}",
            tool,
            c.name,
            frag
        );
    }

    if !c.expect.output_contains_any.is_empty() {
        let matched = c
            .expect
            .output_contains_any
            .iter()
            .any(|frag| output.contains(frag));
        assert!(
            matched,
            "{} behavior {}: output missing any of {:?}",
            tool, c.name, c.expect.output_contains_any
        );
    }

    for frag in c.expect.output_excludes.iter() {
        assert!(
            !output.contains(frag),
            "{} behavior {}: output should not include {}",
            tool,
            c.name,
            frag
        );
    }

    if let Some(max_lines) = c.expect.max_lines {
        let lines = non_empty_lines(output);
        assert!(
            lines.len() <= max_lines,
            "{} behavior {}: expected <= {} lines, got {}",
            tool,
            c.name,
            max_lines,
            lines.len()
        );
    }

    if let Some(pattern) = c.expect.line_regex.as_ref() {
        let re = regex::Regex::new(pattern).expect("regex compile");
        for line in non_empty_lines(output) {
            assert!(
                re.is_match(line),
                "{} behavior {}: line {} does not match {}",
                tool,
                c.name,
                line,
                pattern
            );
        }
    }
}

#[tokio::test]
async fn tools_conformance_local_fs() {
    let Some(fixtures) = read_fixtures() else {
        return;
    };
    let base = conformance_tools_dir().expect("conformance dir");
    let root = base.join(fixtures.workspace.root);

    let registry = new_local_fs_tools(root, []).expect("create registry");

    let read_fixture = fixtures
        .tools
        .get("fs.read_file")
        .expect("fs.read_file fixture");
    for c in read_fixture.schema_invalid.iter() {
        let result = registry.execute(&tool_call("fs.read_file", &c.args)).await;
        assert_schema_invalid("fs.read_file", c, result);
    }
    for c in read_fixture.behavior.iter() {
        let result = registry.execute(&tool_call("fs.read_file", &c.args)).await;
        assert_behavior("fs.read_file", c, result);
    }

    let list_fixture = fixtures
        .tools
        .get("fs.list_files")
        .expect("fs.list_files fixture");
    for c in list_fixture.schema_invalid.iter() {
        let result = registry.execute(&tool_call("fs.list_files", &c.args)).await;
        assert_schema_invalid("fs.list_files", c, result);
    }
    for c in list_fixture.behavior.iter() {
        let result = registry.execute(&tool_call("fs.list_files", &c.args)).await;
        assert_behavior("fs.list_files", c, result);
    }

    let search_fixture = fixtures.tools.get("fs.search").expect("fs.search fixture");
    for c in search_fixture.schema_invalid.iter() {
        let result = registry.execute(&tool_call("fs.search", &c.args)).await;
        assert_schema_invalid("fs.search", c, result);
    }
    for c in search_fixture.behavior.iter() {
        let result = registry.execute(&tool_call("fs.search", &c.args)).await;
        assert_behavior("fs.search", c, result);
    }

    let edit_fixture = fixtures.tools.get("fs.edit").expect("fs.edit fixture");
    for c in edit_fixture.schema_invalid.iter() {
        let result = registry.execute(&tool_call("fs.edit", &c.args)).await;
        assert_schema_invalid("fs.edit", c, result);
    }
    for c in edit_fixture.behavior.iter() {
        let result = registry.execute(&tool_call("fs.edit", &c.args)).await;
        assert_behavior("fs.edit", c, result);
    }
}
