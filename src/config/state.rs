//! Application state persistence (provider/model memory, flags).

use crate::config::models::AppState;
use crate::consts::state_file_path;
use std::fs;

use super::ConfigManager;

impl ConfigManager {
    /// Returns a clone of the current application state.
    ///
    /// On the very first call the state is loaded from disk; subsequent calls
    /// return the in-memory copy.  Mutations (via *`update_state`*, etc.) are
    /// always written through to disk.
    pub fn get_state(&self) -> anyhow::Result<AppState> {
        let read = self
            .app_state
            .read()
            .map_err(|e| anyhow::anyhow!("Lock poisoned: {e}"))?;

        // If the state is still the default placeholder and a state file exists
        // on disk, we need to populate it.  We drop the read lock first to avoid
        // deadlocking when acquiring the write lock.
        if read.last_model.is_none() {
            drop(read);
            let mut write = self
                .app_state
                .write()
                .map_err(|e| anyhow::anyhow!("Lock poisoned: {e}"))?;
            // Double-check after acquiring write lock (another thread may have
            // initialized already).
            if write.last_model.is_none() {
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

    pub fn update_state(&self, model: &str) -> anyhow::Result<()> {
        // `model` should be in "provider:model" format (e.g. "openai:gpt-4o").
        let mut write = self
            .app_state
            .write()
            .map_err(|e| anyhow::anyhow!("Lock poisoned: {e}"))?;
        write.last_model = Some(model.to_string());
        Self::persist_state(&write);
        Ok(())
    }

    /// Resolve the verifier committee configuration.
    ///
    /// Priority (highest first):
    ///   1. Runtime-managed `state.verifier_committee` (via `/verifier add|delete`)
    ///   2. Static `security.verifier_committee` from config.toml (fallback)
    ///   3. Legacy `state.verifier_committee_members` (backward compat, removed field)
    ///
    /// Returns a tuple of:
    /// - All committee members as (provider, model) pairs.
    /// - Whether the verifier is enabled and has at least one member configured.
    pub fn get_verifier_committee(&self) -> (Vec<(String, String)>, bool) {
        let mut members: Vec<(String, String)> = Vec::new();

        // Primary source: runtime state (managed via /verifier add|delete)
        if let Ok(state) = self.get_state() {
            for pm in &state.verifier_committee {
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
        }

        // Fallback: config.toml (only if state list is empty)
        if members.is_empty()
            && let Ok(config) = self.get_config()
        {
            for pm in &config.security.verifier_committee {
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
        }

        let enabled = !members.is_empty();
        (members, enabled)
    }

    /// Add a verifier committee member (provider:model) to state.toml.
    ///
    /// This persists to state.toml so it survives restarts.
    /// Duplicate entries are silently ignored.
    pub fn add_verifier_committee_member(&self, provider_model: &str) -> anyhow::Result<()> {
        let mut write = self
            .app_state
            .write()
            .map_err(|e| anyhow::anyhow!("Lock poisoned: {e}"))?;
        let pm = provider_model.to_string();
        if !write.verifier_committee.contains(&pm) {
            write.verifier_committee.push(pm);
        }
        Self::persist_state(&write);
        Ok(())
    }

    /// Remove a verifier committee member (provider:model) from state.toml.
    ///
    /// Returns `true` if the member existed and was removed, `false` otherwise.
    pub fn remove_verifier_committee_member(&self, provider_model: &str) -> anyhow::Result<bool> {
        let mut write = self
            .app_state
            .write()
            .map_err(|e| anyhow::anyhow!("Lock poisoned: {e}"))?;
        let len_before = write.verifier_committee.len();
        write.verifier_committee.retain(|m| m != provider_model);
        let removed = write.verifier_committee.len() < len_before;
        if removed {
            Self::persist_state(&write);
        }
        Ok(removed)
    }

    /// List all verifier committee members from state.toml.
    pub fn list_verifier_committee_members(&self) -> Vec<String> {
        self.get_state()
            .ok()
            .map(|s| s.verifier_committee.clone())
            .unwrap_or_default()
    }
}
