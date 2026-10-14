use super::*;
use serde_json::json;

#[test]
fn tool_serializes_with_input_schema() {
    let tool = Tool {
        name: "execute".into(),
        description: "Run SQL".into(),
        input_schema: json!({
            "type": "object",
            "properties": { "sql": { "type": "string" } },
            "required": ["sql"]
        }),
    };
    let value = serde_json::to_value(&tool).unwrap();
    assert_eq!(value["name"], "execute");
    assert_eq!(value["inputSchema"]["properties"]["sql"]["type"], "string");
}

#[test]
fn call_tool_result_success_omits_is_error() {
    let result = CallToolResult::success("rows: 3".into());
    let value = serde_json::to_value(&result).unwrap();
    assert_eq!(value["content"][0]["type"], "text");
    assert_eq!(value["content"][0]["text"], "rows: 3");
    assert!(value.get("isError").is_none());
}

#[test]
fn call_tool_result_error_marks_is_error() {
    let result = CallToolResult::error("boom".into());
    let value = serde_json::to_value(&result).unwrap();
    assert_eq!(value["isError"], true);
    assert_eq!(value["content"][0]["text"], "boom");
}
