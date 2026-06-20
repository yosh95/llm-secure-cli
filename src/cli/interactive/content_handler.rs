use crate::cli::ui;
use crate::core::session::ActiveSession;
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
