use crate::security::skill::types::{
    MAX_DESCRIPTION_LENGTH, MAX_NAME_LENGTH, SkillContent, SkillMetadata, SkillValidationError,
    VALID_NAME_PATTERN,
};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Parses a SKILL.md file into structured metadata and body content.
///
/// The Agent Skills specification defines a simple format:
/// - YAML frontmatter delimited by `---` lines
/// - Required fields: `name` (max 64 chars, lowercase+hyphens) and
///   `description` (max 1024 chars)
/// - Optional fields: `license`, `compatibility`, `metadata`,
///   `allowed-tools`
/// - The rest is Markdown instructions
pub fn parse_skill_file(path: &Path) -> Result<SkillContent, Vec<SkillValidationError>> {
    let skill_md_path = find_skill_md(path)?;
    let raw = std::fs::read_to_string(&skill_md_path).map_err(|e| {
        vec![SkillValidationError::InvalidYaml(format!(
            "Cannot read file: {e}"
        ))]
    })?;

    let mut errors = Vec::new();

    // Split by --- delimiters to extract frontmatter
    let parts: Vec<&str> = raw.splitn(3, "---\n").collect();

    if parts.len() < 2 || !raw.starts_with("---") {
        return Err(vec![SkillValidationError::NoYamlFrontmatter]);
    }

    let frontmatter_raw = if parts.len() >= 2 { parts[1] } else { "" };
    let body = if parts.len() >= 3 { parts[2] } else { "" };

    // Parse the YAML frontmatter
    let metadata = match parse_frontmatter(frontmatter_raw) {
        Ok(m) => m,
        Err(e) => {
            errors.push(e);
            // Continue to collect more errors
            SkillMetadata {
                name: String::new(),
                description: String::new(),
                license: None,
                compatibility: None,
                metadata: HashMap::new(),
                allowed_tools: None,
            }
        }
    };

    // Validate name
    if metadata.name.is_empty() {
        errors.push(SkillValidationError::MissingName);
    } else if metadata.name.len() > MAX_NAME_LENGTH {
        errors.push(SkillValidationError::NameTooLong {
            actual: metadata.name.len(),
            max: MAX_NAME_LENGTH,
        });
    } else if !metadata
        .name
        .chars()
        .all(|c| VALID_NAME_PATTERN.contains(c))
    {
        errors.push(SkillValidationError::InvalidName(metadata.name.clone()));
    }

    // Validate description
    if metadata.description.is_empty() {
        errors.push(SkillValidationError::MissingDescription);
    } else if metadata.description.len() > MAX_DESCRIPTION_LENGTH {
        errors.push(SkillValidationError::DescriptionTooLong {
            actual: metadata.description.len(),
            max: MAX_DESCRIPTION_LENGTH,
        });
    }

    if !errors.is_empty() {
        return Err(errors);
    }

    Ok(SkillContent {
        metadata,
        body: body.to_string(),
        raw_frontmatter: frontmatter_raw.to_string(),
    })
}

/// Finds the SKILL.md file in a directory (case-insensitive).
fn find_skill_md(dir: &Path) -> Result<PathBuf, Vec<SkillValidationError>> {
    if !dir.is_dir() {
        return Err(vec![SkillValidationError::MissingSkillMd]);
    }

    // Try exact match first
    let exact = dir.join("SKILL.md");
    if exact.exists() {
        return Ok(exact);
    }

    // Try case-insensitive
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy().to_lowercase();
            if name_str == "skill.md" {
                return Ok(entry.path());
            }
        }
    }

    Err(vec![SkillValidationError::MissingSkillMd])
}

