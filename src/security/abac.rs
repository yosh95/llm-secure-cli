use crate::config::CONFIG_MANAGER;
use crate::security::policy::EvaluationContext;
use serde_json::Value;

pub struct AbacEngine;

impl AbacEngine {
    pub fn evaluate(context: &EvaluationContext) -> Option<String> {
        let config = CONFIG_MANAGER.get_config();
        let rules = &config.security.abac_rules;

        for rule in rules {
            if Self::matches_rule(rule, context) {
                log::debug!("ABAC Rule '{}' matched. Effect: {}", rule.name, rule.effect);
                return Some(rule.effect.clone());
            }
        }

        None
    }

    fn matches_rule(rule: &crate::config::models::AbacRule, context: &EvaluationContext) -> bool {
        for (key, expected_value) in &rule.match_attributes {
            match context.get_attribute(key) {
                Some(actual_value) => {
                    if !Self::values_match(expected_value, actual_value) {
                        return false;
                    }
                }
                None => return false, // Attribute required but not found
            }
        }
        true
    }

    fn values_match(expected: &Value, actual: &Value) -> bool {
        match (expected, actual) {
            (Value::String(e), Value::String(a)) => e == a,
            (Value::Number(e), Value::Number(a)) => e == a,
            (Value::Bool(e), Value::Bool(a)) => e == a,
            (Value::Array(e_arr), Value::Array(a_arr)) => {
                // If it's an array, we might want to check for "contains" or exact match.
                // For simplicity, let's start with exact match.
                e_arr == a_arr
            }
            _ => false,
        }
    }
}
