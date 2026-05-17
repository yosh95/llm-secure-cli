use llm_secure_cli::config::models::AppConfig;
use llm_secure_cli::tools::builtin::file_modification::{create_or_overwrite_file, edit_file};
use llm_secure_cli::tools::builtin::file_ops::{
    grep_files, list_files_in_directory, read_file, search_files,
};
use llm_secure_cli::tools::builtin::python::execute_python;
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
fn test_read_file_options() {
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

    let res = read_file(args, config).expect("read_file should succeed");
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
async fn test_python_execute() {
    let mut args = HashMap::new();
    args.insert("code".to_string(), json!("print('hello world')"));

    let config = Arc::new(AppConfig::default());
    let res = execute_python(args, config)
        .await
        .expect("execute_python should succeed");
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
async fn test_python_security_block_null_bytes() {
    use llm_secure_cli::security::validate_tool_call;

    let mut args = serde_json::Map::new();
    let code_with_null = format!("print('hello{}world')", '\0');
    args.insert("code".to_string(), json!(code_with_null));

    let config = AppConfig::default();
    // Phase 1 (StaticAnalyzer) blocks null bytes in the code string.
    let res = validate_tool_call("execute_python", &args, &config.security);
    assert!(res.is_err());
    assert!(
        res.expect_err("should be Err")
            .contains("control characters or null bytes")
    );
}

#[test]
fn test_static_analyzer_blocks_null_bytes_in_code() {
    use llm_secure_cli::security::static_analyzer::StaticAnalyzer;

    // Normal strings should be allowed
    assert!(!StaticAnalyzer::is_obviously_malicious("print('hello')"));

    // Null bytes should be caught
    let code_with_null = format!("print('hello{}world')", '\0');
    assert!(StaticAnalyzer::is_obviously_malicious(&code_with_null));
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
    args.insert("old".to_string(), json!("line2"));
    args.insert("new".to_string(), json!("line2 modified"));

    let res = edit_file(args, config.clone()).expect("edit_file failed");
    assert!(res["success"].as_bool().expect("success should be a bool"));
    assert_eq!(
        fs::read_to_string(&file_path).expect("Failed to read file"),
        "line1\nline2 modified\nline3"
    );

    // 3. Edit file (fuzzy match)
    let mut args = HashMap::new();
    args.insert("path".to_string(), json!(path_str));
    args.insert("old".to_string(), json!("  line3  ")); // Whitespace difference
    args.insert("new".to_string(), json!("line3 modified"));

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
    args.insert("old".to_string(), json!("search"));
    args.insert("new".to_string(), json!("replace"));

    let config = Arc::new(AppConfig::default());
    let res = edit_file(args, config);
    assert!(res.is_err());
}

#[test]
fn test_read_file_range_panic_fix() {
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

    let res = read_file(args, config).expect("read_file should return Ok even with invalid range");
    let error_msg = res["error"]
        .as_str()
        .expect("Output should contain 'error' field");
    assert!(error_msg.contains("start_line (8) is greater than end_line (3)"));
}

#[tokio::test(flavor = "multi_thread")]
async fn test_python_execute_with_error() {
    // Python with a syntax error should return non-zero exit code
    let mut args = HashMap::new();
    args.insert("code".to_string(), json!("print(undefined_var"));

    let config = Arc::new(AppConfig::default());
    let res = execute_python(args, config).await;
    // Should succeed at the tool level (python ran, just had an error)
    assert!(res.is_ok());
    let val = res.unwrap();
    let exit_code = val["exit_code"]
        .as_i64()
        .expect("exit_code should be an i64");
    assert_ne!(exit_code, 0);
    let stderr = val["stderr"].as_str().expect("stderr should be a string");
    assert!(!stderr.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn test_python_execute_multiline() {
    let mut args = HashMap::new();
    args.insert(
        "code".to_string(),
        json!("import json\ndata = {'key': 'value'}\nprint(json.dumps(data))"),
    );

    let config = Arc::new(AppConfig::default());
    let res = execute_python(args, config)
        .await
        .expect("execute_python should succeed");
    assert_eq!(
        res["exit_code"]
            .as_i64()
            .expect("exit_code should be an i64"),
        0
    );
    let stdout = res["stdout"]
        .as_str()
        .expect("stdout should be a string")
        .trim();
    let parsed: serde_json::Value =
        serde_json::from_str(stdout).expect("stdout should be valid JSON");
    assert_eq!(parsed["key"], "value");
}
