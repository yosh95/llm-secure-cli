use crate::core::context::AppContext;
use crate::core::session::ActiveSession;
use anyhow::bail;
use std::sync::Arc;

/// Grouped arguments for [`start_chat_session`] to keep the argument count
/// under clippy's `too_many_arguments` limit.
pub struct ChatArgs {
    pub provider_arg: Option<String>,
    pub model_arg: Option<String>,
    pub session_arg: Option<String>,
    pub sources: Vec<String>,
    pub stdout: bool,
    pub raw: bool,
    pub is_atty: bool,
}

/// Start a chat session (interactive or one-shot).
///
/// This is extracted from `main.rs` so that the entry point stays focused on
/// CLI routing while the chat-session logic lives alongside the other command
/// implementations.
///
/// # Errors
///
/// Returns an error instead of calling `process::exit` so that the caller
/// (typically `main`) can decide how to handle the failure and ensure that
/// any `Drop` destructors (e.g. `ActiveSession::finalize_audit`) run to
/// completion.
pub async fn start_chat_session(args: ChatArgs, ctx: Arc<AppContext>) -> anyhow::Result<()> {
    let ChatArgs {
        provider_arg,
        model_arg,
        session_arg,
        sources,
        stdout,
        raw,
        is_atty,
    } = args;
    let cm = &ctx.config_manager;
    let _config = match cm.get_config() {
        Ok(c) => c,
        Err(e) => {
            let msg = format!("Failed to load config: {}", e);
            ctx.ui.report_error(&msg);
            bail!(msg);
        }
    };
    let state = match cm.get_state() {
        Ok(s) => s,
        Err(e) => {
            ctx.ui
                .report_warning(&format!("Failed to load app state: {}. Using defaults.", e));
            Default::default()
        }
    };

    let active_providers = cm.get_active_providers();
    let is_first_launch = provider_arg.is_none() && state.last_used_provider.is_none();

    let mut provider = provider_arg
        .or(state.last_used_provider)
        .unwrap_or_else(|| "ollama".to_string());

    if !active_providers.contains(&provider) {
        if active_providers.contains(&"ollama".to_string()) {
            provider = "ollama".to_string();
        } else if !active_providers.is_empty() {
            provider = active_providers[0].clone();
        } else {
            let msg = "No active LLM providers found.";
            ctx.ui.report_error(msg);
            bail!(msg);
        }
    }

    let model = model_arg.or(state.last_used_model).unwrap_or_default();

    let stdout = stdout || !is_atty;

    // Spawn a background task to refresh the models cache if it doesn't exist
    // or is older than 24 hours
    {
        let ctx_bg = ctx.clone();
        tokio::spawn(async move {
            let c_path = crate::consts::models_cache_path();
            let should_refresh = if !c_path.exists() {
                true
            } else {
                match std::fs::metadata(&c_path) {
                    Ok(meta) => match meta.modified() {
                        Ok(mtime) => {
                            let age = std::time::SystemTime::now()
                                .duration_since(mtime)
                                .unwrap_or(std::time::Duration::ZERO);
                            age.as_secs() > 24 * 3600
                        }
                        Err(_) => true,
                    },
                    Err(_) => true,
                }
            };
            if should_refresh {
                tracing::info!("Background refresh of models cache...");
                ctx_bg.config_manager.update_models_cache().await;
            }
        });
    }

    let client = {
        let registry = ctx.client_registry.lock().await;
        registry.create_client(&provider, &model, stdout, raw, &ctx.config_manager)
    };

    if model.is_empty() {
        ctx.ui.report_warning(
            "No model configured. Use /m <model> to set a model before sending requests.",
        );
    } else if is_first_launch {
        ctx.ui.report_warning(
            "No provider/model configured. Use /m <model> or /p <provider> to configure.",
        );
    }

    if let Some(mut client) = client {
        if let Some(session_path) = session_arg {
            // Try session store first (by trace_id/filename), then as file path
            let conv = crate::utils::session_store::load_session(&session_path).or_else(|_| {
                crate::utils::session_store::load_from_path(&std::path::PathBuf::from(
                    &session_path,
                ))
            });
            match conv {
                Ok(conversation) => {
                    client.get_state_mut().conversation = conversation;
                }
                Err(e) => {
                    ctx.ui
                        .report_error(&format!("Failed to load session: {}", e));
                }
            }
        }

        let pdf_as_base64 = client.should_send_pdf_as_base64();
        let mut session = match ActiveSession::new(client, ctx.clone()) {
            Ok(s) => s,
            Err(e) => {
                let msg = format!("Failed to initialize session: {}", e);
                ctx.ui.report_error(&msg);
                bail!(msg);
            }
        };

        let mut all_sources = sources;
        if !is_atty {
            use std::io::Read;
            let mut buffer = String::new();
            if std::io::stdin().read_to_string(&mut buffer).is_ok() {
                let trimmed = buffer.trim();
                if !trimmed.is_empty() {
                    all_sources.insert(0, trimmed.to_string());
                }
            }
        }

        let sources = if all_sources.is_empty() {
            None
        } else {
            Some(crate::utils::media::process_sources(all_sources, pdf_as_base64).await)
        };
        session.run(sources, None).await;
        Ok(())
    } else {
        let msg = format!("Provider '{}' not found or not configured.", provider);
        ctx.ui.report_error(&msg);
        bail!(msg);
    }
}
