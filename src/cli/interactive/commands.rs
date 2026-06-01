//! Central registry of all interactive (slash) commands.
//!
//! Every top-level interactive command **must** be registered here.
//! Both `completion.rs` (Tab-completion) and `dispatcher.rs` (command dispatch)
//! derive the canonical command list from this module.
//!
//! ## Adding a new command
//!
//! 1. Add a new entry to [`INTERACTIVE_COMMANDS`] with the primary name and all aliases.
//! 2. Add the corresponding match arm in `dispatcher.rs`.
//! 3. Add argument-completion logic in `completion.rs` if needed.
//!
//! ## Verification
//!
//! Run `cargo test` to assert that the completion list
//! and the dispatcher match arms are in sync.

/// A single interactive slash command, including its aliases.
#[derive(Debug)]
pub struct CommandEntry {
    /// Canonical (primary) name, e.g. `"help"`.
    pub name: &'static str,
    /// Aliases, e.g. `&["h"]`.
    pub aliases: &'static [&'static str],
    /// One-line description for `--help` / `/help`.
    pub description: &'static str,
}

/// The canonical list of all interactive commands.
///
/// ⚠️ **Keep this list sorted** by primary `name` for readability.
pub const INTERACTIVE_COMMANDS: &[CommandEntry] = &[
    CommandEntry {
        name: "alias",
        aliases: &[],
        description: "List/create/delete model aliases",
    },
    CommandEntry {
        name: "attach",
        aliases: &[],
        description: "Attach a file or URL to the next request",
    },
    CommandEntry {
        name: "clear",
        aliases: &["c"],
        description: "Clear conversation history",
    },
    CommandEntry {
        name: "credits",
        aliases: &[],
        description: "Show detailed OpenRouter credit info",
    },
    CommandEntry {
        name: "edit_history",
        aliases: &["eh"],
        description: "View/edit the conversation history in TOML format",
    },
    CommandEntry {
        name: "help",
        aliases: &["h"],
        description: "Show this help message",
    },
    CommandEntry {
        name: "info",
        aliases: &["i"],
        description: "Show session and security status",
    },
    CommandEntry {
        name: "model",
        aliases: &["m"],
        description: "List models or switch (provider:model)",
    },
    CommandEntry {
        name: "quit",
        aliases: &["q"],
        description: "Exit the session",
    },
    CommandEntry {
        name: "raw",
        aliases: &[],
        description: "Show raw conversation history",
    },
    CommandEntry {
        name: "session",
        aliases: &[],
        description: "List, load, delete, or clear saved sessions",
    },
    CommandEntry {
        name: "summarize",
        aliases: &["s"],
        description: "Summarize history and clear it",
    },
    CommandEntry {
        name: "template",
        aliases: &["t"],
        description: "List templates or insert one into prompt",
    },
    CommandEntry {
        name: "tool_output",
        aliases: &["to"],
        description: "Toggle display of tool execution results",
    },
    CommandEntry {
        name: "tools",
        aliases: &[],
        description: "Toggle or show status of tool execution",
    },
    CommandEntry {
        name: "vcommittee",
        aliases: &["vcom"],
        description: "Manage verifier committee",
    },
    CommandEntry {
        name: "verify",
        aliases: &[],
        description: "Toggle verifier on/off",
    },
    CommandEntry {
        name: "view",
        aliases: &[],
        description: "Open saved image or file with system default app",
    },
];

/// Returns all command names prefixed with `/`, including aliases,
/// suitable for Tab-completion.
#[must_use]
pub fn all_slash_commands() -> Vec<String> {
    let mut cmds: Vec<String> = Vec::with_capacity(INTERACTIVE_COMMANDS.len() * 2);
    for entry in INTERACTIVE_COMMANDS {
        cmds.push(format!("/{}", entry.name));
        for alias in entry.aliases {
            cmds.push(format!("/{alias}"));
        }
    }
    cmds.sort();
    cmds
}

/// Check whether a given slash-command string (e.g., `"/help"` or `"/h"`)
/// is a known command.
#[must_use]
pub fn is_valid_command(cmd: &str) -> bool {
    let raw = cmd.strip_prefix('/').unwrap_or(cmd);
    INTERACTIVE_COMMANDS
        .iter()
        .any(|e| e.name == raw || e.aliases.contains(&raw))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_all_slash_commands_includes_all_names() {
        let s = all_slash_commands();
        for entry in INTERACTIVE_COMMANDS {
            let slash_name = format!("/{}", entry.name);
            assert!(
                s.contains(&slash_name),
                "Missing /{} in all_slash_commands()",
                entry.name
            );
        }
    }

    #[test]
    fn test_is_valid_command() {
        assert!(is_valid_command("/help"));
        assert!(is_valid_command("/h"));
        assert!(is_valid_command("/alias"));
        assert!(is_valid_command("/vcom"));
        assert!(is_valid_command("/tool_output"));
        assert!(!is_valid_command("/nonexistent"));
    }

    #[test]
    fn test_no_duplicate_aliases() {
        let mut seen = std::collections::HashSet::new();
        for entry in INTERACTIVE_COMMANDS {
            assert!(
                seen.insert(entry.name),
                "Duplicate primary name: {}",
                entry.name
            );
            for alias in entry.aliases {
                assert!(
                    seen.insert(alias),
                    "Duplicate alias: {} (primary: {})",
                    alias,
                    entry.name
                );
            }
        }
    }

    #[test]
    fn test_aliases_are_not_primary_names() {
        let names: std::collections::HashSet<&&str> =
            INTERACTIVE_COMMANDS.iter().map(|e| &e.name).collect();
        for entry in INTERACTIVE_COMMANDS {
            for alias in entry.aliases {
                assert!(
                    !names.contains(alias),
                    "Alias '{}' conflicts with primary name",
                    alias
                );
            }
        }
    }
}
