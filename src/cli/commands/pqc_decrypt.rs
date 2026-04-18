use crate::cli::ui;
use crate::security::identity::IdentityManager;
use crate::security::pqc::{EncryptedPacket, SecureStorage};
use serde_json::Value;
use std::fs;
use std::path::PathBuf;

pub fn decrypt_log_file(input_path: PathBuf, output_path: Option<PathBuf>) {
    if !input_path.exists() {
        ui::report_error(&format!("File {:?} not found.", input_path));
        return;
    }

    let kem_sk = match IdentityManager::get_kem_private_key() {
        Ok(k) => k,
        Err(e) => {
            ui::report_error(&format!("Failed to load PQC KEM private key: {}", e));
            return;
        }
    };

    let content = match fs::read_to_string(&input_path) {
        Ok(c) => c,
        Err(e) => {
            ui::report_error(&format!("Failed to read file: {}", e));
            return;
        }
    };

    let mut decrypted_entries = Vec::new();
    for line in content.lines() {
        if let Ok(mut entry) = serde_json::from_str::<Value>(line) {
            if entry.get("pqc_confidential") == Some(&Value::Bool(true)) {
                if let Some(args_val) = entry.get("args") {
                    if let Ok(packet) = serde_json::from_value::<EncryptedPacket>(args_val.clone())
                    {
                        match std::panic::catch_unwind(|| SecureStorage::decrypt(&packet, &kem_sk))
                        {
                            Ok(decrypted_bytes) => {
                                if let Ok(decrypted_json) =
                                    serde_json::from_slice::<Value>(&decrypted_bytes)
                                {
                                    entry["args"] = decrypted_json;
                                    entry["pqc_confidential"] =
                                        Value::String("DECRYPTED".to_string());
                                }
                            }
                            Err(_) => {
                                ui::report_error("Failed to decrypt entry: decryption error.");
                            }
                        }
                    }
                }
            }
            decrypted_entries.push(entry);
        }
    }

    if let Some(out) = output_path {
        let mut out_str = String::new();
        for entry in &decrypted_entries {
            out_str.push_str(&serde_json::to_string(entry).unwrap());
            out_str.push('\n');
        }
        if let Err(e) = fs::write(&out, out_str) {
            ui::report_error(&format!("Failed to write output: {}", e));
        } else {
            ui::report_success(&format!("Decrypted log saved to {:?}", out));
        }
    } else {
        for entry in &decrypted_entries {
            println!("{}", serde_json::to_string_pretty(entry).unwrap());
        }
    }
}
