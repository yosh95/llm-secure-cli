use crate::consts::AUDIT_LOG_PATH;
use crate::security::pqc::ResponseSigner;
use chrono::Utc;
use hostname;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::{self, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;
use uuid::Uuid;

#[derive(Serialize, Deserialize, Clone)]
pub struct AuditEntry {
    pub timestamp: String,
    pub trace_id: String,
    pub subject: String,
    pub audience: String,
    pub model: String,
    pub provider: String,
    pub event_type: String,
    pub tool: String,
    pub args: serde_json::Value,
    pub pqc_confidential: bool,
    pub output: Option<String>,
    pub status: String,
    pub exit_code: Option<i32>,
    pub prev_hash: String,
    pub hash: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pqc_signature: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pqc_algorithm: Option<String>,
    pub hostname: String,
    pub os: String,
    pub arch: String,
    pub cli_version: String,
}

pub struct AuditParams<'a> {
    pub event_type: &'a str,
    pub tool_name: &'a str,
    pub args: serde_json::Value,
    pub output: Option<&'a str>,
    pub exit_code: Option<i32>,
    pub error: Option<&'a str>,
    pub context: Option<&'a serde_json::Value>,
    pub config: &'a crate::config::models::AppConfig,
}

pub struct AuditParamsBuilder<'a> {
    params: AuditParams<'a>,
}

impl<'a> AuditParamsBuilder<'a> {
    pub fn new(
        event_type: &'a str,
        tool_name: &'a str,
        config: &'a crate::config::models::AppConfig,
    ) -> Self {
        Self {
            params: AuditParams {
                event_type,
                tool_name,
                args: serde_json::json!({}),
                output: None,
                exit_code: None,
                error: None,
                context: None,
                config,
            },
        }
    }

    pub fn args(mut self, args: serde_json::Value) -> Self {
        self.params.args = args;
        self
    }

    pub fn output(mut self, output: &'a str) -> Self {
        self.params.output = Some(output);
        self
    }

    pub fn exit_code(mut self, code: i32) -> Self {
        self.params.exit_code = Some(code);
        self
    }

    pub fn error(mut self, error: &'a str) -> Self {
        self.params.error = Some(error);
        self
    }

    pub fn context(mut self, context: &'a serde_json::Value) -> Self {
        self.params.context = Some(context);
        self
    }

    pub fn log(self) {
        log_audit(self.params);
    }

    pub fn log_and_return(self, log_path: Option<&std::path::Path>) -> Option<AuditEntry> {
        log_audit_and_return(self.params, log_path)
    }
}

impl<'a> AuditParams<'a> {
    pub fn builder(
        event_type: &'a str,
        tool_name: &'a str,
        config: &'a crate::config::models::AppConfig,
    ) -> AuditParamsBuilder<'a> {
        AuditParamsBuilder::new(event_type, tool_name, config)
    }
}

pub fn log_audit(params: AuditParams) {
    let _ = log_audit_and_return(params, None);
}

pub fn log_audit_and_return(params: AuditParams, log_path: Option<&Path>) -> Option<AuditEntry> {
    let default_path = &*AUDIT_LOG_PATH;
    let path = log_path.unwrap_or(default_path);
    let max_lines = params.config.general.max_audit_log_lines;

    if let Some(parent) = path.parent()
        && !parent.exists()
    {
        let _ = fs::create_dir_all(parent);
    }

    let timestamp = Utc::now().to_rfc3339();
    let empty_map = serde_json::Map::new();
    let ctx = params
        .context
        .and_then(|c| c.as_object())
        .unwrap_or(&empty_map);

    let trace_id = ctx
        .get("trace_id")
        .and_then(|v| v.as_str())
        .unwrap_or("-")
        .to_string();
    let subject = ctx
        .get("user_id")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();
    let audience = ctx
        .get("audience")
        .and_then(|v| v.as_str())
        .unwrap_or("-")
        .to_string();
    let model = ctx
        .get("model")
        .and_then(|v| v.as_str())
        .unwrap_or("-")
        .to_string();
    let provider = ctx
        .get("provider")
        .and_then(|v| v.as_str())
        .unwrap_or("-")
        .to_string();

    let hostname = hostname::get()
        .ok()
        .and_then(|h| h.into_string().ok())
        .unwrap_or_else(|| "unknown".to_string());
    let os = std::env::consts::OS.to_string();
    let arch = std::env::consts::ARCH.to_string();
    let cli_version = env!("CARGO_PKG_VERSION").to_string();

    let prev_hash = get_last_log_hash(path);

    // Hybrid Encryption for high-risk data
    let mut pqc_encrypted = false;
    let mut final_args = params.args.clone();

    if params.config.security.security_level == "high"
        && let Ok(pk) = crate::security::identity::IdentityManager::get_kem_public_key()
    {
        let arg_bytes = serde_json::to_vec(&params.args).unwrap_or_default();
        match crate::security::pqc::SecureStorage::encrypt(&arg_bytes, &pk) {
            Ok(packet) => {
                final_args = serde_json::to_value(packet).unwrap_or_else(|_| params.args.clone());
                pqc_encrypted = true;
            }
            Err(_e) => {}
        }
    }

    let mut log_entry = AuditEntry {
        timestamp,
        trace_id,
        subject,
        audience,
        model,
        provider,
        event_type: params.event_type.to_string(),
        tool: params.tool_name.to_string(),
        args: final_args,
        pqc_confidential: pqc_encrypted,
        output: params.output.map(|s| s.to_string()),
        status: match params.error {
            None => "SUCCESS".to_string(),
            Some(e) => format!("FAILED: {}", e),
        },
        exit_code: params.exit_code,
        prev_hash,
        hash: String::new(),
        pqc_signature: None,
        pqc_algorithm: None,
        hostname,
        os,
        arch,
        cli_version,
    };

    // Calculate hash over COMPLETE data before truncation for integrity
    let entry_json = serde_json::to_string(&log_entry).unwrap_or_default();
    let mut hasher = Sha256::new();
    hasher.update(entry_json.as_bytes());
    log_entry.hash = crate::utils::hex_encode(hasher.finalize());

    // PQC Signing
    let variant = crate::security::pqc::PQCAgilityManager::get_required_level(
        params.config,
        params.tool_name,
        Some(&params.args),
        if params.config.security.security_level == "high" {
            "high"
        } else {
            "standard"
        },
    );
    if let Ok(sk) = crate::security::identity::IdentityManager::get_pqc_private_key(variant) {
        match ResponseSigner::sign_response(&log_entry.hash, &log_entry.trace_id, &sk, variant) {
            Ok(signed) => {
                log_entry.pqc_signature = Some(signed.pqc_signature);
                log_entry.pqc_algorithm = Some(signed.algorithm);
            }
            Err(_e) => {}
        }
    }

    // Now truncate the output only for storage efficiency
    if let Some(ref mut out) = log_entry.output
        && out.len() > 1024
    {
        let mut end = 1024;
        while !out.is_char_boundary(end) {
            end -= 1;
        }
        out.truncate(end);
        out.push_str("...[TRUNCATED]");
    }

    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path)
        && let Ok(line) = serde_json::to_string(&log_entry)
    {
        let _ = writeln!(file, "{}", line);
    }

    if let Ok(metadata) = fs::metadata(path) {
        let estimated_line_count = metadata.len() / 2000;
        if estimated_line_count > (max_lines as u64 * 11 / 10) {
            trim_log_file(path, max_lines);
        }
    }

    Some(log_entry)
}

