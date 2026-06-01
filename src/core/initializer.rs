use crate::cli::ui::{self, UserInterface};
use crate::core::context::AppContext;
use crate::core::session::ActiveSession;
use crate::llm::base::LlmClient;
use std::io::{IsTerminal, stdin};
use std::sync::Arc;

pub async fn switch_model(
    session: &mut ActiveSession,
    model: &str,
    provider: &str,
    stdout: bool,
    render_markdown: bool,
) -> anyhow::Result<()> {
    // 1. Resolve alias if it exists (backward compat - caller may pass alias)
    let (target_model, target_provider) = {
        let state = session.ctx.config_manager.get_state()?;
        if let Some(alias) = state.model_aliases.get(model) {
            // Alias target is usually "model" or "provider:model"
            if let Some((p, m)) = alias.target.split_once(':') {
                (m.to_string(), p.to_string())
            } else {
                (
                    alias.target.clone(),
                    session.client.get_state().provider.clone(),
                )
            }
        } else {
            (model.to_string(), provider.to_string())
        }
    };

    let client = {
        let registry = session.ctx.client_registry.lock().await;
        registry.create_client(
            &target_provider,
            &target_model,
            stdout,
            !render_markdown,
            &session.ctx.config_manager,
        )
    };

    if let Some(new_client) = client {
        session.switch_client(new_client);
        let full_model = format!("{}:{}", target_provider, target_model);
        if let Err(e) = session.ctx.config_manager.update_state(&full_model) {
            tracing::warn!("Failed to persist state update: {}", e);
        }
        Ok(())
    } else {
        anyhow::bail!("Failed to create client for model: {target_model}")
    }
}

pub async fn switch_provider(session: &mut ActiveSession, provider: &str) -> anyhow::Result<()> {
    let (stdout, render_markdown) = {
        let state = session.client.get_state();
        (state.stdout, state.render_markdown)
    };

    let client = {
        let registry = session.ctx.client_registry.lock().await;
        registry.create_client(
            provider,
            "default",
            stdout,
            !render_markdown,
            &session.ctx.config_manager,
        )
    };

    if let Some(new_client) = client {
        session.switch_client(new_client);
        let full_model = format!("{}:{}", provider, "default");
        if let Err(e) = session.ctx.config_manager.update_state(&full_model) {
            tracing::warn!("Failed to persist state update: {}", e);
        }
        Ok(())
    } else {
        anyhow::bail!("Failed to create client for provider: {provider}")
    }
}

pub async fn initialize_app(ui: Arc<dyn UserInterface>) -> anyhow::Result<Arc<AppContext>> {
    let ctx = Arc::new(AppContext::new(ui));
    let is_atty = stdin().is_terminal();

    // 1. Setup permissions and directories
    crate::security::permissions::setup_permissions();

    // 2. Ensure configuration exists
    if !crate::consts::config_file_path().exists() {
        crate::config::init::init_config();
    }

    // 3. Security & Integrity Checks
    ensure_identity_and_integrity(&ctx, is_atty).await?;

    // 4. Register Tools
    {
        let mut registry = ctx.tool_registry.write().await;
        crate::tools::registry::register_builtin_tools(&mut registry, &ctx.config_manager);
    }

    // 5. Initialize Remote Tools (MCP)
    if let Err(e) = crate::tools::registry::initialize_remote_tools(
        ctx.tool_registry.clone(),
        &ctx.config_manager,
        &ctx.mcp_manager,
    )
    .await
    {
        tracing::warn!("Failed to initialize remote tools: {}", e);
    }

    // 6. Register LLM Clients
    register_clients(&ctx).await;

    Ok(ctx)
}

async fn ensure_identity_and_integrity(ctx: &Arc<AppContext>, is_atty: bool) -> anyhow::Result<()> {
    use crate::security::identity::IdentityManager;
    use crate::security::integrity::IntegrityVerifier;

    // 1. Ensure Identity Keys
    if !IdentityManager::has_keys()
        && is_atty
        && ctx
            .ui
            .ask_confirm_simple("Identity keys not found. Generate new PQC keypair for this agent?")
            .await
            == Some(ui::ConfirmResult::Yes)
    {
        if let Err(e) = IdentityManager::ensure_keys() {
            ctx.ui
                .report_error(&format!("Failed to generate keys: {e}"));
        } else {
            ctx.ui.report_success("Identity keys generated.");
        }
    }

    // 2. System Integrity Check
    let verifier = IntegrityVerifier::new();
    let _config = ctx.config_manager.get_config()?;

    if verifier.manifest_path.exists() {
        match verifier.verify() {
            Ok(true) => {
                // Integrity OK
            }
            Ok(false) => {
                ctx.ui.report_warning("CRITICAL: SYSTEM INTEGRITY MISMATCH");
                ctx.ui.report_warning(
                    "The binary or configuration has changed since the last manifest update.",
                );
                ctx.ui.report_warning(
                    "(This occurs after 'cargo install' or manual configuration edits)",
                );

                if is_atty
                    && ctx
                        .ui
                        .ask_confirm_simple(
                            "Would you like to re-authorize (re-sign) the current system state?",
                        )
                        .await
                        == Some(ui::ConfirmResult::Yes)
                {
                    if let Err(e) = verifier.rebuild_manifest() {
                        ctx.ui
                            .report_error(&format!("Failed to rebuild manifest: {e}"));
                        return Err(anyhow::anyhow!("Integrity manifest rebuild failed: {e}"));
                    } else {
                        ctx.ui.report_success("Integrity manifest updated.");
                    }
                } else {
                    ctx.ui
                        .report_error("Execution aborted due to integrity failure.");
                    return Err(anyhow::anyhow!(
                        "Execution aborted: integrity verification failed."
                    ));
                }
            }
            Err(_e) => {}
        }
    } else {
        let msg = "SECURITY FAILURE: Integrity manifest not found. A signed manifest is required.";

        ctx.ui.report_warning(msg);
        if is_atty
            && ctx
                .ui
                .ask_confirm_simple("Generate and sign integrity manifest now?")
                .await
                == Some(ui::ConfirmResult::Yes)
        {
            if let Err(e) = verifier.rebuild_manifest() {
                ctx.ui
                    .report_error(&format!("Failed to build manifest: {e}"));
                return Err(anyhow::anyhow!("Integrity manifest build failed: {e}"));
            } else {
                ctx.ui.report_success("Integrity manifest generated.");
            }
        } else {
            return Err(anyhow::anyhow!(
                "Execution aborted: integrity manifest not found."
            ));
        }
    }

    Ok(())
}

