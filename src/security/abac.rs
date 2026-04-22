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
            (Value::String(e), Value::String(a)) => {
                if let Some(prefix) = e.strip_prefix("prefix:") {
                    a.starts_with(prefix)
                } else {
                    e == a
                }
            }
            (Value::Number(e), Value::Number(a)) => e == a,
            (Value::Bool(e), Value::Bool(a)) => e == a,
            (Value::Array(e_arr), Value::Array(a_arr)) => {
                // If expected is an array, we check if all elements in expected are in actual
                e_arr.iter().all(|e_val| a_arr.contains(e_val))
            }
            (Value::String(_e), Value::Array(a_arr)) => {
                // If expected is a string and actual is an array, check if array contains string
                a_arr.contains(expected)
            }
            _ => false,
        }
    }
}
