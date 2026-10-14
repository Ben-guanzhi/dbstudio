use super::*;
use serde_json::json;

#[test]
fn parses_request_with_params_shapes() {
    let with_params: Request = serde_json::from_str(
        r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"connect"}}"#,
    )
    .unwrap();
    assert_eq!(with_params.method, "tools/call");
    assert_eq!(with_params.params["name"], "connect");

    let no_params: Request =
        serde_json::from_str(r#"{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}"#)
            .unwrap();
    assert_eq!(no_params.method, "tools/list");
}

#[test]
fn parses_notification_without_id() {
    let notif: Request =
        serde_json::from_str(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#).unwrap();
    assert!(notif.id.is_none());
    assert_eq!(notif.method, "notifications/initialized");
}

#[test]
fn success_response_serializes_without_error_field() {
    let resp = Response::success(Some(json!(1)), json!({"ok": true}));
    let text = serde_json::to_string(&resp).unwrap();
    let value: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(value["jsonrpc"], "2.0");
    assert_eq!(value["id"], 1);
    assert_eq!(value["result"]["ok"], true);
    assert!(value.get("error").is_none(), "error must be omitted");
}

#[test]
fn error_response_serializes_without_result_field() {
    let resp = Response::error(Some(json!(2)), error_codes::METHOD_NOT_FOUND, "nope");
    let text = serde_json::to_string(&resp).unwrap();
    let value: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(value["error"]["code"], -32601);
    assert_eq!(value["error"]["message"], "nope");
    assert!(value.get("result").is_none(), "result must be omitted");
}
