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

    // 0. Check API key environment variables
    check_api_key_env_vars()?;

    // 1. Setup permissions and directories
    crate::security::permissions::setup_permissions();

    // 2. Ensure configuration exists
    if !crate::consts::config_file_path().exists() {
        crate::config::init::init_config();
    }

    // 3. Identity Key Setup
    ensure_identity(&ctx, is_atty).await?;

    // 4. Register Tools
    {
        let mut registry = ctx.tool_registry.write().await;
        crate::tools::registry::register_builtin_tools(&mut registry, &ctx.config_manager);
    }
    // 4b. Warn about unavailable tools
    {
        let registry = ctx.tool_registry.read().await;
        if !registry.has_tool("brave_search") {
            ctx.ui.report_warning(
                "brave_search is not available. Set BRAVE_API_KEY in environment or .env file to enable web search."
            );
        }
        if !registry.has_tool("execute_python") {
            ctx.ui.report_warning(
                "execute_python is not available. Install python3 or python in PATH to enable code execution."
            );
        }
    }

    // 5. Register LLM Clients
    register_clients(&ctx).await;

    Ok(ctx)
}

async fn ensure_identity(ctx: &Arc<AppContext>, is_atty: bool) -> anyhow::Result<()> {
    use crate::security::identity::IdentityManager;

    // Ensure Identity Keys
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

    Ok(())
}

/// Check that at least one of the required API key environment variables is set.
///
/// Required variables: `OLLAMA_API_KEY`, `OPENROUTER_API_KEY`, `VLLM_API_KEY`.
/// Even for local Ollama (which technically does not need an API key), the user
/// must explicitly set one of these environment variables.
fn check_api_key_env_vars() -> anyhow::Result<()> {
    let required_vars = ["OLLAMA_API_KEY", "OPENROUTER_API_KEY", "VLLM_API_KEY"];
    let any_set = required_vars.iter().any(|var| std::env::var(var).is_ok());

    if !any_set {
        // Also check .env files as a fallback
        let env_files = [
            std::path::Path::new(".env").to_path_buf(),
            crate::consts::get_base_dir().join(".env"),
        ];
        let any_in_env_file = env_files.iter().any(|path| {
            if path.exists()
                && let Ok(content) = std::fs::read_to_string(path)
            {
                return content.lines().any(|line| {
                    let line = line.trim();
                    line.starts_with("OLLAMA_API_KEY=")
                        || line.starts_with("OPENROUTER_API_KEY=")
                        || line.starts_with("VLLM_API_KEY=")
                });
            }
            false
        });

        if !any_in_env_file {
            anyhow::bail!(
                "No API key environment variable set.
                 At least one of the following must be defined:
                 - OLLAMA_API_KEY (local Ollama)
                 - OPENROUTER_API_KEY (OpenRouter)
                 - VLLM_API_KEY (vLLM)

                 Even if you are using a local Ollama server that does not require
                 an API key, you must set a dummy API key (e.g. OLLAMA_API_KEY=dummy)
                 to proceed. This is required for security validation.

                 You can set it via:
                   export OLLAMA_API_KEY=dummy
                 Or create a .env file with:
                   OLLAMA_API_KEY=dummy"
            );
        }
    }

    Ok(())
}

async fn register_clients(ctx: &Arc<AppContext>) {
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
                            "vllm" => "http://localhost:8000/v1".to_string(),
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
