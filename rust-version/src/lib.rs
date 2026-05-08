use regex::{Regex, RegexBuilder};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, HashSet};
use std::env;
use std::fs;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{sync_channel, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

pub const VERSION: &str = "0.2.4";

pub const DEFAULT_SCOPES: &[&str] = &["docs", "src", "tests", "config", "analysis"];

const DEFAULT_EXCLUDES: &[&str] = &[
    "**/.git/**",
    "**/.hg/**",
    "**/.svn/**",
    "**/.venv/**",
    "**/venv/**",
    "**/node_modules/**",
    "**/__pycache__/**",
    "**/resources/code/github/**",
    "**/resources/cache/**",
    "**/resources/models/**",
    "**/experiments/runs/**",
    "**/runs/**",
    "**/checkpoints/**",
    "**/wandb/**",
    "**/*.pt",
    "**/*.pth",
    "**/*.safetensors",
    "**/*.onnx",
    "**/*.bin",
    "**/*.gguf",
    "**/*.parquet",
    "**/*.arrow",
    "**/*.npz",
    "**/*.npy",
    "**/*.pkl",
    "**/*.pickle",
    "**/*.zip",
    "**/*.tar",
    "**/*.tar.gz",
    "**/*.tgz",
    "**/*.7z",
    "**/uv.lock",
    "**/package-lock.json",
    "**/pnpm-lock.yaml",
    "**/yarn.lock",
];

const DATA_GLOBS: &[&str] = &[
    "**/*.json",
    "**/*.jsonl",
    "**/*.ndjson",
    "**/*.csv",
    "**/*.tsv",
];

#[derive(Clone, Copy)]
pub struct Budget {
    // Public defaults. Omitted user arguments use these values, so normal edits
    // to the search budget should start here.
    pub max_total_hits: usize,
    pub max_hits_per_file: usize,
    pub max_line_chars: usize,
    pub max_total_chars: usize,
    pub timeout_seconds: f64,
    pub process_output_bytes: usize,

    // Clamp bounds for user-supplied budget arguments. These prevent a single
    // call from turning a bounded search into a raw repository dump.
    pub min_total_hits: usize,
    pub max_total_hits_limit: usize,
    pub min_hits_per_file: usize,
    pub max_hits_per_file_limit: usize,
    pub min_line_chars: usize,
    pub max_line_chars_limit: usize,
    pub min_total_chars: usize,
    pub max_total_chars_limit: usize,
    pub min_timeout_seconds: f64,
    pub max_timeout_seconds: f64,
    pub max_json_timeout_seconds: f64,
    pub min_offset: usize,
    pub max_offset: usize,

    // rg/file-list scan controls. These are intentionally larger than the
    // returned page size so cxs can detect truncation and propose a next step.
    pub max_filesize: &'static str,
    pub process_line_multiplier: usize,
    pub file_scan_line_multiplier: usize,
    pub files_only_min_line_limit: usize,
    pub min_files_only_candidate_chars: usize,
    pub files_only_candidate_char_multiplier: usize,
    pub next_files_batch_size: usize,
    pub next_find_max_total_hits: usize,
    pub limited_files_report_limit: usize,

    // Internal subprocess IO/probe limits.
    pub read_chunk_bytes: usize,
    pub stderr_capture_bytes: usize,
    pub termination_wait_seconds: f64,
    pub selector_min_sleep_seconds: f64,
    pub selector_max_sleep_seconds: f64,
    pub match_all_probe_output_bytes: usize,
    pub match_all_probe_lines: usize,

    // Structured-data search controls.
    pub max_records_examined: usize,
    pub max_records_examined_limit: usize,
    pub min_file_bytes: u64,
    pub max_file_bytes: u64,
    pub max_file_bytes_limit: u64,
    pub json_large_list_items: usize,
    pub json_large_scalar_skip_chars: usize,
    pub json_flatten_value_chars: usize,

    // Bounded diagnostic output.
    pub stderr_chars: usize,
    pub self_check_rg_timeout_seconds: f64,
}

pub const DEFAULT_BUDGET: Budget = Budget {
    max_total_hits: 300,
    max_hits_per_file: 999,
    max_line_chars: 240,
    max_total_chars: 12000,
    timeout_seconds: 5.0,
    process_output_bytes: 2_000_000,
    min_total_hits: 1,
    max_total_hits_limit: 5000,
    min_hits_per_file: 1,
    max_hits_per_file_limit: 5000,
    min_line_chars: 40,
    max_line_chars_limit: 5000,
    min_total_chars: 1000,
    max_total_chars_limit: 500000,
    min_timeout_seconds: 0.2,
    max_timeout_seconds: 60.0,
    max_json_timeout_seconds: 120.0,
    min_offset: 0,
    max_offset: 1_000_000,
    max_filesize: "5M",
    process_line_multiplier: 30,
    file_scan_line_multiplier: 20,
    files_only_min_line_limit: 20,
    min_files_only_candidate_chars: 1000,
    files_only_candidate_char_multiplier: 20,
    next_files_batch_size: 10,
    next_find_max_total_hits: 20,
    limited_files_report_limit: 20,
    read_chunk_bytes: 65536,
    stderr_capture_bytes: 65536,
    termination_wait_seconds: 0.5,
    selector_min_sleep_seconds: 0.01,
    selector_max_sleep_seconds: 0.1,
    match_all_probe_output_bytes: 65536,
    match_all_probe_lines: 2,
    max_records_examined: 200000,
    max_records_examined_limit: 5_000_000,
    min_file_bytes: 1,
    max_file_bytes: 64_000_000,
    max_file_bytes_limit: 1_000_000_000,
    json_large_list_items: 50,
    json_large_scalar_skip_chars: 2000,
    json_flatten_value_chars: 5000,
    stderr_chars: 1000,
    self_check_rg_timeout_seconds: 3.0,
};

#[derive(Debug)]
pub struct ProcResult {
    pub lines: Vec<Vec<u8>>,
    pub returncode: i32,
    pub stderr: String,
    pub timed_out: bool,
    pub byte_limit_hit: bool,
    pub line_limit_hit: bool,
    pub bytes_read: usize,
}

pub fn json_dumps(obj: &Value, pretty: bool) -> String {
    if pretty {
        serde_json::to_string_pretty(obj).unwrap_or_else(|_| "{}".to_string())
    } else {
        serde_json::to_string(obj).unwrap_or_else(|_| "{}".to_string())
    }
}

fn as_list_value(x: Option<&Value>) -> Vec<String> {
    match x {
        None | Some(Value::Null) => vec![],
        Some(Value::String(s)) => {
            if s.is_empty() {
                vec![]
            } else {
                vec![s.clone()]
            }
        }
        Some(Value::Array(xs)) => xs
            .iter()
            .filter_map(|v| match v {
                Value::String(s) if !s.is_empty() => Some(s.clone()),
                Value::Null => None,
                other => {
                    let s = value_to_search_string(other);
                    if s.is_empty() {
                        None
                    } else {
                        Some(s)
                    }
                }
            })
            .collect(),
        Some(other) => {
            let s = value_to_search_string(other);
            if s.is_empty() {
                vec![]
            } else {
                vec![s]
            }
        }
    }
}

fn value_to_search_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Bool(b) => {
            if *b {
                "True".to_string()
            } else {
                "False".to_string()
            }
        }
        Value::Null => "None".to_string(),
        other => other.to_string(),
    }
}

fn str_arg(args: &Value, key: &str) -> Option<String> {
    args.get(key)
        .and_then(|v| v.as_str().map(|s| s.to_string()))
}

fn bool_arg(args: &Value, key: &str, default: bool) -> bool {
    args.get(key).and_then(Value::as_bool).unwrap_or(default)
}

fn opt_bool_arg(args: &Value, key: &str) -> Option<bool> {
    args.get(key).and_then(Value::as_bool)
}

fn usize_arg(args: &Value, key: &str) -> Option<usize> {
    args.get(key)
        .and_then(|v| v.as_u64().and_then(|n| usize::try_from(n).ok()))
}

fn u64_arg(args: &Value, key: &str) -> Option<u64> {
    args.get(key).and_then(Value::as_u64)
}

fn f64_arg(args: &Value, key: &str) -> Option<f64> {
    args.get(key).and_then(Value::as_f64)
}

