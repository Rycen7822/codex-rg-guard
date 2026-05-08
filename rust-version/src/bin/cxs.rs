use serde_json::{json, Map, Value};
use std::env;

fn push_opt(map: &mut Map<String, Value>, key: &str, value: Option<String>) {
    if let Some(v) = value {
        map.insert(key.to_string(), Value::String(v));
    }
}

fn push_list(map: &mut Map<String, Value>, key: &str, values: Vec<String>) {
    if !values.is_empty() {
        map.insert(key.to_string(), json!(values));
    }
}

fn parse_num(s: Option<String>) -> Option<Value> {
    s.and_then(|x| x.parse::<usize>().ok()).map(Value::from)
}

fn parse_float(s: Option<String>) -> Option<Value> {
    s.and_then(|x| x.parse::<f64>().ok()).map(Value::from)
}

fn parse_common(tokens: &[String], i: &mut usize, map: &mut Map<String, Value>) -> bool {
    match tokens.get(*i).map(String::as_str) {
        Some("--root") => {
            *i += 1;
            push_opt(map, "root", tokens.get(*i).cloned());
            true
        }
        Some("--scope") => {
            *i += 1;
            append_array(map, "scopes", tokens.get(*i).cloned());
            true
        }
        Some("--path") => {
            *i += 1;
            append_array(map, "paths", tokens.get(*i).cloned());
            true
        }
        Some("-g") | Some("--glob") => {
            *i += 1;
            append_array(map, "include_globs", tokens.get(*i).cloned());
            true
        }
        Some("--exclude") => {
            *i += 1;
            append_array(map, "exclude_globs", tokens.get(*i).cloned());
            true
        }
        Some("--case-sensitive") => {
            map.insert("case_sensitive".to_string(), Value::Bool(true));
            true
        }
        _ => false,
    }
}

fn append_array(map: &mut Map<String, Value>, key: &str, value: Option<String>) {
    let Some(value) = value else { return };
    let entry = map
        .entry(key.to_string())
        .or_insert_with(|| Value::Array(vec![]));
    if let Value::Array(xs) = entry {
        xs.push(Value::String(value));
    }
}

fn parse_filters(values: Vec<String>) -> Map<String, Value> {
    let mut out = Map::new();
    for item in values {
        if let Some((k, v)) = item.split_once('=') {
            out.insert(k.to_string(), Value::String(v.to_string()));
        }
    }
    out
}

fn usage() {
    eprintln!("usage: cxs-rs [--pretty] <find|files|symbol|json|self-check> ...");
}

