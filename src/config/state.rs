//! Application state persistence (provider/model memory, aliases).

use crate::config::models::AppState;
use crate::consts::state_file_path;
use std::fs;

use super::ConfigManager;

impl ConfigManager {
    /// Returns a clone of the current application state.
    ///
    /// On the very first call the state is loaded from disk; subsequent calls
    /// return the in-memory copy.  Mutations (via *update_state*, etc.) are
    /// always written through to disk.
    pub fn get_state(&self) -> anyhow::Result<AppState> {
        let read = self
            .app_state
            .read()
            .map_err(|e| anyhow::anyhow!("Lock poisoned: {}", e))?;

        // If the state is still the default placeholder and a state file exists
        // on disk, we need to populate it.  We drop the read lock first to avoid
        // deadlocking when acquiring the write lock.
        if read.last_used_provider.is_none()
            && read.last_used_model.is_none()
            && read.last_used_v_provider.is_none()
            && read.last_used_v_model.is_none()
            && read.model_aliases.is_empty()
        {
            drop(read);
            let mut write = self
                .app_state
                .write()
                .map_err(|e| anyhow::anyhow!("Lock poisoned: {}", e))?;
            // Double-check after acquiring write lock (another thread may have
            // initialized already).
            if write.last_used_provider.is_none()
                && write.last_used_model.is_none()
                && write.model_aliases.is_empty()
            {
                *write = Self::load_state_from_disk();
            }
            return Ok(write.clone());
        }

        Ok(read.clone())
    }

    /// Load state from disk (static helper used during first access).
    fn load_state_from_disk() -> AppState {
        let s_path = state_file_path();
        if s_path.exists() {
            let content = match fs::read_to_string(&s_path) {
                Ok(c) => c,
                Err(e) => {
                    tracing::warn!(
                        path = %s_path.display(),
                        error = %e,
                        "Failed to read state file; falling back to defaults"
                    );
                    String::new()
                }
            };
            match toml::from_str(&content) {
                Ok(s) => s,
                Err(e) => {
                    tracing::warn!(
                        path = %s_path.display(),
                        error = %e,
                        "Failed to parse state file; falling back to defaults"
                    );
                    AppState::default()
                }
            }
        } else {
            AppState::default()
        }
    }

    /// Helper: persist an updated state to disk, logging any write failure.
    ///
    /// IMPORTANT: This must be called **while holding the write lock** so that
    /// the in-memory state and on-disk state stay consistent.
    fn persist_state(state: &AppState) {
        if let Ok(content) = toml::to_string(state)
            && let Err(e) = fs::write(state_file_path(), content)
        {
            tracing::error!(
                path = %state_file_path().display(),
                error = %e,
                "CRITICAL: Failed to write state file — state may be lost on restart"
            );
        }
    }

    pub fn update_state(&self, provider: &str, model: &str) -> anyhow::Result<()> {
        let mut write = self
            .app_state
            .write()
            .map_err(|e| anyhow::anyhow!("Lock poisoned: {}", e))?;
        write.last_used_provider = Some(provider.to_string());
        write.last_used_model = Some(model.to_string());
        Self::persist_state(&write);
        Ok(())
    }

    pub fn update_v_state(&self, provider: &str, model: &str) -> anyhow::Result<()> {
        let mut write = self
            .app_state
            .write()
            .map_err(|e| anyhow::anyhow!("Lock poisoned: {}", e))?;
        write.last_used_v_provider = Some(provider.to_string());
        write.last_used_v_model = Some(model.to_string());
        Self::persist_state(&write);
        Ok(())
    }

    pub fn set_alias(&self, alias: &str, target: &str) -> anyhow::Result<()> {
        let mut write = self
            .app_state
            .write()
            .map_err(|e| anyhow::anyhow!("Lock poisoned: {}", e))?;
        write.model_aliases.insert(
            alias.to_string(),
            crate::config::models::ModelAlias {
                target: target.to_string(),
            },
        );
        Self::persist_state(&write);
        Ok(())
    }

    pub fn remove_alias(&self, alias: &str) -> anyhow::Result<bool> {
        let mut write = self
            .app_state
            .write()
            .map_err(|e| anyhow::anyhow!("Lock poisoned: {}", e))?;
        let existed = write.model_aliases.remove(alias).is_some();
        if existed {
            Self::persist_state(&write);
        }
        Ok(existed)
    }

    /// Resolve the verifier provider and model, prioritizing AppState (state.toml)
    /// but falling back to AppConfig (config.toml).
    pub fn get_verifier_settings(&self) -> (String, String) {
        let state = self.get_state().unwrap_or_else(|_| Default::default());
        let config = self.get_config().ok();

        let provider = state
            .last_used_v_provider
            .filter(|s| !s.is_empty())
            .or_else(|| {
                config
                    .as_ref()
                    .map(|c| c.security.verifier_provider.clone())
            })
            .unwrap_or_default();

        let model = state
            .last_used_v_model
            .filter(|s| !s.is_empty())
            .or_else(|| config.as_ref().map(|c| c.security.verifier_model.clone()))
            .unwrap_or_default();

        (provider, model)
    }

    /// Resolve the full verifier committee configuration.
    ///
    /// Returns a tuple of:
    /// - The legacy primary (provider, model) — may be empty if only committee is used.
    /// - A list of additional committee members from `verifier_committee.members`.
    ///
    /// The primary (if configured) is used as the first committee member.
    /// All members are verified concurrently with an "any-flag" policy.
    pub fn get_verifier_committee(&self) -> (Vec<(String, String)>, bool) {
        let state = self.get_state().unwrap_or_else(|_| Default::default());
        let config = self.get_config().ok();
        let mut members: Vec<(String, String)> = Vec::new();

        // 1. Legacy primary verifier (from state or config)
        let primary_provider = state
            .last_used_v_provider
            .filter(|s| !s.is_empty())
            .or_else(|| {
                config
                    .as_ref()
                    .map(|c| c.security.verifier_provider.clone())
            })
            .unwrap_or_default();

        let primary_model = state
            .last_used_v_model
            .filter(|s| !s.is_empty())
            .or_else(|| config.as_ref().map(|c| c.security.verifier_model.clone()))
            .unwrap_or_default();

        if !primary_provider.is_empty() && !primary_model.is_empty() {
            members.push((primary_provider, primary_model));
        }

        // 2. Additional committee members from verifier_committee config
        if let Some(ref cfg) = config {
            for member in &cfg.security.verifier_committee.members {
                if !member.provider.is_empty() && !member.model.is_empty() {
                    // Avoid duplicate if the primary happens to be the same
                    let pair = (member.provider.clone(), member.model.clone());
                    if !members.contains(&pair) {
                        members.push(pair);
                    }
                }
            }
        }

        let enabled = config
            .as_ref()
            .and_then(|c| c.security.verifier_enabled)
            .unwrap_or(false)
            && !members.is_empty();

        (members, enabled)
    }
}
