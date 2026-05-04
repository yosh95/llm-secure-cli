use llm_secure_cli::config::models::AppConfig;
use llm_secure_cli::tools::builtin::web::read_url_content;
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;

#[tokio::test]
async fn test_url_ssrf_protection() {
    let mut args = HashMap::new();
    let config = Arc::new(AppConfig::default());

    // 1. Block localhost
    args.insert("url".to_string(), json!("http://localhost/config"));
    let res = read_url_content(args.clone(), config.clone()).await;
    match res {
        Err(e) => assert!(e.to_string().contains("SSRF protection")),
        Ok(_) => panic!("Should have blocked localhost"),
    }

    // 2. Block private IP
    args.insert("url".to_string(), json!("http://192.168.1.1/admin"));
    let res = read_url_content(args.clone(), config.clone()).await;
    match res {
        Err(e) => assert!(e.to_string().contains("SSRF protection")),
        Ok(_) => panic!("Should have blocked private IP"),
    }

    // 3. Block file scheme
    args.insert("url".to_string(), json!("file:///etc/passwd"));
    let res = read_url_content(args, config).await;
    match res {
        Err(e) => assert!(e.to_string().contains("Only http/https")),
        Ok(_) => panic!("Should have blocked file scheme"),
    }
}