fn get_last_log_hash(path: &Path) -> String {
    if !path.exists() {
        return "0".repeat(64);
    }

    if let Ok(mut file) = fs::File::open(path) {
        let size = file.metadata().map(|m| m.len()).unwrap_or(0);
        if size == 0 {
            return "0".repeat(64);
        }

        // Increase buffer size to 128KB as requested in 3.7
        let read_size = std::cmp::min(size, 131072);
        let mut buffer = vec![0; read_size as usize];
        let _ = file.seek(SeekFrom::End(-(read_size as i64)));
        let _ = file.read_exact(&mut buffer);

        let content = String::from_utf8_lossy(&buffer);
        let lines: Vec<&str> = content.split('\n').collect();

        for line in lines.iter().rev() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            if let Ok(entry) = serde_json::from_str::<serde_json::Value>(trimmed)
                && let Some(hash) = entry.get("hash").and_then(|v| v.as_str())
            {
                return hash.to_string();
            }
        }
    }

    "0".repeat(64)
}

fn trim_log_file(path: &std::path::Path, max_lines: usize) {
    if !path.exists() {
        return;
    }

    let file = match fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return,
    };
    let reader = std::io::BufReader::new(file);
    use std::io::BufRead;

    // 1. First pass: count lines
    let total_lines = reader.lines().count();
    if total_lines <= max_lines {
        return;
    }

    // 2. Second pass: keep only the last max_lines
    let file = match fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return,
    };
    let mut reader = std::io::BufReader::new(file);
    let skip_count = total_lines - max_lines;

    let mut last_removed_hash = "0".repeat(64);
    for (i, line) in (&mut reader).lines().enumerate() {
        if i == skip_count - 1 {
            if let Ok(l) = line
                && let Ok(v) = serde_json::from_str::<serde_json::Value>(&l)
            {
                last_removed_hash = v
                    .get("hash")
                    .and_then(|h| h.as_str())
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| "0".repeat(64));
            }
            break;
        }
    }

    let hostname = hostname::get()
        .ok()
        .and_then(|h| h.into_string().ok())
        .unwrap_or_else(|| "unknown".to_string());
    let os = std::env::consts::OS.to_string();
    let arch = std::env::consts::ARCH.to_string();
    let cli_version = env!("CARGO_PKG_VERSION").to_string();

    // Create temp file for the trimmed content
    let temp_path = path.with_extension("tmp");
    if let Ok(mut temp_file) = fs::File::create(&temp_path) {
        let continuity_marker = serde_json::json!({
            "timestamp": Utc::now().to_rfc3339(),
            "trace_id": "rotation",
            "subject": "system",
            "audience": "audit",
            "model": "-",
            "provider": "-",
            "event_type": "LOG_ROTATION_MARKER",
            "tool": "system",
            "args": {},
            "pqc_confidential": false,
            "prev_hash": last_removed_hash,
            "hash": format!("ROTATION-NONCE-{}", Uuid::new_v4()),
            "status": "CONTINUITY_MAINTAINED",
            "hostname": hostname,
            "os": os,
            "arch": arch,
            "cli_version": cli_version
        });
        let _ = writeln!(temp_file, "{}", continuity_marker);

        for l in reader.lines().map_while(Result::ok) {
            let _ = writeln!(temp_file, "{}", l);
        }

        // Finalize by renaming
        drop(temp_file);
        let _ = fs::rename(&temp_path, path);
    }
}