fn root_path(root: Option<String>) -> Result<PathBuf, String> {
    let p = root
        .map(PathBuf::from)
        .unwrap_or_else(|| env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let expanded = if let Some(rest) = p.to_string_lossy().strip_prefix("~/") {
        if let Some(home) = env::var_os("HOME") {
            PathBuf::from(home).join(rest)
        } else {
            p
        }
    } else {
        p
    };
    if !expanded.exists() {
        return Err(format!("root does not exist: {}", expanded.display()));
    }
    expanded
        .canonicalize()
        .map_err(|e| format!("root canonicalize failed: {e}"))
}

fn safe_rel(root: &Path, p: &Path) -> Option<String> {
    let rp = p.canonicalize().ok()?;
    if rp == root {
        return Some(".".to_string());
    }
    let rel = rp.strip_prefix(root).ok()?;
    Some(rel.to_string_lossy().replace('\\', "/"))
}

fn normalize_paths(root: &Path, paths: Option<&Value>) -> (Vec<String>, Vec<Value>) {
    let raw = as_list_value(paths);
    if raw.is_empty() {
        return (vec![".".to_string()], vec![]);
    }
    let mut ok = Vec::new();
    let mut bad = Vec::new();
    for item in raw {
        let p0 = PathBuf::from(&item);
        let p = if p0.is_absolute() { p0 } else { root.join(&p0) };
        if !p.exists() {
            bad.push(json!({"path": item, "reason": "missing"}));
            continue;
        }
        match safe_rel(root, &p) {
            Some(rel) => ok.push(rel),
            None => bad.push(json!({"path": item, "reason": "outside_root"})),
        }
    }
    (ok, bad)
}

fn paths_all_files(root: &Path, rels: &[String]) -> bool {
    !(rels.is_empty() || rels.len() == 1 && rels[0] == ".")
        && rels.iter().all(|r| root.join(r).is_file())
}

pub fn find_rg() -> Option<String> {
    let shim = env::var_os("CXS_SHIM_PATH")
        .map(PathBuf::from)
        .and_then(|p| p.canonicalize().ok());
    find_rg_excluding(shim.as_deref())
}

pub fn find_rg_excluding(exclude: Option<&Path>) -> Option<String> {
    let path = env::var_os("PATH")?;
    for dir in env::split_paths(&path) {
        let cand = dir.join("rg");
        if !cand.is_file() {
            continue;
        }
        if let Some(skip) = exclude {
            if cand.canonicalize().ok().as_deref() == Some(skip) {
                continue;
            }
        }
        return Some(cand.to_string_lossy().to_string());
    }
    None
}

fn clamp_usize(x: Option<usize>, lo: usize, hi: usize, default: usize) -> usize {
    x.unwrap_or(default).clamp(lo, hi)
}

fn clamp_u64(x: Option<u64>, lo: u64, hi: u64, default: u64) -> u64 {
    x.unwrap_or(default).clamp(lo, hi)
}

fn clamp_f64(x: Option<f64>, lo: f64, hi: f64, default: f64) -> f64 {
    x.unwrap_or(default).clamp(lo, hi)
}

fn clean_rel(s: &str) -> String {
    s.strip_prefix("./").unwrap_or(s).to_string()
}

fn truncate_chars(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        return s.to_string();
    }
    if n <= 16 {
        return s.chars().take(n).collect();
    }
    let keep = n.saturating_sub(14);
    format!("{}…[truncated]", s.chars().take(keep).collect::<String>())
}

fn scope_includes(scope: &str) -> Option<&'static [&'static str]> {
    match scope {
        "docs" => Some(&[
            "**/*.md",
            "**/*.mdx",
            "**/*.txt",
            "**/*.rst",
            "**/*.adoc",
            "AGENTS.md",
            "README*",
            "CHANGELOG*",
            "TODO*",
            "NOTES*",
        ]),
        "src" => Some(&[
            "**/*.py",
            "**/*.pyi",
            "**/*.rs",
            "**/*.go",
            "**/*.js",
            "**/*.jsx",
            "**/*.ts",
            "**/*.tsx",
            "**/*.java",
            "**/*.kt",
            "**/*.c",
            "**/*.cc",
            "**/*.cpp",
            "**/*.h",
            "**/*.hpp",
            "**/*.cs",
            "**/*.rb",
            "**/*.php",
            "**/*.swift",
            "**/*.m",
            "**/*.scala",
            "**/*.sh",
            "**/*.bash",
            "**/*.zsh",
            "**/*.fish",
            "**/*.pl",
            "**/*.r",
            "**/*.R",
            "**/*.jl",
            "**/*.lua",
        ]),
        "tests" => Some(&["**/test/**", "**/tests/**", "**/*test*.*", "**/*spec*.*"]),
        "config" => Some(&[
            "**/*.toml",
            "**/*.yaml",
            "**/*.yml",
            "**/*.json",
            "**/*.jsonl",
            "**/*.ini",
            "**/*.cfg",
            "**/*.conf",
            "**/*.env",
            "**/Dockerfile",
            "**/Makefile",
            "**/*.mk",
        ]),
        "analysis" => Some(&[
            "**/analysis/**",
            "**/reports/**",
            "**/summaries/**",
            "**/*.csv",
            "**/*.tsv",
        ]),
        "runs" => Some(&[
            "**/experiments/runs/**",
            "**/runs/**",
            "**/artifacts/**",
            "**/outputs/**",
        ]),
        "vendor" => Some(&[
            "**/resources/code/github/**",
            "**/vendor/**",
            "**/third_party/**",
        ]),
        "all" => Some(&["**/*"]),
        _ => None,
    }
}

fn build_globs(
    scopes: Option<&Value>,
    include_globs: Option<&Value>,
    exclude_globs: Option<&Value>,
    explicit_paths: bool,
    explicit_files: bool,
) -> (Vec<String>, Vec<String>) {
    let requested = {
        let xs = as_list_value(scopes);
        if xs.is_empty() {
            DEFAULT_SCOPES.iter().map(|s| s.to_string()).collect()
        } else {
            xs
        }
    };
    let mut scope_set = Vec::<String>::new();
    for s in requested {
        let ss = s.trim().to_lowercase();
        if !ss.is_empty() && !scope_set.contains(&ss) {
            scope_set.push(ss);
        }
    }
    let mut includes: Vec<String> = vec![];
    let mut warnings: Vec<String> = vec![];
    if !explicit_paths {
        for s in &scope_set {
            if let Some(gs) = scope_includes(s) {
                includes.extend(gs.iter().map(|g| g.to_string()));
            } else {
                warnings.push(format!("unknown_scope:{s}"));
            }
        }
    }
    includes.extend(as_list_value(include_globs));
    let mut excludes: Vec<String> = if explicit_files {
        vec![]
    } else {
        DEFAULT_EXCLUDES.iter().map(|g| g.to_string()).collect()
    };
    if scope_set.iter().any(|s| s == "vendor") {
        excludes.retain(|g| {
            !g.contains("resources/code/github")
                && !g.contains("vendor")
                && !g.contains("third_party")
        });
    }
    if scope_set.iter().any(|s| s == "runs") {
        excludes.retain(|g| !g.contains("experiments/runs") && !g.contains("**/runs/**"));
    }
    excludes.extend(as_list_value(exclude_globs));
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for g in includes {
        if !g.is_empty() && seen.insert(g.clone()) {
            out.push("-g".to_string());
            out.push(g);
        }
    }
    for g in excludes {
        let gg = if g.starts_with('!') {
            g
        } else {
            format!("!{g}")
        };
        if seen.insert(gg.clone()) {
            out.push("-g".to_string());
            out.push(gg);
        }
    }
    (out, warnings)
}

