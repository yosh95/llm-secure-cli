use llm_secure_cli::config::models::AppConfig;
use llm_secure_cli::tools::builtin::file_modification::{create_or_overwrite_file, edit_file};
use llm_secure_cli::tools::builtin::file_ops::{
    grep_files, list_files_in_directory, read_file_content, search_files,
};
use llm_secure_cli::tools::builtin::shell::execute_command;
use serde_json::json;
use std::collections::HashMap;
use std::fs;
use std::sync::Arc;
use tempfile::tempdir;

#[test]
fn test_file_ops_list_and_search() {
    let dir = tempdir().expect("Failed to create temp dir");
    let root = dir.path();

    // Setup allowed paths for test
    let mut config_raw = AppConfig::default();
    let canon_path = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    config_raw.security.allowed_paths =
        vec![".".to_string(), canon_path.to_string_lossy().to_string()];
    let config = Arc::new(config_raw);

    // Setup directory structure
    // root/
    //   file1.txt
    //   subdir/
    //     file2.rs
    //     .hidden_file
    fs::write(root.join("file1.txt"), "hello world").expect("Failed to write file1.txt");
    fs::create_dir(root.join("subdir")).expect("Failed to create subdir");
    fs::write(
        root.join("subdir").join("file2.rs"),
        "fn main() { println!(\"test\"); }",
    )
    .expect("Failed to write file2.rs");
    fs::write(root.join("subdir").join(".hidden_file"), "secret")
        .expect("Failed to write .hidden_file");

    let root_str = root.to_str().expect("root path should be valid UTF-8");

    // 1. Test list_files_in_directory (depth 1)
    let mut args = HashMap::new();
    args.insert("directory".to_string(), json!(root_str));
    args.insert("depth".to_string(), json!(1));
    let res = list_files_in_directory(args, config.clone())
        .expect("list_files_in_directory should succeed");
    let files = res["files"].as_array().expect("files should be an array");
    assert!(files.iter().any(|f| f["path"] == "file1.txt"));
    assert!(files.iter().any(|f| f["path"] == "subdir"));
    assert!(!files.iter().any(|f| f["path"] == "subdir/file2.rs")); // depth 1

    // 2. Test list_files_in_directory (recursive)
    let mut args = HashMap::new();
    args.insert("directory".to_string(), json!(root_str));
    args.insert("depth".to_string(), json!(2));
    let res = list_files_in_directory(args, config.clone())
        .expect("list_files_in_directory (recursive) should succeed");
    let files = res["files"].as_array().expect("files should be an array");
    assert!(files.iter().any(|f| f["path"] == "subdir/file2.rs"));
    assert!(!files.iter().any(|f| f["path"] == "subdir/.hidden_file")); // hidden excluded by default

    // 3. Test list_files_in_directory (include hidden)
    let mut args = HashMap::new();
    args.insert("directory".to_string(), json!(root_str));
    args.insert("depth".to_string(), json!(2));
    args.insert("include_hidden".to_string(), json!(true));
    let res = list_files_in_directory(args, config.clone())
        .expect("list_files_in_directory (include_hidden) should succeed");
    let files = res["files"].as_array().expect("files should be an array");
    assert!(files.iter().any(|f| f["path"] == "subdir/.hidden_file"));

    // 4. Test search_files
    let mut args = HashMap::new();
    args.insert("directory".to_string(), json!(root_str));
    args.insert("pattern".to_string(), json!("*.rs"));
    let res = search_files(args.clone(), config.clone()).expect("search_files should succeed");
    let results = res["results"]
        .as_array()
        .expect("results should be an array");
    assert!(results.iter().any(|r| r["path"] == "subdir/file2.rs"));

    // 5. Test search_files with *middle* pattern
    let mut args = HashMap::new();
    args.insert("directory".to_string(), json!(root_str));
    args.insert("pattern".to_string(), json!("*file*"));
    let res = search_files(args, config.clone()).expect("search_files should succeed");
    let results = res["results"]
        .as_array()
        .expect("results should be an array");
    assert!(results.iter().any(|r| r["path"] == "file1.txt"));
    assert!(results.iter().any(|r| r["path"] == "subdir/file2.rs"));
}

