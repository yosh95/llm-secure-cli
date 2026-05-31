//! Application state persistence (provider/model memory, aliases, flags).

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

    /// Set the primary verifier committee member (replaces all members).
    /// The format is "provider:model" (e.g. "ollama:gemma4:e2b").
    /// This clears any existing committee members and sets this as the sole member.
    pub fn set_primary_verifier(&self, provider_model: &str) -> anyhow::Result<()> {
        let mut write = self
            .app_state
            .write()
            .map_err(|e| anyhow::anyhow!("Lock poisoned: {}", e))?;
        write.verifier_committee_members.clear();
        write
            .verifier_committee_members
            .push(provider_model.to_string());
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

    // ── Verifier enabled flag (state.toml) ─────────────────────────────────

    /// Get the verifier enabled state from state.toml.
    pub fn get_verifier_enabled(&self) -> bool {
        let state = self.get_state().unwrap_or_else(|_| Default::default());
        state.verifier_enabled.unwrap_or(true)
    }

    /// Toggle verifier on/off and persist to state.toml.
    pub fn set_verifier_enabled(&self, enabled: bool) -> anyhow::Result<()> {
        let mut write = self
            .app_state
            .write()
            .map_err(|e| anyhow::anyhow!("Lock poisoned: {}", e))?;
        write.verifier_enabled = Some(enabled);
        Self::persist_state(&write);
        Ok(())
    }

    // ── Show tool result flag (state.toml) ─────────────────────────────────

    /// Whether tool execution results should be displayed to the user.
    /// Default (None/false) = hidden — useful when output is very large.
    pub fn get_show_tool_result(&self) -> bool {
        let state = self.get_state().unwrap_or_else(|_| Default::default());
        state.show_tool_result.unwrap_or(false)
    }

    /// Set the show_tool_result flag and persist to state.toml.
    pub fn set_show_tool_result(&self, show: bool) -> anyhow::Result<()> {
        let mut write = self
            .app_state
            .write()
            .map_err(|e| anyhow::anyhow!("Lock poisoned: {}", e))?;
        write.show_tool_result = Some(show);
        Self::persist_state(&write);
        Ok(())
    }

    // ── Verifier committee (state.toml) ────────────────────────────────────

    /// Resolve the primary verifier provider:model from committee members.
    /// Returns (provider, model) parsed from "provider:model" format.
    /// Returns empty strings if no committee members configured.
    pub fn get_verifier_settings(&self) -> (String, String) {
        let state = self.get_state().unwrap_or_else(|_| Default::default());
        if let Some(first) = state.verifier_committee_members.first()
            && !first.is_empty()
            && let Some((provider, model)) = first.split_once(':')
        {
            return (provider.to_string(), model.to_string());
        }
        (String::new(), String::new())
    }

    /// Add a committee member (provider:model format) to the verifier committee.
    pub fn add_verifier_committee_member(&self, provider_model: &str) -> anyhow::Result<()> {
        let mut write = self
            .app_state
            .write()
            .map_err(|e| anyhow::anyhow!("Lock poisoned: {}", e))?;
        let pm = provider_model.to_string();
        if !write.verifier_committee_members.contains(&pm) {
            write.verifier_committee_members.push(pm);
        }
        Self::persist_state(&write);
        Ok(())
    }

    /// Remove a committee member (provider:model format) from the verifier committee.
    pub fn remove_verifier_committee_member(&self, provider_model: &str) -> anyhow::Result<bool> {
        let mut write = self
            .app_state
            .write()
            .map_err(|e| anyhow::anyhow!("Lock poisoned: {}", e))?;
        let len_before = write.verifier_committee_members.len();
        write
            .verifier_committee_members
            .retain(|m| m != provider_model);
        let removed = write.verifier_committee_members.len() < len_before;
        if removed {
            Self::persist_state(&write);
        }
        Ok(removed)
    }

    /// List all verifier committee members (provider:model format).
    pub fn list_verifier_committee(&self) -> Vec<String> {
        let state = self.get_state().unwrap_or_else(|_| Default::default());
        state.verifier_committee_members.clone()
    }

    /// Resolve the verifier committee configuration.
    ///
    /// Returns a tuple of:
    /// - All committee members as (provider, model) pairs.
    /// - Whether the verifier is enabled and has at least one member configured.
    ///
    /// Members are resolved from `verifier_committee_members` in state.toml.
    /// A single member means single-verifier mode; multiple means committee mode
    /// with an "any-flag" policy.
    pub fn get_verifier_committee(&self) -> (Vec<(String, String)>, bool) {
        let state = self.get_state().unwrap_or_else(|_| Default::default());
        let mut members: Vec<(String, String)> = Vec::new();

        for pm in &state.verifier_committee_members {
            if let Some((provider, model)) = pm.split_once(':')
                && !provider.is_empty()
                && !model.is_empty()
            {
                let pair = (provider.to_string(), model.to_string());
                if !members.contains(&pair) {
                    members.push(pair);
                }
            }
        }

        let enabled = state.verifier_enabled.unwrap_or(true) && !members.is_empty();

        (members, enabled)
    }
}