async fn register_clients(ctx: &Arc<AppContext>) {
    use crate::llm::providers::deepinfra::DeepInfraClient;
    use crate::llm::providers::ollama::OllamaClient;
    use crate::llm::providers::openai_compatible::OpenAiCompatibleClient;
    use crate::llm::providers::openrouter::OpenRouterClient;

    let mut registry = ctx.client_registry.lock().await;
    let active_providers = ctx.config_manager.get_active_providers();

    for provider in active_providers {
        let p_name = provider.clone();
        let closure_p_name = p_name.clone();
        registry.register(
            &p_name,
            Arc::new(move |model, stdout, raw, config_manager| {
                let api_key = config_manager
                    .get_api_key(&closure_p_name)
                    .unwrap_or_else(String::new);

                // Read api_url and the optional formatter hint from config in one pass.
                let (api_url, formatter_hint) = if let Ok(config) = config_manager.get_config() {
                    let p_cfg = config.providers.get(&closure_p_name);
                    let url = p_cfg
                        .and_then(|p| p.api_url.clone())
                        .unwrap_or_else(|| match closure_p_name.as_str() {
                            "openai" => "https://api.openai.com/v1".to_string(),
                            "ollama" => "http://localhost:11434/v1".to_string(),
                            "openrouter" => "https://openrouter.ai/api/v1".to_string(),
                            _ => String::new(),
                        });
                    let hint = p_cfg.and_then(|p| p.formatter.clone());
                    (url, hint)
                } else {
                    (String::new(), None)
                };

                let client: Box<dyn LlmClient> = match closure_p_name.as_str() {
                    "openrouter" => Box::new(OpenRouterClient::new(
                        config_manager,
                        &closure_p_name,
                        &api_url,
                        &api_key,
                        model,
                        stdout,
                        raw,
                    )?),
                    "ollama" => Box::new(OllamaClient::new(
                        config_manager,
                        &closure_p_name,
                        &api_url,
                        &api_key,
                        model,
                        stdout,
                        raw,
                    )?),
                    "deepinfra" => Box::new(DeepInfraClient::new(
                        config_manager,
                        &closure_p_name,
                        &api_url,
                        &api_key,
                        model,
                        stdout,
                        raw,
                    )?),
                    _ => {
                        // Formatter selection priority:
                        //   1. Explicit `formatter` field in config.toml  ← new, preferred
                        //   2. Legacy model-name heuristic                ← backwards compat
                        let use_high_feature = match formatter_hint.as_deref() {
                            Some("high_feature") => true,
                            Some("generic") => false,
                            // Fallback: infer from well-known model name fragments.
                            _ => {
                                let m_lower = model.to_lowercase();
                                m_lower.contains("claude")
                                    || m_lower.contains("anthropic")
                                    || m_lower.contains("gemini")
                                    || m_lower.contains("google")
                            }
                        };
                        let formatter: Box<dyn crate::llm::providers::openai_compatible::PayloadFormatter> = if use_high_feature {
                            Box::new(crate::llm::providers::openai_compatible::HighFeaturePayloadFormatter { is_anthropic_gemini: true })
                        } else {
                            Box::new(crate::llm::providers::openai_compatible::GenericPayloadFormatter)
                        };

                        // Look up whether this model supports tool calling from the cache.
                        // When the cache has not been populated yet (first launch) we default
                        // to `true` so that tool definitions are still sent.
                        let model_supports_tools = config_manager
                            .model_supports_tools(&closure_p_name, model)
                            .unwrap_or(true);

                        Box::new(OpenAiCompatibleClient::builder(config_manager)
                            .provider_name(&closure_p_name)
                            .api_url(&api_url)
                            .api_key(&api_key)
                            .model(model)
                            .stdout(stdout)
                            .raw(raw)
                            .formatter(formatter)
                            .supports_tools(Some(model_supports_tools))
                            .build()?)
                    },
                };
                Ok(client)
            }),
        );
    }
}