#[test]
fn test_file_ops_grep() {
    let dir = tempdir().expect("Failed to create temp dir");
    let root = dir.path();
    let root_str = root.to_str().expect("root path should be valid UTF-8");

    let mut config_raw = AppConfig::default();
    config_raw.security.allowed_paths = vec![".".to_string(), root_str.to_string()];
    let config = Arc::new(config_raw);

    fs::write(root.join("test.txt"), "line one\ntarget line\nline three")
        .expect("Failed to write test file");

    let mut args = HashMap::new();
    args.insert("directory".to_string(), json!(root_str));
    args.insert("query".to_string(), json!("target"));
    let res = grep_files(args, config).expect("grep_files should succeed");
    let matches = res["matches"]
        .as_array()
        .expect("matches should be an array");
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0]["file"], "test.txt");
    assert_eq!(matches[0]["line"], 2);
    assert_eq!(matches[0]["text"], "target line");
}

#[test]
fn test_read_file_content_options() {
    let dir = tempdir().expect("Failed to create temp dir");
    let file_path = dir.path().join("test.txt");
    let path_str = file_path.to_str().expect("file path should be valid UTF-8");

    let mut config_raw = AppConfig::default();
    // Canonicalize the allowed path to match validator behavior
    let canon_path = std::fs::canonicalize(dir.path()).unwrap_or_else(|_| dir.path().to_path_buf());
    config_raw.security.allowed_paths =
        vec![".".to_string(), canon_path.to_string_lossy().to_string()];
    let config = Arc::new(config_raw);

    fs::write(&file_path, "line1\nline2\nline3\nline4").expect("Failed to write test file");

    // Test with line numbers
    let mut args = HashMap::new();
    args.insert("path".to_string(), json!(path_str));
    args.insert("start_line".to_string(), json!(2));
    args.insert("end_line".to_string(), json!(3));
    args.insert("with_line_numbers".to_string(), json!(true));

    let res = read_file_content(args, config).expect("read_file_content should succeed");
    let content = res["content"]
        .as_str()
        .expect("Output should contain 'content' field");
    assert!(content.contains("   2 | line2"));
    assert!(content.contains("   3 | line3"));
    assert!(!content.contains("line1"));
    assert!(!contains_exact_line(content, "line4"));
}

fn contains_exact_line(content: &str, line: &str) -> bool {
    content.lines().any(|l| l.trim().ends_with(line))
}