fn main() {
    let mut raw: Vec<String> = env::args().skip(1).collect();
    let pretty = raw.iter().any(|a| a == "--pretty");
    raw.retain(|a| a != "--pretty");
    if raw.is_empty() {
        usage();
        std::process::exit(2);
    }
    let cmd = raw.remove(0);
    let mut args = Map::new();
    let op = match cmd.as_str() {
        "find" => {
            let mut terms = Vec::new();
            let mut positional = Vec::new();
            let mut i = 0;
            while i < raw.len() {
                if parse_common(&raw, &mut i, &mut args) {
                    i += 1;
                    continue;
                }
                match raw[i].as_str() {
                    "--term" => {
                        i += 1;
                        if let Some(v) = raw.get(i) {
                            terms.push(v.clone());
                        }
                    }
                    "--match" => {
                        i += 1;
                        push_opt(&mut args, "match", raw.get(i).cloned());
                    }
                    "--regex" => {
                        args.insert("mode".to_string(), json!("regex"));
                    }
                    "--max-hits" => {
                        i += 1;
                        if let Some(v) = parse_num(raw.get(i).cloned()) {
                            args.insert("max_total_hits".to_string(), v);
                        }
                    }
                    "--per-file" => {
                        i += 1;
                        if let Some(v) = parse_num(raw.get(i).cloned()) {
                            args.insert("max_hits_per_file".to_string(), v);
                        }
                    }
                    "--line-chars" => {
                        i += 1;
                        if let Some(v) = parse_num(raw.get(i).cloned()) {
                            args.insert("max_line_chars".to_string(), v);
                        }
                    }
                    "--timeout" => {
                        i += 1;
                        if let Some(v) = parse_float(raw.get(i).cloned()) {
                            args.insert("timeout_seconds".to_string(), v);
                        }
                    }
                    "--files-only" => {
                        args.insert("files_only".to_string(), Value::Bool(true));
                    }
                    "--offset" => {
                        i += 1;
                        if let Some(v) = parse_num(raw.get(i).cloned()) {
                            args.insert("offset".to_string(), v);
                        }
                    }
                    other if other.starts_with('-') => {}
                    other => positional.push(other.to_string()),
                }
                i += 1;
            }
            if let Some(q) = positional.first() {
                args.insert("query".to_string(), Value::String(q.clone()));
            }
            push_list(&mut args, "terms", terms);
            "find"
        }
        "files" => {
            let mut positional = Vec::new();
            let mut i = 0;
            while i < raw.len() {
                if parse_common(&raw, &mut i, &mut args) {
                    i += 1;
                    continue;
                }
                match raw[i].as_str() {
                    "--regex" => {
                        args.insert("mode".to_string(), json!("regex"));
                    }
                    "--max-files" => {
                        i += 1;
                        if let Some(v) = parse_num(raw.get(i).cloned()) {
                            args.insert("max_files".to_string(), v);
                        }
                    }
                    "--timeout" => {
                        i += 1;
                        if let Some(v) = parse_float(raw.get(i).cloned()) {
                            args.insert("timeout_seconds".to_string(), v);
                        }
                    }
                    other if other.starts_with('-') => {}
                    other => positional.push(other.to_string()),
                }
                i += 1;
            }
            if let Some(q) = positional.first() {
                args.insert("query".to_string(), Value::String(q.clone()));
            }
            "files"
        }
        "symbol" => {
            let mut positional = Vec::new();
            let mut i = 0;
            while i < raw.len() {
                if parse_common(&raw, &mut i, &mut args) {
                    i += 1;
                    continue;
                }
                match raw[i].as_str() {
                    "--max-hits" => {
                        i += 1;
                        if let Some(v) = parse_num(raw.get(i).cloned()) {
                            args.insert("max_total_hits".to_string(), v);
                        }
                    }
                    "--timeout" => {
                        i += 1;
                        if let Some(v) = parse_float(raw.get(i).cloned()) {
                            args.insert("timeout_seconds".to_string(), v);
                        }
                    }
                    other if other.starts_with('-') => {}
                    other => positional.push(other.to_string()),
                }
                i += 1;
            }
            if let Some(name) = positional.first() {
                args.insert("name".to_string(), Value::String(name.clone()));
            }
            "symbol"
        }
        "json" => {
            let mut positional = Vec::new();
            let mut terms = Vec::new();
            let mut filters = Vec::new();
            let mut i = 0;
            while i < raw.len() {
                if parse_common(&raw, &mut i, &mut args) {
                    i += 1;
                    continue;
                }
                match raw[i].as_str() {
                    "--term" => {
                        i += 1;
                        if let Some(v) = raw.get(i) {
                            terms.push(v.clone());
                        }
                    }
                    "--filter" => {
                        i += 1;
                        if let Some(v) = raw.get(i) {
                            filters.push(v.clone());
                        }
                    }
                    "--field" => {
                        i += 1;
                        append_array(&mut args, "fields", raw.get(i).cloned());
                    }
                    "--omit-field" => {
                        i += 1;
                        append_array(&mut args, "omit_fields", raw.get(i).cloned());
                    }
                    "--match" => {
                        i += 1;
                        push_opt(&mut args, "match", raw.get(i).cloned());
                    }
                    "--search-large-fields" => {
                        args.insert("search_large_fields".to_string(), Value::Bool(true));
                    }
                    "--allow-large-files" => {
                        args.insert("allow_large_files".to_string(), Value::Bool(true));
                    }
                    "--max-records" => {
                        i += 1;
                        if let Some(v) = parse_num(raw.get(i).cloned()) {
                            args.insert("max_records".to_string(), v);
                        }
                    }
                    "--max-files" => {
                        i += 1;
                        if let Some(v) = parse_num(raw.get(i).cloned()) {
                            args.insert("max_files".to_string(), v);
                        }
                    }
                    "--timeout" => {
                        i += 1;
                        if let Some(v) = parse_float(raw.get(i).cloned()) {
                            args.insert("timeout_seconds".to_string(), v);
                        }
                    }
                    other if other.starts_with('-') => {}
                    other => positional.push(other.to_string()),
                }
                i += 1;
            }
            if let Some(q) = positional.first() {
                args.insert("query".to_string(), Value::String(q.clone()));
            }
            push_list(&mut args, "terms", terms);
            let filters = parse_filters(filters);
            if !filters.is_empty() {
                args.insert("filters".to_string(), Value::Object(filters));
            }
            "json"
        }
        "self-check" => {
            let mut i = 0;
            while i < raw.len() {
                if raw[i] == "--root" {
                    i += 1;
                    push_opt(&mut args, "root", raw.get(i).cloned());
                }
                i += 1;
            }
            "self_check"
        }
        _ => {
            usage();
            std::process::exit(2);
        }
    };
    let res = cxs::call_op(op, &Value::Object(args));
    println!("{}", cxs::json_dumps(&res, pretty));
    std::process::exit(
        if res.get("status").and_then(Value::as_str) == Some("error") {
            1
        } else {
            0
        },
    );
}