pub fn run_lines(
    cmd: &[String],
    cwd: &Path,
    timeout: f64,
    max_bytes: usize,
    max_lines: usize,
) -> ProcResult {
    let mut child = match Command::new(&cmd[0])
        .args(&cmd[1..])
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            return ProcResult {
                lines: vec![],
                returncode: 127,
                stderr: e.to_string(),
                timed_out: false,
                byte_limit_hit: false,
                line_limit_hit: false,
                bytes_read: 0,
            }
        }
    };
    let stdout = child.stdout.take().expect("stdout piped");
    let stderr = child.stderr.take().expect("stderr piped");
    let (tx, rx) = sync_channel::<Vec<u8>>(1024);
    let out_thread = thread::spawn(move || {
        let mut rdr = BufReader::new(stdout);
        loop {
            let mut line = Vec::new();
            match rdr.read_until(b'\n', &mut line) {
                Ok(0) => break,
                Ok(_) => {
                    if line.ends_with(b"\n") {
                        line.pop();
                        if line.ends_with(b"\r") {
                            line.pop();
                        }
                    }
                    if tx.send(line).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });
    let err_thread = thread::spawn(move || {
        let mut rdr = BufReader::new(stderr);
        let mut buf = vec![0_u8; DEFAULT_BUDGET.read_chunk_bytes];
        let mut err = Vec::new();
        loop {
            match rdr.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    let remaining = DEFAULT_BUDGET
                        .stderr_capture_bytes
                        .saturating_sub(err.len());
                    if remaining > 0 {
                        err.extend_from_slice(&buf[..n.min(remaining)]);
                    }
                }
                Err(_) => break,
            }
        }
        err
    });

    let start = Instant::now();
    let deadline = start + Duration::from_secs_f64(timeout.max(0.0));
    let mut lines = Vec::new();
    let mut bytes_read = 0_usize;
    let mut timed_out = false;
    let mut byte_limit_hit = false;
    let mut line_limit_hit = false;
    let mut returncode = 0_i32;

    loop {
        while let Ok(line) = rx.try_recv() {
            bytes_read = bytes_read.saturating_add(line.len() + 1);
            lines.push(line);
            if lines.len() >= max_lines {
                line_limit_hit = true;
                break;
            }
            if bytes_read >= max_bytes {
                byte_limit_hit = true;
                break;
            }
        }
        if line_limit_hit || byte_limit_hit {
            let _ = child.kill();
            break;
        }
        if Instant::now() >= deadline {
            timed_out = true;
            let _ = child.kill();
            break;
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                returncode = status.code().unwrap_or(-1);
                break;
            }
            Ok(None) => {}
            Err(e) => {
                returncode = -1;
                if lines.is_empty() {
                    let stderr = e.to_string();
                    let _ = child.kill();
                    let _ = child.wait();
                    let _ = out_thread.join();
                    let _ = err_thread.join();
                    return ProcResult {
                        lines,
                        returncode,
                        stderr,
                        timed_out,
                        byte_limit_hit,
                        line_limit_hit,
                        bytes_read,
                    };
                }
                break;
            }
        }
        let now = Instant::now();
        let wait_for = if deadline > now {
            (deadline - now).min(Duration::from_secs_f64(
                DEFAULT_BUDGET.selector_max_sleep_seconds,
            ))
        } else {
            Duration::from_secs_f64(DEFAULT_BUDGET.selector_min_sleep_seconds)
        };
        match rx.recv_timeout(wait_for) {
            Ok(line) => {
                bytes_read = bytes_read.saturating_add(line.len() + 1);
                lines.push(line);
                if lines.len() >= max_lines {
                    line_limit_hit = true;
                    let _ = child.kill();
                    break;
                }
                if bytes_read >= max_bytes {
                    byte_limit_hit = true;
                    let _ = child.kill();
                    break;
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                if let Ok(Some(status)) = child.try_wait() {
                    returncode = status.code().unwrap_or(-1);
                    break;
                }
            }
        }
    }

    let _ = child.wait();
    let drain_until =
        Instant::now() + Duration::from_secs_f64(DEFAULT_BUDGET.termination_wait_seconds);
    loop {
        match rx.recv_timeout(Duration::from_millis(10)) {
            Ok(line) => {
                let line_bytes = line.len() + 1;
                if lines.len() < max_lines && bytes_read < max_bytes {
                    bytes_read = bytes_read.saturating_add(line_bytes);
                    if bytes_read >= max_bytes {
                        byte_limit_hit = true;
                    } else {
                        lines.push(line);
                    }
                }
            }
            Err(RecvTimeoutError::Disconnected) => break,
            Err(RecvTimeoutError::Timeout) => {
                if out_thread.is_finished() || Instant::now() >= drain_until {
                    while let Ok(line) = rx.try_recv() {
                        let line_bytes = line.len() + 1;
                        if lines.len() < max_lines && bytes_read < max_bytes {
                            bytes_read = bytes_read.saturating_add(line_bytes);
                            if bytes_read < max_bytes {
                                lines.push(line);
                            }
                        }
                    }
                    break;
                }
            }
        }
    }
    let _ = out_thread.join();
    let err = err_thread.join().unwrap_or_default();
    let stderr = String::from_utf8_lossy(&err).to_string();
    ProcResult {
        lines,
        returncode,
        stderr,
        timed_out,
        byte_limit_hit,
        line_limit_hit,
        bytes_read,
    }
}

fn make_base_response(status: &str, pairs: Vec<(&str, Value)>) -> Value {
    let mut m = Map::new();
    m.insert("status".to_string(), Value::String(status.to_string()));
    for (k, v) in pairs {
        m.insert(k.to_string(), v);
    }
    Value::Object(m)
}

fn build_rg_files_with_matches_cmd(
    rg: &str,
    terms: &[String],
    mode: &str,
    search_paths: &[String],
    globs: &[String],
    case_sensitive: Option<bool>,
) -> Vec<String> {
    let mut cmd = vec![
        rg.to_string(),
        "-l".to_string(),
        "--color=never".to_string(),
        "--max-filesize".to_string(),
        DEFAULT_BUDGET.max_filesize.to_string(),
    ];
    if case_sensitive != Some(true) {
        cmd.push("-i".to_string());
    }
    cmd.extend(globs.iter().cloned());
    if mode == "literal" {
        cmd.push("-F".to_string());
    }
    for t in terms {
        cmd.push("-e".to_string());
        cmd.push(t.clone());
    }
    cmd.extend(search_paths.iter().cloned());
    cmd
}

fn collect_file_hits(
    lines: &[Vec<u8>],
    max_files: usize,
    max_total_chars: usize,
    offset: usize,
) -> (Vec<Value>, usize, bool, usize, bool) {
    let mut files = Vec::new();
    let mut seen = HashSet::new();
    let mut total_chars = 0_usize;
    let mut truncated = false;
    let mut has_more = false;
    for raw in lines {
        let rel = clean_rel(String::from_utf8_lossy(raw).trim());
        if rel.is_empty() || !seen.insert(rel.clone()) {
            continue;
        }
        if seen.len() <= offset {
            continue;
        }
        let add = rel.chars().count() + 16;
        if files.len() >= max_files || total_chars + add > max_total_chars {
            has_more = true;
            truncated = true;
            break;
        }
        total_chars += add;
        files.push(json!({"file": rel}));
    }
    (files, total_chars, truncated, seen.len(), has_more)
}

struct FindNextInput<'a> {
    query: Option<&'a str>,
    term_list: &'a [String],
    match_mode: &'a str,
    mode: &'a str,
    files: &'a [Value],
    max_total_hits: usize,
    max_hits_per_file: usize,
    max_line_chars: usize,
    case_sensitive: Option<bool>,
}

fn make_find_files_next(input: FindNextInput<'_>) -> Vec<Value> {
    let FindNextInput {
        query,
        term_list,
        match_mode,
        mode,
        files,
        max_total_hits,
        max_hits_per_file,
        max_line_chars,
        case_sensitive,
    } = input;
    if files.is_empty() {
        return vec![];
    }
    let batch_size = DEFAULT_BUDGET.next_files_batch_size;
    let paths: Vec<Value> = files
        .iter()
        .take(batch_size)
        .filter_map(|f| {
            f.get("file")
                .and_then(Value::as_str)
                .map(|s| Value::String(s.to_string()))
        })
        .collect();
    let mut args = Map::new();
    args.insert("paths".to_string(), Value::Array(paths));
    args.insert(
        "max_total_hits".to_string(),
        json!(DEFAULT_BUDGET.next_find_max_total_hits.min(max_total_hits)),
    );
    args.insert("max_hits_per_file".to_string(), json!(max_hits_per_file));
    args.insert("max_line_chars".to_string(), json!(max_line_chars));
    if let (Some(query), 1) = (query, term_list.len()) {
        args.insert("query".to_string(), Value::String(query.to_string()));
    } else {
        args.insert("terms".to_string(), json!(term_list));
    }
    if match_mode != "any" {
        args.insert("match".to_string(), Value::String(match_mode.to_string()));
    }
    if mode != "literal" {
        args.insert("mode".to_string(), Value::String(mode.to_string()));
    }
    if case_sensitive == Some(true) {
        args.insert("case_sensitive".to_string(), Value::Bool(true));
    }
    vec![json!({"op": "find", "args": Value::Object(args)})]
}

fn make_find_next_meta(files: &[Value], batch_size: Option<usize>) -> Value {
    let bs = batch_size.unwrap_or(DEFAULT_BUDGET.next_files_batch_size);
    let total = files.len();
    json!({
        "batch_size": bs,
        "files_in_next": total.min(bs),
        "total_files_in_page": total,
        "remaining_files_in_page": total.saturating_sub(bs),
        "next_is_first_batch": total > bs
    })
}

