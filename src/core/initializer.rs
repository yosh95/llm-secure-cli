use crate::cli::ui::{self, UserInterface};
use crate::core::context::AppContext;
use crate::core::session::ActiveSession;
use crate::llm::base::LlmClient;
use std::io::{IsTerminal, stdin};
use std::sync::Arc;

pub fn switch_model(
    session: &mut ActiveSession,
    model: &str,
    provider: &str,
    stdout: bool,
    render_markdown: bool,
) -> anyhow::Result<()> {
    let client = {
        let registry = session
            .ctx
            .client_registry
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        registry.create_client(
            provider,
            model,
            stdout,
            !render_markdown,
            &session.ctx.config_manager,
        )
    };

    if let Some(new_client) = client {
        session.switch_client(new_client);
        let full_model = format!("{provider}:{model}");
        if let Err(e) = session.ctx.config_manager.update_state(&full_model) {
            tracing::warn!("Failed to persist state update: {}", e);
        }
        Ok(())
    } else {
        anyhow::bail!("Failed to create client for model: {model}")
    }
}

pub fn switch_provider(session: &mut ActiveSession, provider: &str) -> anyhow::Result<()> {
    let (stdout, render_markdown) = {
        let state = session.client.get_state();
        (state.stdout, state.render_markdown)
    };

    let client = {
        let registry = session
            .ctx
            .client_registry
            .lock()
            .unwrap_or_else(|p| p.into_inner());
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

pub fn initialize_app(ui: Arc<dyn UserInterface>) -> anyhow::Result<Arc<AppContext>> {
    let ctx = Arc::new(AppContext::new(ui));
    let is_atty = stdin().is_terminal();

    // 0. Check API key environment variables
    check_api_key_env_vars()?;

    // 1. Setup permissions and directories
    crate::security::permissions::setup_permissions();

    // 2. Identity Key Setup
    ensure_identity(&ctx, is_atty)?;

    // 4. Register Tools
    {
        let mut registry = ctx.tool_registry.write().unwrap_or_else(|p| p.into_inner());
        crate::tools::registry::register_builtin_tools(&mut registry, &ctx.config_manager);
    }
    // 4b. Warn about unavailable tools
    {
        let registry = ctx.tool_registry.read().unwrap_or_else(|p| p.into_inner());
        if !registry.has_tool("execute_python") {
            ctx.ui.report_warning(
                "execute_python is not available. Install python3 or python in PATH to enable code execution."
            );
        }
    }

    // 5. Register LLM Clients
    register_clients(&ctx);

    Ok(ctx)
}

fn ensure_identity(ctx: &Arc<AppContext>, is_atty: bool) -> anyhow::Result<()> {
    use crate::security::identity::IdentityManager;

    // Ensure Identity Keys
    if !IdentityManager::has_keys()
        && is_atty
        && ctx
            .ui
            .ask_confirm_simple("Identity keys not found. Generate new PQC keypair for this agent?")
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

fn register_clients(ctx: &Arc<AppContext>) {
    use crate::llm::providers::openai_compatible::OpenAiCompatibleClient;

    let mut registry = ctx
        .client_registry
        .lock()
        .unwrap_or_else(|p| p.into_inner());
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

                // Read api_url from config.
                let api_url = if let Ok(config) = config_manager.get_config() {
                    let p_cfg = config.providers.get(&closure_p_name);
                    p_cfg.and_then(|p| p.api_url.clone()).unwrap_or_else(|| {
                        match closure_p_name.as_str() {
                            "openai" => "https://api.openai.com/v1".to_string(),
                            "ollama" => "http://localhost:11434/v1".to_string(),
                            "openrouter" => "https://openrouter.ai/api/v1".to_string(),
                            "vllm" => "http://localhost:8000/v1".to_string(),
                            _ => String::new(),
                        }
                    })
                } else {
                    String::new()
                };

                // All providers use the OpenAI-compatible API.
                // The api_url uniquely identifies the service.
                let model_supports_tools = config_manager
                    .model_supports_tools(&closure_p_name, model)
                    .unwrap_or(true);

                let client: Box<dyn LlmClient> = Box::new(
                    OpenAiCompatibleClient::builder(config_manager)
                        .provider_name(&closure_p_name)
                        .api_url(&api_url)
                        .api_key(&api_key)
                        .model(model)
                        .stdout(stdout)
                        .raw(raw)
                        .supports_tools(Some(model_supports_tools))
                        .build()?,
                );
                Ok(client)
            }),
        );
    }
}