/// Minimal YAML frontmatter parser for the Agent Skills spec.
///
/// Handles:
/// - Simple `key: value` pairs
/// - `>-` folded block scalars (multi-line descriptions)
/// - Nested `metadata:` blocks (one level deep)
/// - `allowed-tools:` list items (lines starting with `  - `)
fn parse_frontmatter(raw: &str) -> Result<SkillMetadata, SkillValidationError> {
    let mut name = String::new();
    let mut description = String::new();
    let mut license: Option<String> = None;
    let mut compatibility: Option<String> = None;
    let mut extra_metadata = HashMap::new();
    let mut allowed_tools: Option<Vec<String>> = None;

    let lines: Vec<&str> = raw.lines().collect();
    let mut i = 0;

    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim();

        // Skip empty lines and comments
        if trimmed.is_empty() || trimmed.starts_with('#') {
            i += 1;
            continue;
        }

        // Try to parse `key: value`
        if let Some((key, value)) = parse_kv_line(trimmed) {
            match key {
                "name" => name = value.to_string(),
                "description" => {
                    // Check if this is a folded block scalar (>-)
                    if value == ">-" || value.starts_with(">-") {
                        // Collect continuation lines that are indented
                        let mut desc_parts = Vec::new();
                        i += 1;
                        while i < lines.len() {
                            let cont = lines[i];
                            if cont.is_empty()
                                || (!cont.starts_with(' ') && !cont.starts_with('\t'))
                            {
                                // Check if this looks like a new top-level key
                                if cont.contains(':')
                                    && !cont.starts_with(' ')
                                    && !cont.starts_with('\t')
                                {
                                    break;
                                }
                                // Empty line might end the block scalar
                                if cont.is_empty() {
                                    i += 1;
                                    break;
                                }
                            }
                            desc_parts.push(cont.trim().to_string());
                            i += 1;
                        }
                        description = desc_parts.join(" ");
                        continue; // i is already advanced
                    }
                    description = value.to_string();
                }
                "license" => license = Some(value.to_string()),
                "compatibility" => compatibility = Some(value.to_string()),
                "allowed-tools" => {
                    // Could be inline list: [a, b] or block list
                    if value == "[" || value.starts_with('[') {
                        // Inline list - rough parse
                        let list_str = if value.starts_with('[') {
                            // The value might be a full list
                            let full = if value.ends_with(']') {
                                value.to_string()
                            } else {
                                // Collect until ]
                                let mut full_val = value.to_string();
                                i += 1;
                                while i < lines.len() {
                                    full_val.push_str(lines[i].trim());
                                    if lines[i].trim().ends_with(']') {
                                        i += 1;
                                        break;
                                    }
                                    i += 1;
                                }
                                full_val
                            };
                            full.trim_start_matches('[')
                                .trim_end_matches(']')
                                .to_string()
                        } else {
                            value.to_string()
                        };
                        let tools: Vec<String> = list_str
                            .split(',')
                            .map(|s| s.trim().trim_matches('"').trim_matches('\'').to_string())
                            .filter(|s| !s.is_empty())
                            .collect();
                        if !tools.is_empty() {
                            allowed_tools = Some(tools);
                        }
                    }
                    // Block list handled below in the metadata parsing
                }
                other => {
                    extra_metadata.insert(other.to_string(), value.to_string());
                }
            }
            i += 1;
        } else if trimmed.starts_with("  - ") && allowed_tools.is_some() {
            // Block list item for allowed-tools (already started)
            let item = trimmed.trim_start_matches("  - ").trim().to_string();
            if let Some(ref mut tools) = allowed_tools {
                tools.push(item);
            }
            i += 1;
        } else if trimmed.starts_with('-') && i > 0 {
            // Check if the previous line was "allowed-tools:" or "allowed-tools:"
            let prev = if i > 0 {
                lines[i - 1].trim().to_string()
            } else {
                String::new()
            };
            if prev == "allowed-tools:" {
                let item = trimmed.trim_start_matches('-').trim().to_string();
                allowed_tools.get_or_insert_with(Vec::new).push(item);
            }
            i += 1;
        } else if trimmed.ends_with(':') && !trimmed.contains(' ') {
            // Nested block like `metadata:` — skip for simplicity
            i += 1;
            while i < lines.len() && (lines[i].starts_with(' ') || lines[i].starts_with('\t')) {
                i += 1;
            }
        } else {
            i += 1;
        }
    }

    Ok(SkillMetadata {
        name,
        description,
        license,
        compatibility,
        metadata: extra_metadata,
        allowed_tools,
    })
}

/// Parses a single `key: value` line. Returns None if it's not a simple KV pair.
fn parse_kv_line(line: &str) -> Option<(&str, &str)> {
    // Find the first colon that is followed by a space or end of line
    let colon_pos = line.find(':')?;

    let key = line[..colon_pos].trim();
    let value = line[colon_pos + 1..].trim();

    // Key must be non-empty and not contain spaces
    if key.is_empty() || key.contains(' ') {
        return None;
    }

    Some((key, value))
}

/// Validates the structural conformance of a skill directory.
/// Returns Ok(metadata) if valid, or a list of validation errors.
#[must_use]
pub fn validate_skill_structure(dir: &Path) -> crate::security::skill::types::SkillStructureResult {
    match parse_skill_file(dir) {
        Ok(content) => crate::security::skill::types::SkillStructureResult::Pass {
            metadata: content.metadata,
        },
        Err(errors) => crate::security::skill::types::SkillStructureResult::Fail {
            errors: errors
                .iter()
                .map(std::string::ToString::to_string)
                .collect(),
        },
    }
}

/// Discovers skill directories (containing SKILL.md) recursively.
#[must_use]
pub fn discover_skills(root: &Path, recursive: bool) -> Vec<PathBuf> {
    let mut skills = Vec::new();

    if !root.is_dir() {
        return skills;
    }

    // Check if root itself is a skill
    if find_skill_md(root).is_ok() {
        skills.push(root.to_path_buf());
        if !recursive {
            return skills;
        }
    }

    if recursive && let Ok(entries) = std::fs::read_dir(root) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                skills.extend(discover_skills(&path, true));
            }
        }
    }

    skills
}
