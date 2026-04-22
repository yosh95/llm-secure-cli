use llm_secure_cli::tools::builtin::file_modification::{create_or_overwrite_file, edit_file};
use llm_secure_cli::tools::builtin::file_ops::{
    grep_files, list_files_in_directory, read_file_content, search_files,
};
use llm_secure_cli::tools::builtin::shell::execute_command;
use serde_json::json;
use std::collections::HashMap;
use std::fs;
use tempfile::tempdir;

#[test]
fn test_file_ops_list_and_search() {
    let dir = tempdir().unwrap();
    let root = dir.path();

    // Setup directory structure
    // root/
    //   file1.txt
    //   subdir/
    //     file2.rs
    //     .hidden_file
    fs::write(root.join("file1.txt"), "hello world").unwrap();
    fs::create_dir(root.join("subdir")).unwrap();
    fs::write(
        root.join("subdir").join("file2.rs"),
        "fn main() { println!(\"test\"); }",
    )
    .unwrap();
    fs::write(root.join("subdir").join(".hidden_file"), "secret").unwrap();

    let root_str = root.to_str().unwrap();

    // 1. Test list_files_in_directory (depth 1)
    let mut args = HashMap::new();
    args.insert("directory".to_string(), json!(root_str));
    args.insert("depth".to_string(), json!(1));
    let res = list_files_in_directory(args).unwrap();
    let files = res["files"].as_array().unwrap();
    assert!(files.iter().any(|f| f["path"] == "file1.txt"));
    assert!(files.iter().any(|f| f["path"] == "subdir"));
    assert!(!files.iter().any(|f| f["path"] == "subdir/file2.rs")); // depth 1

    // 2. Test list_files_in_directory (recursive)
    let mut args = HashMap::new();
    args.insert("directory".to_string(), json!(root_str));
    args.insert("depth".to_string(), json!(2));
    let res = list_files_in_directory(args).unwrap();
    let files = res["files"].as_array().unwrap();
    assert!(files.iter().any(|f| f["path"] == "subdir/file2.rs"));
    assert!(!files.iter().any(|f| f["path"] == "subdir/.hidden_file")); // hidden excluded by default

    // 3. Test list_files_in_directory (include hidden)
    let mut args = HashMap::new();
    args.insert("directory".to_string(), json!(root_str));
    args.insert("depth".to_string(), json!(2));
    args.insert("include_hidden".to_string(), json!(true));
    let res = list_files_in_directory(args).unwrap();
    let files = res["files"].as_array().unwrap();
    assert!(files.iter().any(|f| f["path"] == "subdir/.hidden_file"));

    // 4. Test search_files
    let mut args = HashMap::new();
    args.insert("directory".to_string(), json!(root_str));
    args.insert("pattern".to_string(), json!("*.rs"));
    let res = search_files(args.clone()).unwrap();
    let results = res["results"].as_array().unwrap();
    assert!(results.iter().any(|r| r["path"] == "subdir/file2.rs"));

    // 5. Test search_files with *middle* pattern
    let mut args = HashMap::new();
    args.insert("directory".to_string(), json!(root_str));
    args.insert("pattern".to_string(), json!("*file*"));
    let res = search_files(args).unwrap();
    let results = res["results"].as_array().unwrap();
    assert!(results.iter().any(|r| r["path"] == "file1.txt"));
    assert!(results.iter().any(|r| r["path"] == "subdir/file2.rs"));
}

#[test]
fn test_file_ops_grep() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    fs::write(root.join("test.txt"), "line one\ntarget line\nline three").unwrap();
    let root_str = root.to_str().unwrap();

    let mut args = HashMap::new();
    args.insert("directory".to_string(), json!(root_str));
    args.insert("query".to_string(), json!("target"));
    let res = grep_files(args).unwrap();
    let matches = res["matches"].as_array().unwrap();
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0]["file"], "test.txt");
    assert_eq!(matches[0]["line"], 2);
    assert_eq!(matches[0]["text"], "target line");
}