fn root_or_error(args: &Value) -> Result<PathBuf, Value> {
    root_path(str_arg(args, "root"))
        .map_err(|e| make_base_response("error", vec![("error", Value::String(e))]))
}

pub fn cxs_find(args: &Value) -> Value {
    let rp = match root_or_error(args) {
        Ok(p) => p,
        Err(v) => return v,
    };
    let rg = match find_rg() {
        Some(r) => r,
        None => return make_base_response("error", vec![("error", json!("rg_not_found"))]),
    };
    let query = str_arg(args, "query");
    let mut term_list = Vec::new();
    if let Some(q) = &query {
        if !q.is_empty() {
            term_list.push(q.clone());
        }
    }
    term_list.extend(as_list_value(args.get("terms")));
    term_list.retain(|s| !s.is_empty());
    if term_list.is_empty() {
        return make_base_response("error", vec![("error", json!("empty_query"))]);
    }
    let match_mode = str_arg(args, "match").unwrap_or_else(|| "any".to_string());
    let mode = str_arg(args, "mode").unwrap_or_else(|| "literal".to_string());
    let case_sensitive = opt_bool_arg(args, "case_sensitive");
    let max_hits = clamp_usize(
        usize_arg(args, "max_total_hits"),
        DEFAULT_BUDGET.min_total_hits,
        DEFAULT_BUDGET.max_total_hits_limit,
        DEFAULT_BUDGET.max_total_hits,
    );
    let max_per = clamp_usize(
        usize_arg(args, "max_hits_per_file"),
        DEFAULT_BUDGET.min_hits_per_file,
        DEFAULT_BUDGET.max_hits_per_file_limit,
        DEFAULT_BUDGET.max_hits_per_file,
    );
    let max_line = clamp_usize(
        usize_arg(args, "max_line_chars"),
        DEFAULT_BUDGET.min_line_chars,
        DEFAULT_BUDGET.max_line_chars_limit,
        DEFAULT_BUDGET.max_line_chars,
    );
    let max_chars = clamp_usize(
        usize_arg(args, "max_total_chars"),
        DEFAULT_BUDGET.min_total_chars,
        DEFAULT_BUDGET.max_total_chars_limit,
        DEFAULT_BUDGET.max_total_chars,
    );
    let timeout = clamp_f64(
        f64_arg(args, "timeout_seconds"),
        DEFAULT_BUDGET.min_timeout_seconds,
        DEFAULT_BUDGET.max_timeout_seconds,
        DEFAULT_BUDGET.timeout_seconds,
    );
    let file_offset = clamp_usize(
        usize_arg(args, "offset"),
        DEFAULT_BUDGET.min_offset,
        DEFAULT_BUDGET.max_offset,
        DEFAULT_BUDGET.min_offset,
    );
    let (search_paths, rejected) = normalize_paths(&rp, args.get("paths"));
    if search_paths.is_empty() {
        return make_base_response(
            "error",
            vec![
                ("error", json!("no_valid_paths")),
                ("rejected_paths", Value::Array(rejected)),
            ],
        );
    }
    let explicit = !as_list_value(args.get("paths")).is_empty();
    let explicit_files = paths_all_files(&rp, &search_paths);
    let (globs, warnings) = build_globs(
        args.get("scopes"),
        args.get("include_globs"),
        args.get("exclude_globs"),
        explicit,
        explicit_files,
    );
    if bool_arg(args, "files_only", false) {
        return cxs_find_files_only(
            &rg,
            &rp,
            query,
            &term_list,
            &match_mode,
            &mode,
            &search_paths,
            &globs,
            rejected,
            warnings,
            case_sensitive,
            max_hits,
            max_per,
            max_line,
            max_chars,
            timeout,
            file_offset,
        );
    }

    let mut cmd = vec![
        rg,
        "-n".to_string(),
        "-H".to_string(),
        "--no-heading".to_string(),
        "--color=never".to_string(),
        "--field-match-separator".to_string(),
        "\t".to_string(),
        "--max-count".to_string(),
        max_per.to_string(),
        "--max-columns".to_string(),
        max_line.to_string(),
        "--max-columns-preview".to_string(),
        "--max-filesize".to_string(),
        DEFAULT_BUDGET.max_filesize.to_string(),
    ];
    if case_sensitive != Some(true) {
        cmd.push("-i".to_string());
    }
    cmd.extend(globs);
    let search_terms = if match_mode == "all" {
        let first = term_list
            .iter()
            .max_by_key(|s| s.len())
            .cloned()
            .unwrap_or_default();
        vec![first]
    } else {
        term_list.clone()
    };
    if mode == "literal" {
        cmd.push("-F".to_string());
    }
    for t in &search_terms {
        cmd.push("-e".to_string());
        cmd.push(t.clone());
    }
    cmd.extend(search_paths);
    let line_limit = max_hits * DEFAULT_BUDGET.process_line_multiplier;
    let proc = run_lines(
        &cmd,
        &rp,
        timeout,
        DEFAULT_BUDGET.process_output_bytes,
        line_limit,
    );
    let mut hits = Vec::new();
    let mut file_counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut total_chars = 0_usize;
    let mut hit_limit_hit = false;
    let mut output_limit_hit = false;
    let regex_terms = if match_mode == "all" && mode != "literal" {
        match compile_regex_terms(&term_list, case_sensitive) {
            Ok(rs) => Some(rs),
            Err(_) => {
                return make_base_response(
                    "error",
                    vec![
                        ("error", json!("bad_regex")),
                        (
                            "query",
                            query.clone().map(Value::String).unwrap_or(Value::Null),
                        ),
                        ("terms", json!(term_list)),
                    ],
                )
            }
        }
    } else {
        None
    };
    for raw in &proc.lines {
        let s = String::from_utf8_lossy(raw).to_string();
        let mut parts = s.splitn(3, '\t');
        let Some(file_s0) = parts.next() else {
            continue;
        };
        let Some(line_s) = parts.next() else { continue };
        let Some(text) = parts.next() else { continue };
        let file_s = clean_rel(file_s0);
        if match_mode == "all" {
            if mode == "literal" {
                let hay = if case_sensitive == Some(true) {
                    text.to_string()
                } else {
                    text.to_lowercase()
                };
                let needles: Vec<String> = if case_sensitive == Some(true) {
                    term_list.clone()
                } else {
                    term_list.iter().map(|t| t.to_lowercase()).collect()
                };
                if !needles.iter().all(|n| hay.contains(n)) {
                    continue;
                }
            } else if let Some(rs) = &regex_terms {
                if !rs.iter().all(|r| r.is_match(text)) {
                    continue;
                }
            }
        }
        let line_val = line_s
            .parse::<usize>()
            .ok()
            .map(Value::from)
            .unwrap_or(Value::Null);
        if file_counts.get(&file_s).copied().unwrap_or(0) >= max_per {
            continue;
        }
        let snippet = truncate_chars(text.trim_end_matches('\n'), max_line);
        let add = file_s.chars().count() + snippet.chars().count() + 32;
        if hits.len() >= max_hits {
            hit_limit_hit = true;
            break;
        }
        if total_chars + add > max_chars {
            output_limit_hit = true;
            break;
        }
        *file_counts.entry(file_s.clone()).or_insert(0) += 1;
        total_chars += add;
        hits.push(json!({"file": file_s, "line": line_val, "snippet": snippet}));
    }
    let files_at_hit_limit: Vec<String> = file_counts
        .iter()
        .filter_map(|(f, count)| {
            if *count >= max_per {
                Some(f.clone())
            } else {
                None
            }
        })
        .collect();
    let mut limit_reasons = Vec::new();
    if proc.timed_out {
        limit_reasons.push(json!("timeout"));
    }
    if proc.byte_limit_hit {
        limit_reasons.push(json!("process_output_bytes"));
    }
    if proc.line_limit_hit || proc.lines.len() >= line_limit {
        limit_reasons.push(json!("process_line_limit"));
    }
    if hit_limit_hit || hits.len() >= max_hits {
        limit_reasons.push(json!("max_total_hits"));
    }
    if output_limit_hit || total_chars >= max_chars {
        limit_reasons.push(json!("max_total_chars"));
    }
    if !files_at_hit_limit.is_empty() {
        limit_reasons.push(json!("max_hits_per_file"));
    }
    let truncated = proc.timed_out
        || proc.byte_limit_hit
        || proc.line_limit_hit
        || proc.lines.len() >= line_limit
        || hit_limit_hit
        || hits.len() >= max_hits
        || output_limit_hit
        || total_chars >= max_chars
        || !files_at_hit_limit.is_empty();
    let mut status = if truncated { "truncated" } else { "ok" };
    if proc.returncode != 0 && proc.returncode != 1 && hits.is_empty() {
        status = "error";
    }
    let mut out = Map::new();
    out.insert("status".to_string(), json!(status));
    out.insert(
        "query".to_string(),
        query.map(Value::String).unwrap_or(Value::Null),
    );
    out.insert("terms".to_string(), json!(term_list));
    out.insert("match".to_string(), json!(match_mode));
    out.insert("mode".to_string(), json!(mode));
    out.insert(
        "stats".to_string(),
        json!({
            "hits": hits.len(),
            "files": file_counts.len(),
            "truncated": truncated,
            "limit_reasons": limit_reasons,
            "files_at_hit_limit": files_at_hit_limit.len(),
            "rc": proc.returncode,
            "bytes": proc.bytes_read,
        }),
    );
    out.insert("hits".to_string(), Value::Array(hits));
    if !files_at_hit_limit.is_empty() {
        let limited: Vec<Value> = files_at_hit_limit
            .iter()
            .take(DEFAULT_BUDGET.limited_files_report_limit)
            .map(|f| json!({"file": f, "hits": file_counts[f], "reason": "max_hits_per_file"}))
            .collect();
        out.insert("limited_files".to_string(), Value::Array(limited));
    }
    if !rejected.is_empty() {
        out.insert("rejected_paths".to_string(), Value::Array(rejected));
    }
    if !warnings.is_empty() {
        out.insert("warnings".to_string(), json!(warnings));
    }
    if !proc.stderr.is_empty() && (status == "error" || out.contains_key("warnings")) {
        out.insert(
            "stderr".to_string(),
            json!(truncate_chars(&proc.stderr, DEFAULT_BUDGET.stderr_chars)),
        );
    }
    Value::Object(out)
}

