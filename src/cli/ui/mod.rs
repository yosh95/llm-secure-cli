//! User interface module — terminal output, prompts, and interactive widgets.
//!
//! This module provides the [`UserInterface`] trait and its concrete
//! implementation [`CliUi`], along with free-standing helper functions
//! for formatted output.

pub mod display;
pub mod prompt;
pub mod report;

pub use display::{
    format_tool_call, print_block, print_key_value, print_panel, print_rule, print_tool_call,
    print_tool_call_direct, print_tool_result,
};
pub use prompt::{
    ConfirmResult, PromptMode, ask_confirm, ask_confirm_async, ask_confirm_simple,
    ask_confirm_simple_async, get_user_input, open_external_editor,
};
pub use report::{report_error, report_info, report_success, report_warning};

use async_trait::async_trait;

/// Abstract user interface trait — enables testing with mock implementations.
#[async_trait]
pub trait UserInterface: Send + Sync {
    fn print_block(&self, content: &str, title: Option<&str>, style: Option<&str>);
    fn print_rule(&self, title: Option<&str>, style: Option<&str>);
    fn print_tool_call(&self, name: &str, args: &serde_json::Value);
    fn print_tool_call_direct(&self, name: &str, args: &serde_json::Value);
    fn print_tool_result(&self, result: &str);
    fn report_error(&self, message: &str);
    fn report_info(&self, message: &str);
    fn report_warning(&self, message: &str);
    fn report_success(&self, message: &str);
    async fn ask_confirm(&self, prompt: &str) -> Option<ConfirmResult>;
    async fn ask_confirm_simple(&self, prompt: &str) -> Option<ConfirmResult>;
}

/// Concrete UI implementation using terminal output.
pub struct CliUi;

#[async_trait]
impl UserInterface for CliUi {
    fn print_block(&self, content: &str, title: Option<&str>, style: Option<&str>) {
        display::print_block(content, title, style);
    }
    fn print_rule(&self, title: Option<&str>, style: Option<&str>) {
        display::print_rule(title, style);
    }
    fn print_tool_call(&self, name: &str, args: &serde_json::Value) {
        display::print_tool_call(name, args);
    }
    fn print_tool_call_direct(&self, name: &str, args: &serde_json::Value) {
        display::print_tool_call_direct(name, args);
    }
    fn print_tool_result(&self, result: &str) {
        display::print_tool_result(result);
    }
    fn report_error(&self, message: &str) {
        report::report_error(message);
    }
    fn report_info(&self, message: &str) {
        report::report_info(message);
    }
    fn report_warning(&self, message: &str) {
        report::report_warning(message);
    }
    fn report_success(&self, message: &str) {
        report::report_success(message);
    }
    async fn ask_confirm(&self, prompt: &str) -> Option<ConfirmResult> {
        prompt::ask_confirm_async(prompt).await
    }
    async fn ask_confirm_simple(&self, prompt: &str) -> Option<ConfirmResult> {
        prompt::ask_confirm_simple_async(prompt).await
    }
}
