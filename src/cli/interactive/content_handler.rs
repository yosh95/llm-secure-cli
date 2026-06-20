use crate::cli::ui;
use crate::core::session::ActiveSession;
use crate::llm::models::{DataSource, Message, MessagePart, Role};
use console::Term;
use std::collections::HashMap;

pub fn handle_attach(session: &mut ActiveSession, source: &str) {
    if source.is_empty() {
        ui::report_error("Usage: /attach <path_or_url>");
        return;
    }

    let pdf_as_base64 = session.get_client().should_send_pdf_as_base64();
    let data = crate::utils::media::process_single_source(source, pdf_as_base64);
    if let Some(d) = data {
        ui::report_success(&format!("Attached {}: {}", d.content_type, source));
        session.pending_data.push(d);
        ui::report_info(
            "File queued. Type your question about it before sending (e.g. \"Summarize this PDF\").",
        );
    } else {
        ui::report_error(&format!("Failed to attach: {source}"));
    }
}

pub fn handle_summarize(session: &mut ActiveSession) {
    let history_len = session.get_client().get_state().conversation.len();
    if history_len == 0 {
        ui::report_warning("Conversation is empty, nothing to summarize.");
        return;
    }

    ui::report_info("Summarizing conversation and clearing history...");

    let summary_prompt = "Please provide a concise summary of the conversation so far. This summary will be used as context for future interactions. IMPORTANT: The summary must be written in the same language as the conversation (e.g., if the user is speaking Japanese, summarize in Japanese).";

    // Prepare data source for summarization
    let data = vec![DataSource {
        content: serde_json::Value::String(summary_prompt.to_string()),
        content_type: "text/plain".to_string(),
        is_file_or_url: false,
        metadata: HashMap::new(),
    }];

    // We use the empty tool_schemas as we just want a summary
    match session.get_client_mut().send(data, Vec::new()) {
        Ok(response) => {
            let summary_text = response.content.clone().unwrap_or_default();

            // Reconstruct history with summary
            let mut new_conversation = Vec::new();

            // Add the summary as a historical context rather than a system message
            // to avoid clashing with the dynamic system prompt (which includes the date).
            new_conversation.push(Message {
                role: Role::User,
                parts: vec![MessagePart::Text(format!(
                    "Summary of our previous conversation for context:\n{summary_text}"
                ))],
            });

            new_conversation.push(Message {
                role: Role::Assistant,
                parts: vec![MessagePart::Text(
                    "I have acknowledged the summary and will use it as context for our continued conversation."
                        .to_string(),
                )],
            });

            session.get_client_mut().get_state_mut().conversation = new_conversation;

            ui::report_success("Conversation summarized and history cleared.");
            let (_, width) = Term::stdout().size();
            let sep = "─".repeat(width as usize);
            println!("\n{}\n", sep);
            println!("{summary_text}");
            println!("{}\n", sep);
        }
        Err(e) => ui::report_error(&format!("Failed to summarize: {e}")),
    }
}

pub fn handle_view_cmd(session: &mut ActiveSession, args: &str) {
    let config = match session.ctx.config_manager.get_config() {
        Ok(c) => c,
        Err(e) => {
            ui::report_error(&format!("Failed to load config: {e}"));
            return;
        }
    };

    let save_dir = std::path::Path::new(&config.general.image_save_path);

    if args.is_empty() {
        // No argument: find the most recently saved media file
        match crate::utils::media::find_latest_media(save_dir) {
            Some(latest) => match crate::utils::media::open_file_with_default_app(&latest) {
                Ok(()) => ui::report_success(&format!("Opened: {}", latest.display())),
                Err(e) => ui::report_error(&e.to_string()),
            },
            None => {
                ui::report_error(&format!(
                    "No saved media found in {}. Generate an image first.",
                    save_dir.display()
                ));
            }
        }
    } else {
        // Argument: treat as a file path
        let path = std::path::Path::new(args);
        let path = if path.is_relative() {
            // Try relative to CWD, then relative to the save directory
            let cwd_path = std::env::current_dir().unwrap_or_default().join(args);
            if cwd_path.exists() {
                cwd_path
            } else {
                save_dir.join(args)
            }
        } else {
            path.to_path_buf()
        };

        // Expand ~ if present
        let path = if path.starts_with("~") {
            if let Some(home) = dirs::home_dir() {
                if let Ok(stripped) = path.strip_prefix("~") {
                    home.join(stripped)
                } else {
                    path
                }
            } else {
                path
            }
        } else {
            path
        };

        if !path.exists() {
            ui::report_error(&format!("File not found: {}", path.display()));
            return;
        }

        match crate::utils::media::open_file_with_default_app(&path) {
            Ok(()) => ui::report_success(&format!("Opened: {}", path.display())),
            Err(e) => ui::report_error(&e.to_string()),
        }
    }
}