#[allow(clippy::too_many_arguments)]
fn cxs_find_files_only(
    rg: &str,
    rp: &Path,
    query: Option<String>,
    term_list: &[String],
    match_mode: &str,
    mode: &str,
    search_paths: &[String],
    globs: &[String],
    rejected: Vec<Value>,
    warnings: Vec<String>,
    case_sensitive: Option<bool>,
    max_total_hits: usize,
    max_hits_per_file: usize,
    max_line_chars: usize,
    max_total_chars: usize,
    timeout: f64,
    offset: usize,
) -> Value {
    let line_limit = DEFAULT_BUDGET
        .files_only_min_line_limit
        .max(max_total_hits * DEFAULT_BUDGET.file_scan_line_multiplier)
        .max(offset + max_total_hits + 1);
    let mut stderr: String;
    let mut bytes_read = 0_usize;
    let mut truncated = false;
    let mut has_more = false;
    let mut seen_files = 0_usize;
    let mut files: Vec<Value> = vec![];
    let mut proc_opt: Option<ProcResult> = None;

    if match_mode == "all" && term_list.len() > 1 {
        let first = term_list
            .iter()
            .max_by_key(|s| s.len())
            .cloned()
            .unwrap_or_default();
        let rest: Vec<String> = term_list.iter().filter(|t| **t != first).cloned().collect();
        let proc = run_lines(
            &build_rg_files_with_matches_cmd(
                rg,
                &[first],
                mode,
                search_paths,
                globs,
                case_sensitive,
            ),
            rp,
            timeout,
            DEFAULT_BUDGET.process_output_bytes,
            line_limit,
        );
        stderr = proc.stderr.clone();
        bytes_read += proc.bytes_read;
        if proc.returncode == 0 || proc.returncode == 1 {
            let candidate_max_chars = DEFAULT_BUDGET
                .min_files_only_candidate_chars
                .max(max_total_chars * DEFAULT_BUDGET.files_only_candidate_char_multiplier);
            let (candidates, _, collect_truncated, candidate_count, candidate_has_more) =
                collect_file_hits(&proc.lines, line_limit, candidate_max_chars, 0);
            truncated =
                proc.timed_out || proc.byte_limit_hit || proc.line_limit_hit || collect_truncated;
            has_more = candidate_has_more;
            let deadline = Instant::now() + Duration::from_secs_f64(timeout.max(0.0));
            let mut total_chars = 0_usize;
            let mut matched_files = 0_usize;
            let mut seen = HashSet::new();
            for cand in candidates {
                let rel = cand
                    .get("file")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                if rel.is_empty() || !seen.insert(rel.clone()) {
                    continue;
                }
                let mut ok = true;
                for term in &rest {
                    let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
                        truncated = true;
                        ok = false;
                        break;
                    };
                    let sub = run_lines(
                        &build_rg_files_with_matches_cmd(
                            rg,
                            std::slice::from_ref(term),
                            mode,
                            std::slice::from_ref(&rel),
                            &[],
                            case_sensitive,
                        ),
                        rp,
                        remaining.as_secs_f64().min(timeout),
                        DEFAULT_BUDGET.match_all_probe_output_bytes,
                        DEFAULT_BUDGET.match_all_probe_lines,
                    );
                    bytes_read += sub.bytes_read;
                    if !sub.stderr.is_empty() && stderr.is_empty() {
                        stderr = sub.stderr.clone();
                    }
                    if sub.returncode != 0 && sub.returncode != 1 {
                        ok = false;
                        if files.is_empty() {
                            proc_opt = Some(sub);
                        }
                        break;
                    }
                    if sub.lines.is_empty() {
                        ok = false;
                        break;
                    }
                }
                if !ok {
                    continue;
                }
                matched_files += 1;
                if matched_files <= offset {
                    continue;
                }
                let add = rel.chars().count() + 16;
                if files.len() >= max_total_hits || total_chars + add > max_total_chars {
                    has_more = true;
                    truncated = true;
                    break;
                }
                total_chars += add;
                files.push(json!({"file": rel}));
            }
            seen_files = if !has_more {
                matched_files
            } else {
                matched_files.max(candidate_count)
            };
        }
        if proc_opt.is_none() {
            proc_opt = Some(proc);
        }
    } else {
        let proc = run_lines(
            &build_rg_files_with_matches_cmd(
                rg,
                term_list,
                mode,
                search_paths,
                globs,
                case_sensitive,
            ),
            rp,
            timeout,
            DEFAULT_BUDGET.process_output_bytes,
            line_limit,
        );
        stderr = proc.stderr.clone();
        bytes_read = proc.bytes_read;
        let (f, _, collect_truncated, seen, hm) =
            collect_file_hits(&proc.lines, max_total_hits, max_total_chars, offset);
        files = f;
        seen_files = seen;
        has_more = hm;
        truncated =
            proc.timed_out || proc.byte_limit_hit || proc.line_limit_hit || collect_truncated;
        proc_opt = Some(proc);
    }
    if files.len() >= max_total_hits {
        truncated = true;
        has_more = true;
    }
    let rc = proc_opt.as_ref().map(|p| p.returncode);
    let mut status = if truncated { "truncated" } else { "ok" };
    if !matches!(rc, Some(0) | Some(1) | None) && files.is_empty() {
        status = "error";
    }
    let mut limit_reasons = Vec::new();
    if let Some(proc) = &proc_opt {
        if proc.timed_out {
            limit_reasons.push(json!("timeout"));
        }
        if proc.byte_limit_hit {
            limit_reasons.push(json!("process_output_bytes"));
        }
        if proc.line_limit_hit {
            limit_reasons.push(json!("process_line_limit"));
        }
    }
    if files.len() >= max_total_hits {
        limit_reasons.push(json!("max_total_hits"));
    }
    if has_more {
        limit_reasons.push(json!("files_page_has_more"));
    }
    if truncated && limit_reasons.is_empty() {
        limit_reasons.push(json!("files_only_limit"));
    }
    let nexts = make_find_files_next(FindNextInput {
        query: query.as_deref(),
        term_list,
        match_mode,
        mode,
        files: &files,
        max_total_hits,
        max_hits_per_file,
        max_line_chars,
        case_sensitive,
    });
    let mut out = Map::new();
    out.insert("status".to_string(), json!(status));
    out.insert(
        "query".to_string(),
        query.map(Value::String).unwrap_or(Value::Null),
    );
    out.insert("terms".to_string(), json!(term_list));
    out.insert("match".to_string(), json!(match_mode));
    out.insert("mode".to_string(), json!(mode));
    out.insert("files_only".to_string(), Value::Bool(true));
    out.insert(
        "stats".to_string(),
        json!({
            "files": files.len(),
            "offset": offset,
            "seen_files": seen_files,
            "has_more": has_more,
            "truncated": truncated,
            "limit_reasons": limit_reasons,
            "rc": rc,
            "bytes": bytes_read,
        }),
    );
    out.insert("files".to_string(), Value::Array(files.clone()));
    if !nexts.is_empty() {
        out.insert("next".to_string(), Value::Array(nexts));
        out.insert("next_meta".to_string(), make_find_next_meta(&files, None));
    }
    if has_more {
        out.insert(
            "next_page".to_string(),
            json!({"offset": offset + files.len(), "max_total_hits": max_total_hits}),
        );
    }
    if !rejected.is_empty() {
        out.insert("rejected_paths".to_string(), Value::Array(rejected));
    }
    if !warnings.is_empty() {
        out.insert("warnings".to_string(), json!(warnings));
    }
    if !stderr.is_empty() && (status == "error" || out.contains_key("warnings")) {
        out.insert(
            "stderr".to_string(),
            json!(truncate_chars(&stderr, DEFAULT_BUDGET.stderr_chars)),
        );
    }
    Value::Object(out)
}