#[tokio::test(flavor = "multi_thread")]
async fn test_shell_execute_command() {
    let mut args = HashMap::new();
    if cfg!(windows) {
        args.insert("command".to_string(), json!("cmd"));
        args.insert("args".to_string(), json!(["/C", "echo hello world"]));
    } else {
        args.insert("command".to_string(), json!("echo"));
        args.insert("args".to_string(), json!(["hello", "world"]));
    }

    let config = Arc::new(AppConfig::default());
    let res = execute_command(args, config)
        .await
        .expect("execute_command should succeed");
    assert_eq!(
        res["stdout"]
            .as_str()
            .expect("stdout should be a string")
            .trim(),
        "hello world"
    );
    assert_eq!(
        res["exit_code"]
            .as_i64()
            .expect("exit_code should be an i64"),
        0
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn test_shell_security_block() {
    use llm_secure_cli::security::validate_tool_call;

    let mut args = serde_json::Map::new();
    args.insert("command".to_string(), json!("rm\0"));
    args.insert("args".to_string(), json!(["-rf", "/"]));

    let config = AppConfig::default();
    // Phase 1 (StaticAnalyzer) now actively blocks null bytes in the command name.
    let res = validate_tool_call("execute_command", &args, &config.security);
    assert!(res.is_err());
    assert!(
        res.expect_err("should be Err")
            .contains("control characters or null bytes")
    );
}

#[test]
fn test_static_analyzer_blocks_shell_invocation() {
    use llm_secure_cli::security::static_analyzer::StaticAnalyzer;

    // Normal commands should be allowed
    let (safe, _) = StaticAnalyzer::check("git", &["log".to_string(), "--oneline".to_string()]);
    assert!(safe);

    // StaticAnalyzer::check itself still implements the check,
    // even if not called from Phase 1 anymore.
    let (safe, violations) = StaticAnalyzer::check("echo", &["hello\0world".to_string()]);
    assert!(!safe);
    assert!(violations.iter().any(|v| v.contains("control characters")));
}

#[test]
fn test_file_modification_tools() {
    let dir = tempdir().expect("Failed to create temp dir");
    let file_path = dir.path().join("test.txt");
    let path_str = file_path.to_str().expect("file path should be valid UTF-8");

    let mut config_raw = AppConfig::default();
    let canon_path = std::fs::canonicalize(dir.path()).unwrap_or_else(|_| dir.path().to_path_buf());
    config_raw.security.allowed_paths =
        vec![".".to_string(), canon_path.to_string_lossy().to_string()];
    let config = Arc::new(config_raw);

    // 1. Create file
    let mut args = HashMap::new();
    args.insert("path".to_string(), json!(path_str));
    args.insert("content".to_string(), json!("line1\nline2\nline3"));

    let res =
        create_or_overwrite_file(args, config.clone()).expect("create_or_overwrite_file failed");
    assert!(res["success"].as_bool().expect("success should be a bool"));
    assert_eq!(
        fs::read_to_string(&file_path).expect("Failed to read file"),
        "line1\nline2\nline3"
    );

    // 2. Edit file (exact)
    let mut args = HashMap::new();
    args.insert("path".to_string(), json!(path_str));
    args.insert("search".to_string(), json!("line2"));
    args.insert("replace".to_string(), json!("line2 modified"));

    let res = edit_file(args, config.clone()).expect("edit_file failed");
    assert!(res["success"].as_bool().expect("success should be a bool"));
    assert_eq!(
        fs::read_to_string(&file_path).expect("Failed to read file"),
        "line1\nline2 modified\nline3"
    );

    // 3. Edit file (fuzzy match)
    let mut args = HashMap::new();
    args.insert("path".to_string(), json!(path_str));
    args.insert("search".to_string(), json!("  line3  ")); // Whitespace difference
    args.insert("replace".to_string(), json!("line3 modified"));

    let res = edit_file(args, config.clone()).expect("Fuzzy match should now succeed");
    assert!(res["success"].as_bool().expect("success should be a bool"));
    assert_eq!(
        res["match_type"]
            .as_str()
            .expect("match_type should be a string"),
        "flexible"
    );
    assert_eq!(
        fs::read_to_string(&file_path).expect("Failed to read file"),
        "line1\nline2 modified\nline3 modified"
    );
}

#[test]
fn test_edit_file_not_found() {
    let mut args = HashMap::new();
    args.insert("path".to_string(), json!("non_existent_file.txt"));
    args.insert("search".to_string(), json!("search"));
    args.insert("replace".to_string(), json!("replace"));

    let config = Arc::new(AppConfig::default());
    let res = edit_file(args, config);
    assert!(res.is_err());
}

#[test]
fn test_read_file_content_range_panic_fix() {
    let dir = tempdir().expect("Failed to create temp dir");
    let file_path = dir.path().join("test_read.txt");
    let path_str = file_path.to_str().expect("file path should be valid UTF-8");

    let mut config_raw = AppConfig::default();
    config_raw.security.allowed_paths = vec![".".to_string(), path_str.to_string()];
    let config = Arc::new(config_raw);

    fs::write(&file_path, "1\n2\n3\n4\n5\n6\n7\n8\n9\n10").expect("Failed to write test file");

    let mut args = HashMap::new();
    args.insert("path".to_string(), json!(path_str));
    args.insert("start_line".to_string(), json!(8));
    args.insert("end_line".to_string(), json!(3));

    let res = read_file_content(args, config)
        .expect("read_file_content should return Ok even with invalid range");
    let error_msg = res["error"]
        .as_str()
        .expect("Output should contain 'error' field");
    assert!(error_msg.contains("start_line (8) is greater than end_line (3)"));
}

#[tokio::test(flavor = "multi_thread")]
async fn test_shell_operator_standalone_blocked() {
    // Standalone shell operators are now passed as literal arguments
    // unless Dual LLM blocks them (Phase 3).
    let mut args = HashMap::new();
    args.insert("command".to_string(), json!("echo"));
    args.insert("args".to_string(), json!(["hello", ";", "rm", "-rf", "/"]));
    let config = Arc::new(AppConfig::default());
    let res = execute_command(args, config).await;
    // Should be Ok in the tool level execution (echo will just print them)
    assert!(res.is_ok());
}

#[tokio::test(flavor = "multi_thread")]
async fn test_shell_operator_embedded_allowed() {
    // Shell operators embedded within a larger argument value (like ffmpeg filter graphs)
    // should NOT be blocked, since they are not standalone shell operators.
    let mut args = HashMap::new();
    args.insert("command".to_string(), json!("ffmpeg"));
    args.insert(
        "args".to_string(),
        json!([
            "-i",
            "input.mp4",
            "-filter_complex",
            "fps=10,scale=640:-1:flags=lanczos,split[s0][s1];[s0]palettegen[p];[s1][p]paletteuse",
            "-y",
            "output.gif"
        ]),
    );
    let config = Arc::new(AppConfig::default());
    // Should not error on the embedded semicolons
    // (ffmpeg itself may not exist, so we only check it doesn't fail with shell operator error)
    let res = execute_command(args, config).await;
    if let Err(e) = res {
        let msg = e.to_string();
        assert!(
            !msg.contains("Shell operator"),
            "Embedded shell operators should not be blocked: {}",
            msg
        );
    }
}
