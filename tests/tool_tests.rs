use llm_secure_cli::modules::tools::file_modification::{create_or_overwrite_file, edit_file};
use serde_json::json;
use std::collections::HashMap;
use std::fs;
use tempfile::tempdir;

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
    assert_eq!(fs::read_to_string(&file_path).unwrap(), "line1\nline2\nline3");

    // 2. Edit file (exact)
    let mut args = HashMap::new();
    args.insert("path".to_string(), json!(path_str));
    args.insert("search".to_string(), json!("line2"));
    args.insert("replace".to_string(), json!("line2 modified"));

    let res = edit_file(args).expect("edit_file failed");
    assert!(res["success"].as_bool().unwrap());
    assert_eq!(res["match_type"].as_str().unwrap(), "exact");
    assert_eq!(fs::read_to_string(&file_path).unwrap(), "line1\nline2 modified\nline3");

    // 3. Edit file (fuzzy)
    let mut args = HashMap::new();
    args.insert("path".to_string(), json!(path_str));
    args.insert("search".to_string(), json!("  line3  ")); // Whitespace difference
    args.insert("replace".to_string(), json!("line3 modified"));

    let res = edit_file(args).expect("edit_file (fuzzy) failed");
    assert!(res["success"].as_bool().unwrap());
    assert_eq!(res["match_type"].as_str().unwrap(), "fuzzy");
    // Indentation from search block is preserved
    assert_eq!(fs::read_to_string(&file_path).unwrap(), "line1\nline2 modified\n  line3 modified");
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