#[test]
fn test_read_file_content_options() {
    let dir = tempdir().unwrap();
    let file_path = dir.path().join("test.txt");
    fs::write(&file_path, "line1\nline2\nline3\nline4").unwrap();
    let path_str = file_path.to_str().unwrap();

    // Test with line numbers
    let mut args = HashMap::new();
    args.insert("path".to_string(), json!(path_str));
    args.insert("start_line".to_string(), json!(2));
    args.insert("end_line".to_string(), json!(3));
    args.insert("with_line_numbers".to_string(), json!(true));

    let res = read_file_content(args).unwrap();
    let content = res.as_str().unwrap();
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
    args.insert("command".to_string(), json!("echo"));
    args.insert("args".to_string(), json!(["hello", "world"]));

    let res = execute_command(args).unwrap();
    assert_eq!(res["stdout"].as_str().unwrap().trim(), "hello world");
    assert_eq!(res["exit_code"].as_i64().unwrap(), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_shell_security_block() {
    let mut args = HashMap::new();
    args.insert("command".to_string(), json!("rm"));
    args.insert("args".to_string(), json!(["-rf", "/"]));

    let res = execute_command(args);
    assert!(res.is_err());
    assert!(res.unwrap_err().to_string().contains("Security Blocked"));
}

#[test]
fn test_file_modification_tools() {
    let dir = tempdir().unwrap();
    let file_path = dir.path().join("test.txt");
    let path_str = file_path.to_str().unwrap();

    // 1. Create file
    let mut args = HashMap::new();
    args.insert("path".to_string(), json!(path_str));
    args.insert("content".to_string(), json!("line1\nline2\nline3"));

    let res = create_or_overwrite_file(args).expect("create_or_overwrite_file failed");
    assert!(res["success"].as_bool().unwrap());
    assert_eq!(
        fs::read_to_string(&file_path).unwrap(),
        "line1\nline2\nline3"
    );

    // 2. Edit file (exact)
    let mut args = HashMap::new();
    args.insert("path".to_string(), json!(path_str));
    args.insert("search".to_string(), json!("line2"));
    args.insert("replace".to_string(), json!("line2 modified"));

    let res = edit_file(args).expect("edit_file failed");
    assert!(res["success"].as_bool().unwrap());
    assert_eq!(
        fs::read_to_string(&file_path).unwrap(),
        "line1\nline2 modified\nline3"
    );

    // 3. Edit file (fuzzy match)
    let mut args = HashMap::new();
    args.insert("path".to_string(), json!(path_str));
    args.insert("search".to_string(), json!("  line3  ")); // Whitespace difference
    args.insert("replace".to_string(), json!("line3 modified"));

    let res = edit_file(args).expect("Fuzzy match should now succeed");
    assert!(res["success"].as_bool().unwrap());
    assert_eq!(res["match_type"].as_str().unwrap(), "fuzzy");
    assert_eq!(
        fs::read_to_string(&file_path).unwrap(),
        "line1\nline2 modified\nline3 modified"
    );
}

#[test]
fn test_edit_file_not_found() {
    let mut args = HashMap::new();
    args.insert("path".to_string(), json!("non_existent_file.txt"));
    args.insert("search".to_string(), json!("search"));
    args.insert("replace".to_string(), json!("replace"));

    let res = edit_file(args);
    assert!(res.is_err());
}

#[test]
fn test_read_file_content_range_panic_fix() {
    let dir = tempdir().unwrap();
    let file_path = dir.path().join("test_read.txt");
    fs::write(&file_path, "1\n2\n3\n4\n5\n6\n7\n8\n9\n10").unwrap();
    let path_str = file_path.to_str().unwrap();

    let mut args = HashMap::new();
    args.insert("path".to_string(), json!(path_str));
    args.insert("start_line".to_string(), json!(8));
    args.insert("end_line".to_string(), json!(3));

    let res = read_file_content(args)
        .expect("read_file_content should return Ok even with invalid range");
    let error_msg = res.as_str().unwrap();
    assert!(error_msg.contains("Error: start_line (8) is greater than end_line (3)"));
}