pub fn cxs_files(args: &Value) -> Value {
    let rp = match root_or_error(args) {
        Ok(p) => p,
        Err(v) => return v,
    };
    let rg = match find_rg() {
        Some(r) => r,
        None => return make_base_response("error", vec![("error", json!("rg_not_found"))]),
    };
    let query = str_arg(args, "query");
    let mode = str_arg(args, "mode").unwrap_or_else(|| "literal".to_string());
    let case_sensitive = opt_bool_arg(args, "case_sensitive");
    let maxf = clamp_usize(
        usize_arg(args, "max_files"),
        DEFAULT_BUDGET.min_total_hits,
        DEFAULT_BUDGET.max_total_hits_limit,
        DEFAULT_BUDGET.max_total_hits,
    );
    let maxc = clamp_usize(
        usize_arg(args, "max_total_chars"),
        DEFAULT_BUDGET.min_total_chars,
        DEFAULT_BUDGET.max_total_chars_limit,
        DEFAULT_BUDGET.max_total_chars,
    );
    let (search_paths, rejected) = normalize_paths(&rp, args.get("paths"));
    if search_paths.is_empty() {
        return make_base_response(
            "error",
            vec![
                ("error", json!("no_valid_paths")),
                ("rejected_paths", Value::Array(rejected)),
            ],
        );
    }
    let explicit = !as_list_value(args.get("paths")).is_empty();
    let explicit_files = paths_all_files(&rp, &search_paths);
    let (globs, warnings) = build_globs(
        args.get("scopes"),
        args.get("include_globs"),
        args.get("exclude_globs"),
        explicit,
        explicit_files,
    );
    let mut cmd = vec![rg, "--files".to_string(), "--color=never".to_string()];
    cmd.extend(globs);
    cmd.extend(search_paths);
    let timeout = clamp_f64(
        f64_arg(args, "timeout_seconds"),
        DEFAULT_BUDGET.min_timeout_seconds,
        DEFAULT_BUDGET.max_timeout_seconds,
        DEFAULT_BUDGET.timeout_seconds,
    );
    let proc = run_lines(
        &cmd,
        &rp,
        timeout,
        DEFAULT_BUDGET.process_output_bytes,
        maxf * DEFAULT_BUDGET.file_scan_line_multiplier,
    );
    let path_regex = if query.is_some() && mode != "literal" {
        match compile_regex(query.as_deref().unwrap_or(""), case_sensitive) {
            Ok(r) => Some(r),
            Err(_) => {
                return make_base_response(
                    "error",
                    vec![
                        ("error", json!("bad_regex")),
                        (
                            "query",
                            query.clone().map(Value::String).unwrap_or(Value::Null),
                        ),
                    ],
                )
            }
        }
    } else {
        None
    };
    let mut files = Vec::new();
    let mut total = 0_usize;
    for raw in &proc.lines {
        let p = clean_rel(String::from_utf8_lossy(raw).trim());
        if p.is_empty() {
            continue;
        }
        let mut ok = true;
        if let Some(q) = &query {
            if mode == "literal" {
                if case_sensitive == Some(true) {
                    ok = p.contains(q);
                } else {
                    ok = p.to_lowercase().contains(&q.to_lowercase());
                }
            } else if let Some(r) = &path_regex {
                ok = r.is_match(&p);
            }
        }
        if !ok {
            continue;
        }
        if files.len() >= maxf || total + p.chars().count() > maxc {
            break;
        }
        total += p.chars().count();
        files.push(json!({"file": p}));
    }
    let truncated = proc.timed_out
        || proc.byte_limit_hit
        || proc.line_limit_hit
        || files.len() >= maxf
        || total >= maxc;
    let mut out = Map::new();
    out.insert(
        "status".to_string(),
        json!(if truncated { "truncated" } else { "ok" }),
    );
    out.insert(
        "query".to_string(),
        query.map(Value::String).unwrap_or(Value::Null),
    );
    out.insert(
        "stats".to_string(),
        json!({"files": files.len(), "truncated": truncated, "rc": proc.returncode}),
    );
    out.insert("files".to_string(), Value::Array(files));
    if !rejected.is_empty() {
        out.insert("rejected_paths".to_string(), Value::Array(rejected));
    }
    if !warnings.is_empty() {
        out.insert("warnings".to_string(), json!(warnings));
    }
    Value::Object(out)
}

fn compile_regex(pattern: &str, case_sensitive: Option<bool>) -> Result<Regex, regex::Error> {
    RegexBuilder::new(pattern)
        .case_insensitive(case_sensitive != Some(true))
        .build()
}

fn compile_regex_terms(
    terms: &[String],
    case_sensitive: Option<bool>,
) -> Result<Vec<Regex>, regex::Error> {
    terms
        .iter()
        .map(|t| compile_regex(t, case_sensitive))
        .collect()
}

pub fn symbol_pattern(name: &str) -> String {
    let n = regex::escape(name);
    [
        format!(r"^\s*(?:async\s+def|def|class)\s+{}\b", n),
        format!(r"^\s*(?:export\s+)?(?:async\s+)?function\s+{}\b", n),
        format!(r"^\s*(?:export\s+)?(?:const|let|var)\s+{}\s*[=:]", n),
        format!(r"^\s*(?:export\s+)?class\s+{}\b", n),
        format!(r"^\s*(?:pub\s+)?(?:async\s+)?fn\s+{}\b", n),
        format!(r"^\s*(?:pub\s+)?(?:struct|enum|trait|type)\s+{}\b", n),
        format!(r"^\s*func\s+(?:\([^)]*\)\s*)?{}\b", n),
        format!(r"^\s*type\s+{}\b", n),
        format!(r"^\s*(?:public|private|protected|static|final|abstract|internal|open|override|inline|extern|constexpr)\b.*\b{}\s*(?:\(|[:={{])", n),
        format!(r"^\s*{}\s*\([^)]*\)\s*(?:=>|\{{)", n),
    ]
    .join("|")
}

pub fn cxs_symbol(args: &Value) -> Value {
    let name = str_arg(args, "name").unwrap_or_default();
    let valid = Regex::new(r"^[\w.$:-]+$").unwrap();
    if name.is_empty() || !valid.is_match(&name) {
        return make_base_response("error", vec![("error", json!("bad_symbol_name"))]);
    }
    let mut next = args.as_object().cloned().unwrap_or_default();
    next.insert("query".to_string(), json!(symbol_pattern(&name)));
    next.insert("match".to_string(), json!("any"));
    next.insert("mode".to_string(), json!("regex"));
    next.remove("name");
    if !next.contains_key("scopes") {
        next.insert("scopes".to_string(), json!(["src", "tests"]));
    }
    cxs_find(&Value::Object(next))
}

fn large_field_regex() -> Regex {
    Regex::new(r"(?i)(?:^|_)(text|raw|content|prompt|completion|response|body|html|markdown|tokens?)(?:_|$)").unwrap()
}

fn flatten_text(rec: &Value, search_large_fields: bool) -> String {
    let large = large_field_regex();
    let mut parts = Vec::new();
    fn walk(
        x: &Value,
        key: &str,
        search_large_fields: bool,
        large: &Regex,
        parts: &mut Vec<String>,
    ) {
        match x {
            Value::Object(m) => {
                for (k, v) in m {
                    walk(v, k, search_large_fields, large, parts);
                }
            }
            Value::Array(xs) => {
                for v in xs.iter().take(DEFAULT_BUDGET.json_large_list_items) {
                    walk(v, key, search_large_fields, large, parts);
                }
            }
            _ => {
                if !search_large_fields && large.is_match(key) {
                    return;
                }
                let s = value_to_plain_string(x);
                if s.chars().count() > DEFAULT_BUDGET.json_large_scalar_skip_chars
                    && !search_large_fields
                {
                    return;
                }
                parts.push(
                    s.chars()
                        .take(DEFAULT_BUDGET.json_flatten_value_chars)
                        .collect(),
                );
            }
        }
    }
    walk(rec, "", search_large_fields, &large, &mut parts);
    parts.join("\n")
}

