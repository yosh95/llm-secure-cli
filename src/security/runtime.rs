use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Task {
    pub id: String,
    pub task_type: String,
    pub description: String,
    pub parameters: serde_json::Value,
    pub status: TaskStatus,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub enum TaskStatus {
    Pending,
    Running,
    Completed,
    Failed(String),
}

#[async_trait]
pub trait SecureRuntime: Send + Sync {
    async fn execute_task(&self, task: Task) -> Result<serde_json::Value>;
    async fn get_kms_key(&self, key_id: &str) -> Result<Vec<u8>>;
    fn name(&self) -> &str;
}

pub struct LocalRuntime;

#[async_trait]
impl SecureRuntime for LocalRuntime {
    async fn execute_task(&self, task: Task) -> Result<serde_json::Value> {
        // In local runtime, we just execute the tool if it matches
        // For now, this is a placeholder for actual local execution logic
        // that might be moved here later.
        Ok(serde_json::json!({
            "status": "executed_locally",
            "task_id": task.id,
            "result": "Local execution not fully implemented in runtime abstraction yet"
        }))
    }

    async fn get_kms_key(&self, _key_id: &str) -> Result<Vec<u8>> {
        // In local runtime, we might return keys from the local filesystem
        // or just a dummy key for now.
        Ok(vec![0u8; 32])
    }

    fn name(&self) -> &str {
        "local"
    }
}

pub struct TeeRuntime;

#[async_trait]
impl SecureRuntime for TeeRuntime {
    async fn execute_task(&self, _task: Task) -> Result<serde_json::Value> {
        // Future TEE implementation
        Err(anyhow::anyhow!("TEE runtime not yet implemented"))
    }

    async fn get_kms_key(&self, _key_id: &str) -> Result<Vec<u8>> {
        // Future KMS implementation
        Err(anyhow::anyhow!("KMS not yet implemented for TEE runtime"))
    }

    fn name(&self) -> &str {
        "tee"
    }
}

pub fn get_runtime(runtime_type: &str) -> Box<dyn SecureRuntime> {
    match runtime_type {
        "tee" => Box::new(TeeRuntime),
        _ => Box::new(LocalRuntime),
    }
}
