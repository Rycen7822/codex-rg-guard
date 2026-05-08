use serde_json::{json, Value};
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("cxs-rs-test-{}-{stamp}", std::process::id()));
        fs::create_dir_all(root.join("src")).unwrap();
        fs::create_dir_all(root.join("docs")).unwrap();
        fs::create_dir_all(root.join("data")).unwrap();
        fs::create_dir_all(root.join("resources/code/github")).unwrap();
        fs::create_dir_all(root.join("experiments/runs/r1")).unwrap();
        fs::write(
            root.join("src/app.py"),
            "def train_loop(x):\n    return x\n\nclass Trainer:\n    pass\n",
        )
        .unwrap();
        fs::write(
            root.join("docs/README.md"),
            format!("hello\nneedle {}\n", "x".repeat(1000)),
        )
        .unwrap();
        fs::write(
            root.join("resources/code/github/vendor.txt"),
            "needle vendor\n",
        )
        .unwrap();
        fs::write(
            root.join("data/records.jsonl"),
            format!(
                "{}\n{}\n",
                json!({"doc_id":"doc-1","token_count":12,"text":"A".repeat(10000),"kind":"summary"}),
                json!({"doc_id":"doc-2","token_count":99,"text":"needle hidden in large text","kind":"raw"})
            ),
        )
        .unwrap();
        fs::write(
            root.join("experiments/runs/r1/metrics.jsonl"),
            json!({"run_id":"r1","acc":0.9}).to_string(),
        )
        .unwrap();
        Self { root }
    }

    fn root_arg(&self) -> Value {
        json!(self.root.to_string_lossy().to_string())
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn find_budget_and_excludes() {
    let fx = Fixture::new();
    let res = cxs::cxs_find(
        &json!({"query":"needle","root":fx.root_arg(),"scopes":["docs"],"max_line_chars":60}),
    );
    assert!(!res["hits"].as_array().unwrap().is_empty());
    assert!(res["hits"][0]["snippet"].as_str().unwrap().chars().count() <= 80);
    let res2 =
        cxs::cxs_find(&json!({"query":"vendor","root":fx.root_arg(),"scopes":["docs","src"]}));
    assert_eq!(res2["hits"].as_array().unwrap().len(), 0);
}

#[test]
fn find_defaults_come_from_budget() {
    let fx = Fixture::new();
    fs::write(fx.root.join("src/budget.py"), "budget-key\n".repeat(25)).unwrap();
    let res = cxs::cxs_find(
        &json!({"query":"budget-key","root":fx.root_arg(),"paths":["src/budget.py"]}),
    );
    assert_eq!(res["hits"].as_array().unwrap().len(), 25);
}

#[test]
fn find_files_only_and_pagination() {
    let fx = Fixture::new();
    fs::write(fx.root.join("src/multi.py"), "alpha\nbeta\n").unwrap();
    fs::write(fx.root.join("src/one.py"), "alpha\n").unwrap();
    let res = cxs::cxs_find(
        &json!({"query":"needle","root":fx.root_arg(),"scopes":["docs"],"files_only":true}),
    );
    assert_eq!(res["files_only"], true);
    assert!(res["files"]
        .as_array()
        .unwrap()
        .contains(&json!({"file":"docs/README.md"})));
    assert!(res.get("hits").is_none());
    assert_eq!(res["next"][0]["op"], "find");

    let res2 = cxs::cxs_find(
        &json!({"terms":["alpha","beta"],"match":"all","root":fx.root_arg(),"scopes":["src"],"files_only":true}),
    );
    assert!(res2["files"]
        .as_array()
        .unwrap()
        .contains(&json!({"file":"src/multi.py"})));
    assert!(!res2["files"]
        .as_array()
        .unwrap()
        .contains(&json!({"file":"src/one.py"})));

    for i in 0..4 {
        fs::write(fx.root.join(format!("src/page{i}.py")), "page-key\n").unwrap();
    }
    let page1 = cxs::cxs_find(
        &json!({"query":"page-key","root":fx.root_arg(),"scopes":["src"],"files_only":true,"max_total_hits":2}),
    );
    assert_eq!(page1["files"].as_array().unwrap().len(), 2);
    assert_eq!(page1["status"], "truncated");
    assert_eq!(page1["stats"]["has_more"], true);
    assert_eq!(page1["next_page"]["offset"], 2);
    assert!(page1.get("next_meta").is_some());
    let page2 = cxs::cxs_find(
        &json!({"query":"page-key","root":fx.root_arg(),"scopes":["src"],"files_only":true,"max_total_hits":2,"offset":page1["next_page"]["offset"]}),
    );
    assert_eq!(page2["files"].as_array().unwrap().len(), 2);
    assert_ne!(page1["files"], page2["files"]);
}

#[test]
fn find_line_limits_and_missing_paths() {
    let fx = Fixture::new();
    let res = cxs::cxs_find(&json!({"query":"return","root":fx.root_arg(),"paths":["src/app.py"]}));
    assert_eq!(res["hits"].as_array().unwrap().len(), 1);
    assert_eq!(res["hits"][0]["file"], "src/app.py");
    assert_eq!(res["hits"][0]["line"], 2);
    assert!(res.get("next").is_none());

    fs::write(
        fx.root.join("src/many.py"),
        ("needle ".to_owned() + &"x".repeat(200) + "\n").repeat(10),
    )
    .unwrap();
    let limited = cxs::cxs_find(
        &json!({"query":"needle","root":fx.root_arg(),"paths":["src/many.py"],"max_hits_per_file":3}),
    );
    assert_eq!(limited["status"], "truncated");
    assert!(limited["stats"]["limit_reasons"]
        .as_array()
        .unwrap()
        .contains(&json!("max_hits_per_file")));
    assert_eq!(limited["limited_files"][0]["file"], "src/many.py");

    let missing =
        cxs::cxs_find(&json!({"query":"needle","root":fx.root_arg(),"paths":["missing.md"]}));
    assert_eq!(missing["status"], "error");
    assert_eq!(missing["error"], "no_valid_paths");
}

#[test]
fn files_symbol_and_json() {
    let fx = Fixture::new();
    let files = cxs::cxs_files(&json!({"query":"app","root":fx.root_arg(),"scopes":["src"]}));
    assert!(files["files"]
        .as_array()
        .unwrap()
        .iter()
        .any(|x| x["file"] == "src/app.py"));

    let sym = cxs::cxs_symbol(&json!({"name":"train_loop","root":fx.root_arg()}));
    assert!(sym["hits"]
        .as_array()
        .unwrap()
        .iter()
        .any(|h| h["snippet"].as_str().unwrap().contains("def train_loop")));

    let hidden = cxs::cxs_json(
        &json!({"query":"needle hidden","root":fx.root_arg(),"paths":["data/records.jsonl"]}),
    );
    assert_eq!(hidden["hits"].as_array().unwrap().len(), 0);
    let hidden2 = cxs::cxs_json(
        &json!({"query":"needle hidden","root":fx.root_arg(),"paths":["data/records.jsonl"],"search_large_fields":true}),
    );
    assert_eq!(hidden2["hits"].as_array().unwrap().len(), 1);
    let fields = cxs::cxs_json(
        &json!({"filters":{"doc_id":"doc-1"},"fields":["doc_id","token_count","text"],"root":fx.root_arg(),"paths":["data/records.jsonl"]}),
    );
    assert_eq!(fields["hits"][0]["record"]["doc_id"], "doc-1");
    assert!(
        fields["hits"][0]["record"]["text"]
            .as_str()
            .unwrap()
            .chars()
            .count()
            <= 320
    );

    let default_runs =
        cxs::cxs_json(&json!({"query":"r1","root":fx.root_arg(),"scopes":["analysis"]}));
    assert_eq!(default_runs["hits"].as_array().unwrap().len(), 0);
    let runs = cxs::cxs_json(&json!({"query":"r1","root":fx.root_arg(),"scopes":["runs"]}));
    assert_eq!(runs["hits"].as_array().unwrap().len(), 1);
}

#[test]
fn mcp_single_tool_contract() {
    let server = env!("CARGO_BIN_EXE_cxs-mcp-server-rs");
    let mut proc = Command::new(server)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = proc.stdin.take().unwrap();
    let stdout = proc.stdout.take().unwrap();
    writeln!(stdin, "{}", json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"t","version":"0"}}})).unwrap();
    writeln!(
        stdin,
        "{}",
        json!({"jsonrpc":"2.0","method":"notifications/initialized"})
    )
    .unwrap();
    writeln!(
        stdin,
        "{}",
        json!({"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}})
    )
    .unwrap();
    stdin.flush().unwrap();
    drop(stdin);
    let mut lines = BufReader::new(stdout).lines();
    let r1: Value = serde_json::from_str(&lines.next().unwrap().unwrap()).unwrap();
    let r2: Value = serde_json::from_str(&lines.next().unwrap().unwrap()).unwrap();
    let _ = proc.kill();
    let _ = proc.wait();
    let text = serde_json::to_string(&r2).unwrap();
    assert!(r1["result"]["instructions"]
        .as_str()
        .unwrap()
        .contains("find(files_only:true)"));
    assert!(text.contains("\"cxs\""));
    assert!(!text.contains("\"read\""));
    assert!(!text.contains("cxs_find"));
    assert!(text.len() < 900);
}

#[test]
fn cli_and_rg_shim_work() {
    let fx = Fixture::new();
    let cxs_bin = env!("CARGO_BIN_EXE_cxs-rs");
    let cli = Command::new(cxs_bin)
        .args([
            "find",
            "needle",
            "--root",
            fx.root.to_str().unwrap(),
            "--scope",
            "docs",
            "--files-only",
        ])
        .output()
        .unwrap();
    assert!(cli.status.success());
    let cli_json: Value = serde_json::from_slice(&cli.stdout).unwrap();
    assert_eq!(cli_json["files_only"], true);
    assert!(cli_json["files"]
        .as_array()
        .unwrap()
        .contains(&json!({"file":"docs/README.md"})));

    let shim_bin = env!("CARGO_BIN_EXE_rg");
    let shim = Command::new(shim_bin)
        .current_dir(&fx.root)
        .args(["-n", "return", fx.root.join("src/app.py").to_str().unwrap()])
        .output()
        .unwrap();
    assert!(shim.status.success());
    let shim_json: Value = serde_json::from_slice(&shim.stdout).unwrap();
    assert_eq!(shim_json["hits"][0]["file"], "src/app.py");

    let self_check = cxs::cxs_self_check(&json!({"root":fx.root_arg()}));
    assert_eq!(self_check["status"], "ok");
    assert_eq!(self_check["runtime"], "rust");
}