fn value_to_plain_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Bool(b) => b.to_string(),
        Value::Null => "null".to_string(),
        other => other.to_string(),
    }
}

fn project_record(
    rec: &Map<String, Value>,
    fields: Option<&Value>,
    omit_fields: Option<&Value>,
    max_value_chars: usize,
) -> Value {
    let field_list = as_list_value(fields);
    let omit: HashSet<String> = as_list_value(omit_fields).into_iter().collect();
    let large = large_field_regex();
    let mut out = Map::new();
    if !field_list.is_empty() {
        for k in field_list {
            if omit.contains(&k) {
                continue;
            }
            if let Some(v) = rec.get(&k) {
                out.insert(k, projected_value(v, max_value_chars));
            }
        }
    } else {
        for (k, v) in rec {
            if omit.contains(k) {
                continue;
            }
            if large.is_match(k) {
                out.insert(k.clone(), json!("[omitted]"));
            } else {
                out.insert(k.clone(), projected_value(v, max_value_chars));
            }
        }
    }
    Value::Object(out)
}

fn projected_value(v: &Value, max_value_chars: usize) -> Value {
    match v {
        Value::String(s) => json!(truncate_chars(s, max_value_chars)),
        Value::Array(_) | Value::Object(_) => {
            json!(truncate_chars(&json_dumps(v, false), max_value_chars))
        }
        other => other.clone(),
    }
}

