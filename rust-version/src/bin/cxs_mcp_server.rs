use serde_json::{json, Map, Value};
use std::io::{self, BufRead, Write};

const PROTOCOL_VERSION: &str = "2025-06-18";

fn tool_defs() -> Value {
    json!([{
        "name": "cxs",
        "description": "Prefer over raw rg/grep for broad searches in many-file projects.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "op": {"type": "string", "enum": ["find", "files", "symbol", "json", "self_check"]},
                "args": {"type": "object"}
            },
            "required": ["op"],
            "additionalProperties": false
        }
    }])
}

fn result(id: Value, data: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": data})
}

fn error(id: Value, code: i64, msg: String, data: Option<Value>) -> Value {
    let mut e = Map::new();
    e.insert("code".to_string(), json!(code));
    e.insert("message".to_string(), json!(msg));
    if let Some(data) = data {
        e.insert("data".to_string(), data);
    }
    json!({"jsonrpc": "2.0", "id": id, "error": Value::Object(e)})
}

fn call_tool(name: &str, arguments: &Value) -> Value {
    if name == "cxs" {
        let op = arguments.get("op").and_then(Value::as_str).unwrap_or("");
        let args = arguments.get("args").unwrap_or(&Value::Null);
        if !args.is_null() && !args.is_object() {
            return json!({"status": "error", "error": "args_must_be_object"});
        }
        return cxs::call_op(op, args);
    }
    match name {
        "cxs_find" => cxs::call_op("find", arguments),
        "cxs_files" => cxs::call_op("files", arguments),
        "cxs_symbol" => cxs::call_op("symbol", arguments),
        "cxs_json" => cxs::call_op("json", arguments),
        "cxs_self_check" => cxs::call_op("self_check", arguments),
        _ => json!({"status": "error", "error": "unknown_tool", "tool": name}),
    }
}

fn handle(msg: Value) -> Option<Value> {
    let method = msg.get("method").and_then(Value::as_str).unwrap_or("");
    let rid = msg.get("id").cloned().unwrap_or(Value::Null);
    let params = msg.get("params").unwrap_or(&Value::Null);
    match method {
        "initialize" => {
            let requested = params
                .get("protocolVersion")
                .and_then(Value::as_str)
                .unwrap_or(PROTOCOL_VERSION);
            let proto = if matches!(requested, "2025-06-18" | "2025-03-26" | "2024-11-05") {
                requested
            } else {
                PROTOCOL_VERSION
            };
            Some(result(
                rid,
                json!({
                    "protocolVersion": proto,
                    "capabilities": {"tools": {"listChanged": false}},
                    "serverInfo": {"name": "cxs-rg-guard", "version": cxs::VERSION},
                    "instructions": "In large or many-file projects, prefer cxs over raw rg/grep for broad searches. For known files or small scopes, read directly. Broad search: find(files_only:true), then find(paths) for bounded line hits."
                }),
            ))
        }
        "notifications/initialized" => None,
        "ping" => Some(result(rid, json!({}))),
        "tools/list" => Some(result(rid, json!({"tools": tool_defs()}))),
        "tools/call" => {
            let name = params.get("name").and_then(Value::as_str).unwrap_or("");
            let args = params.get("arguments").unwrap_or(&Value::Null);
            let data = call_tool(name, args);
            let is_error = data.get("status").and_then(Value::as_str) == Some("error");
            Some(result(
                rid,
                json!({
                    "content": [{"type": "text", "text": cxs::json_dumps(&data, false)}],
                    "isError": is_error
                }),
            ))
        }
        _ if rid.is_null() => None,
        _ => Some(error(
            rid,
            -32601,
            format!("method not found: {method}"),
            None,
        )),
    }
}

fn main() {
    let stdin = io::stdin();
    let mut stdout = io::stdout();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { continue };
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let resp = match serde_json::from_str::<Value>(line) {
            Ok(msg) => handle(msg),
            Err(e) => Some(error(
                Value::Null,
                -32700,
                "parse error".to_string(),
                Some(json!(e.to_string())),
            )),
        };
        if let Some(resp) = resp {
            let _ = writeln!(stdout, "{}", cxs::json_dumps(&resp, false));
            let _ = stdout.flush();
        }
    }
}