fn iter_records(path: &Path) -> Vec<(usize, Map<String, Value>)> {
    let suffix = path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let mut out = Vec::new();
    if suffix == "jsonl" || suffix == "ndjson" {
        if let Ok(file) = fs::File::open(path) {
            for (i, line) in BufReader::new(file).lines().enumerate() {
                let Ok(line) = line else { continue };
                if line.trim().is_empty() {
                    continue;
                }
                if let Ok(Value::Object(m)) = serde_json::from_str::<Value>(&line) {
                    out.push((i + 1, m));
                }
            }
        }
    } else if suffix == "json" {
        if let Ok(text) = fs::read_to_string(path) {
            if let Ok(v) = serde_json::from_str::<Value>(&text) {
                match v {
                    Value::Object(m) => out.push((1, m)),
                    Value::Array(xs) => {
                        for (i, rec) in xs.into_iter().enumerate() {
                            if let Value::Object(m) = rec {
                                out.push((i + 1, m));
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    } else if suffix == "csv" || suffix == "tsv" {
        let delim = if suffix == "tsv" { b'\t' } else { b',' };
        if let Ok(mut rdr) = csv::ReaderBuilder::new().delimiter(delim).from_path(path) {
            if let Ok(headers) = rdr.headers().cloned() {
                for (i, rec) in rdr.records().enumerate() {
                    let Ok(rec) = rec else { continue };
                    let mut m = Map::new();
                    for (k, v) in headers.iter().zip(rec.iter()) {
                        m.insert(k.to_string(), json!(v));
                    }
                    out.push((i + 2, m));
                }
            }
        }
    }
    out
}

fn data_file_candidates(
    root: &Path,
    paths: Option<&Value>,
    scopes: Option<&Value>,
    include_globs: Option<&Value>,
    exclude_globs: Option<&Value>,
    max_files: usize,
    timeout_seconds: f64,
) -> (Vec<String>, Map<String, Value>) {
    let mut meta = Map::new();
    let rg = match find_rg() {
        Some(r) => r,
        None => {
            meta.insert("error".to_string(), json!("rg_not_found"));
            return (vec![], meta);
        }
    };
    let (search_paths, rejected) = normalize_paths(root, paths);
    if search_paths.is_empty() {
        meta.insert("error".to_string(), json!("no_valid_paths"));
        meta.insert("rejected_paths".to_string(), Value::Array(rejected));
        return (vec![], meta);
    }
    let explicit = !as_list_value(paths).is_empty();
    let explicit_files = paths_all_files(root, &search_paths);
    let mut includes = as_list_value(include_globs);
    includes.extend(DATA_GLOBS.iter().map(|s| s.to_string()));
    let (globs, warnings) = build_globs(
        scopes.or(Some(&json!(["config", "analysis"]))),
        Some(&json!(includes)),
        exclude_globs,
        explicit,
        explicit_files,
    );
    let mut cmd = vec![rg, "--files".to_string(), "--color=never".to_string()];
    cmd.extend(globs);
    cmd.extend(search_paths);
    let proc = run_lines(
        &cmd,
        root,
        timeout_seconds,
        DEFAULT_BUDGET.process_output_bytes,
        max_files * DEFAULT_BUDGET.file_scan_line_multiplier,
    );
    let mut files = Vec::new();
    for raw in &proc.lines {
        let p = clean_rel(String::from_utf8_lossy(raw).trim());
        if p.is_empty() {
            continue;
        }
        let suffix = Path::new(&p)
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if !matches!(suffix.as_str(), "json" | "jsonl" | "ndjson" | "csv" | "tsv") {
            continue;
        }
        if files.len() >= max_files {
            break;
        }
        files.push(p);
    }
    meta.insert("rejected_paths".to_string(), Value::Array(rejected));
    meta.insert("warnings".to_string(), json!(warnings));
    meta.insert(
        "truncated".to_string(),
        json!(
            proc.timed_out
                || proc.byte_limit_hit
                || proc.line_limit_hit
                || files.len() >= max_files
        ),
    );
    (files, meta)
}

pub fn cxs_json(args: &Value) -> Value {
    let rp = match root_or_error(args) {
        Ok(p) => p,
        Err(v) => return v,
    };
    let maxf = clamp_usize(
        usize_arg(args, "max_files"),
        DEFAULT_BUDGET.min_total_hits,
        DEFAULT_BUDGET.max_total_hits_limit,
        DEFAULT_BUDGET.max_total_hits,
    );
    let maxr = clamp_usize(
        usize_arg(args, "max_records"),
        DEFAULT_BUDGET.min_total_hits,
        DEFAULT_BUDGET.max_total_hits_limit,
        DEFAULT_BUDGET.max_total_hits,
    );
    let maxexam = clamp_usize(
        usize_arg(args, "max_records_examined"),
        DEFAULT_BUDGET.min_total_hits,
        DEFAULT_BUDGET.max_records_examined_limit,
        DEFAULT_BUDGET.max_records_examined,
    );
    let maxfile = clamp_u64(
        u64_arg(args, "max_file_bytes"),
        DEFAULT_BUDGET.min_file_bytes,
        DEFAULT_BUDGET.max_file_bytes_limit,
        DEFAULT_BUDGET.max_file_bytes,
    );
    let maxval = clamp_usize(
        usize_arg(args, "max_value_chars"),
        DEFAULT_BUDGET.min_line_chars,
        DEFAULT_BUDGET.max_line_chars_limit,
        DEFAULT_BUDGET.max_line_chars,
    );
    let maxtotal = clamp_usize(
        usize_arg(args, "max_total_chars"),
        DEFAULT_BUDGET.min_total_chars,
        DEFAULT_BUDGET.max_total_chars_limit,
        DEFAULT_BUDGET.max_total_chars,
    );
    let timeout = clamp_f64(
        f64_arg(args, "timeout_seconds"),
        DEFAULT_BUDGET.min_timeout_seconds,
        DEFAULT_BUDGET.max_json_timeout_seconds,
        DEFAULT_BUDGET.timeout_seconds,
    );
    let (files, meta) = data_file_candidates(
        &rp,
        args.get("paths"),
        args.get("scopes"),
        args.get("include_globs"),
        args.get("exclude_globs"),
        maxf,
        timeout,
    );
    if let Some(e) = meta.get("error") {
        let mut out = Map::new();
        out.insert("status".to_string(), json!("error"));
        out.insert("error".to_string(), e.clone());
        for (k, v) in meta {
            if k != "error" {
                out.insert(k, v);
            }
        }
        return Value::Object(out);
    }
    let query = str_arg(args, "query");
    let mut qs = Vec::new();
    if let Some(q) = &query {
        if !q.is_empty() {
            qs.push(q.clone());
        }
    }
    qs.extend(as_list_value(args.get("terms")));
    qs.retain(|s| !s.is_empty());
    let filters = args
        .get("filters")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let match_mode = str_arg(args, "match").unwrap_or_else(|| "any".to_string());
    let case_sensitive = opt_bool_arg(args, "case_sensitive");
    let search_large_fields = bool_arg(args, "search_large_fields", false);
    let allow_large_files = bool_arg(args, "allow_large_files", false);
    let mut hits = Vec::new();
    let mut examined = 0_usize;
    let mut total_chars = 0_usize;
    let mut truncated = meta
        .get("truncated")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let start = Instant::now();
    for rel in &files {
        if start.elapsed().as_secs_f64() > timeout {
            truncated = true;
            break;
        }
        let p = rp.join(rel);
        if !allow_large_files {
            if let Ok(md) = fs::metadata(&p) {
                if md.len() > maxfile {
                    continue;
                }
            }
        }
        for (line_no, rec) in iter_records(&p) {
            examined += 1;
            if examined > maxexam {
                truncated = true;
                break;
            }
            let mut ok = true;
            for (k, v) in &filters {
                let lhs = rec.get(k).map(value_to_plain_string).unwrap_or_default();
                let rhs = value_to_plain_string(v);
                if lhs != rhs {
                    ok = false;
                    break;
                }
            }
            if !ok {
                continue;
            }
            if !qs.is_empty() {
                let hay = flatten_text(&Value::Object(rec.clone()), search_large_fields);
                if case_sensitive == Some(true) {
                    ok = if match_mode == "all" {
                        qs.iter().all(|q| hay.contains(q))
                    } else {
                        qs.iter().any(|q| hay.contains(q))
                    };
                } else {
                    let hay_cmp = hay.to_lowercase();
                    let qs_cmp: Vec<String> = qs.iter().map(|q| q.to_lowercase()).collect();
                    ok = if match_mode == "all" {
                        qs_cmp.iter().all(|q| hay_cmp.contains(q))
                    } else {
                        qs_cmp.iter().any(|q| hay_cmp.contains(q))
                    };
                }
                if !ok {
                    continue;
                }
            }
            let proj = project_record(&rec, args.get("fields"), args.get("omit_fields"), maxval);
            let entry = json!({"file": rel, "line": line_no, "record": proj});
            let entry_chars = json_dumps(&entry, false).chars().count();
            if hits.len() >= maxr || total_chars + entry_chars > maxtotal {
                truncated = true;
                break;
            }
            total_chars += entry_chars;
            hits.push(entry);
        }
        if hits.len() >= maxr || examined > maxexam {
            break;
        }
    }
    let mut out = Map::new();
    out.insert(
        "status".to_string(),
        json!(if truncated { "truncated" } else { "ok" }),
    );
    out.insert(
        "query".to_string(),
        query.map(Value::String).unwrap_or(Value::Null),
    );
    out.insert("stats".to_string(), json!({"files": files.len(), "records_examined": examined, "hits": hits.len(), "truncated": truncated}));
    out.insert("hits".to_string(), Value::Array(hits));
    if let Some(v) = meta
        .get("rejected_paths")
        .filter(|v| v.as_array().map(|a| !a.is_empty()).unwrap_or(false))
    {
        out.insert("rejected_paths".to_string(), v.clone());
    }
    if let Some(v) = meta
        .get("warnings")
        .filter(|v| v.as_array().map(|a| !a.is_empty()).unwrap_or(false))
    {
        out.insert("warnings".to_string(), v.clone());
    }
    Value::Object(out)
}

pub fn cxs_self_check(args: &Value) -> Value {
    let rp = match root_or_error(args) {
        Ok(p) => p,
        Err(v) => return v,
    };
    let rg = find_rg();
    let mut out = Map::new();
    out.insert(
        "status".to_string(),
        json!(if rg.is_some() { "ok" } else { "error" }),
    );
    out.insert("version".to_string(), json!(VERSION));
    out.insert("root".to_string(), json!(rp.to_string_lossy()));
    out.insert("runtime".to_string(), json!("rust"));
    out.insert(
        "rg".to_string(),
        rg.clone().map(Value::String).unwrap_or(Value::Null),
    );
    out.insert("default_scopes".to_string(), json!(DEFAULT_SCOPES));
    if let Some(rg_path) = rg {
        let proc = run_lines(
            &[rg_path, "--version".to_string()],
            &rp,
            DEFAULT_BUDGET.self_check_rg_timeout_seconds,
            DEFAULT_BUDGET.process_output_bytes,
            2,
        );
        if let Some(first) = proc.lines.first() {
            out.insert(
                "rg_version".to_string(),
                json!(String::from_utf8_lossy(first).to_string()),
            );
        } else {
            out.insert("rg_version".to_string(), json!("unknown"));
        }
    } else {
        out.insert("error".to_string(), json!("rg_not_found"));
    }
    Value::Object(out)
}

pub fn call_op(op: &str, args: &Value) -> Value {
    match op {
        "find" | "cxs_find" => cxs_find(args),
        "files" | "cxs_files" => cxs_files(args),
        "symbol" | "cxs_symbol" => cxs_symbol(args),
        "json" | "cxs_json" => cxs_json(args),
        "self_check" | "cxs_self_check" => cxs_self_check(args),
        _ => make_base_response(
            "error",
            vec![
                ("error", json!("bad_op")),
                (
                    "valid_ops",
                    json!(["find", "files", "symbol", "json", "self_check"]),
                ),
            ],
        ),
    }
}

pub fn shim_rg_main(args: &[String]) -> i32 {
    let exe = env::current_exe().ok().and_then(|p| p.canonicalize().ok());
    if env::var_os("CXS_RAW_RG").is_some() || env::var_os("RG_RAW").is_some() {
        if let Some(real) = find_rg_excluding(exe.as_deref()) {
            return Command::new(real)
                .args(args)
                .status()
                .ok()
                .and_then(|s| s.code())
                .unwrap_or(1);
        }
        eprintln!("rg not found");
        return 127;
    }
    if args
        .iter()
        .any(|a| matches!(a.as_str(), "--files" | "-l" | "--files-with-matches"))
    {
        if let Some(real) = find_rg_excluding(exe.as_deref()) {
            return Command::new(real)
                .args(args)
                .status()
                .ok()
                .and_then(|s| s.code())
                .unwrap_or(1);
        }
        return 127;
    }
    let numbered = args
        .iter()
        .any(|a| a == "-n" || a.starts_with("-n") || a == "--line-number");
    if numbered {
        let mut pats = Vec::new();
        let mut paths = Vec::new();
        let mut mode = "regex".to_string();
        let mut i = 0;
        let mut unsupported = false;
        while i < args.len() {
            let a = &args[i];
            if matches!(
                a.as_str(),
                "-n" | "--line-number" | "--no-heading" | "--color=never" | "-S" | "--smart-case"
            ) {
                i += 1;
                continue;
            }
            if matches!(a.as_str(), "-F" | "--fixed-strings") {
                mode = "literal".to_string();
                i += 1;
                continue;
            }
            if (a == "-e" || a == "--regexp") && i + 1 < args.len() {
                pats.push(args[i + 1].clone());
                i += 2;
                continue;
            }
            if a.starts_with('-') {
                unsupported = true;
                break;
            }
            if pats.is_empty() {
                pats.push(a.clone());
            } else {
                paths.push(a.clone());
            }
            i += 1;
        }
        if !unsupported {
            if let Some(exe_path) = exe {
                // The shim may itself be named rg and precede the real rg in PATH.
                // Set this guard for the library lookup before it shells out again.
                unsafe {
                    env::set_var("CXS_SHIM_PATH", exe_path);
                }
            }
            let mut m = Map::new();
            m.insert("terms".to_string(), json!(pats));
            m.insert("mode".to_string(), json!(mode));
            if !paths.is_empty() {
                m.insert("paths".to_string(), json!(paths));
            }
            let res = cxs_find(&Value::Object(m));
            println!("{}", json_dumps(&res, false));
            return if res.get("status").and_then(Value::as_str) == Some("error") {
                1
            } else {
                0
            };
        }
    }
    if let Some(real) = find_rg_excluding(exe.as_deref()) {
        let mut guard = vec![
            "--max-count".to_string(),
            DEFAULT_BUDGET.max_hits_per_file.to_string(),
            "--max-columns".to_string(),
            DEFAULT_BUDGET.max_line_chars.to_string(),
            "--max-columns-preview".to_string(),
        ];
        guard.extend(args.iter().cloned());
        return Command::new(real)
            .args(guard)
            .status()
            .ok()
            .and_then(|s| s.code())
            .unwrap_or(1);
    }
    eprintln!("rg not found");
    127
}
