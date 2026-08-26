use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    str::FromStr,
};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Queued,
    Planning,
    Running,
    WaitingApproval,
    Verifying,
    Completed,
    Failed,
    Paused,
    Cancelled,
    NeedsInput,
}
impl TaskStatus {
    pub fn can_resume(self) -> bool {
        matches!(
            self,
            Self::Paused | Self::Failed | Self::NeedsInput | Self::Queued
        )
    }
}
impl std::fmt::Display for TaskStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            serde_json::to_string(self)
                .unwrap()
                .trim_matches('"')
                .replace('_', " ")
        )
    }
}
impl FromStr for TaskStatus {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(serde_json::from_value(serde_json::Value::String(
            s.replace(' ', "_"),
        ))?)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: Uuid,
    pub title: String,
    pub repo: PathBuf,
    pub status: TaskStatus,
    pub model: String,
    pub provider: String,
    pub session_id: String,
    pub budget_usd: Option<f64>,
    pub plan: Vec<String>,
    pub checkpoint: Option<String>,
    pub worktree: Option<PathBuf>,
    pub cost_usd: f64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskEvent {
    pub id: Uuid,
    pub task_id: Uuid,
    pub kind: String,
    pub message: String,
    pub output: Option<String>,
    pub duration_ms: Option<i64>,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_write_tokens: i64,
    pub cost_usd: f64,
    pub created_at: DateTime<Utc>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Approval {
    pub id: Uuid,
    pub task_id: Uuid,
    pub action: String,
    pub detail: String,
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub resolved_at: Option<DateTime<Utc>>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub model: String,
    pub fallbacks: Vec<String>,
    pub database_url: String,
    pub max_output_bytes: usize,
}
impl Default for AppConfig {
    fn default() -> Self {
        Self {
            model: "deepseek/deepseek-chat".into(),
            fallbacks: vec!["openai/gpt-4o-mini".into()],
            database_url: "sqlite://./kairos.db?mode=rwc".into(),
            max_output_bytes: 32_000,
        }
    }
}
impl AppConfig {
    pub fn path() -> anyhow::Result<PathBuf> {
        Ok(directories::ProjectDirs::from("com", "kairos", "kairos")
            .ok_or_else(|| anyhow::anyhow!("cannot determine config directory"))?
            .config_dir()
            .join("config.json"))
    }
    pub fn load() -> anyhow::Result<Self> {
        let path = Self::path()?;
        if !path.exists() {
            return Ok(Self::default());
        }
        Ok(serde_json::from_slice(&std::fs::read(path)?)?)
    }
    pub fn save(&self) -> anyhow::Result<()> {
        let path = Self::path()?;
        std::fs::create_dir_all(path.parent().unwrap())?;
        std::fs::write(path, serde_json::to_vec_pretty(self)?)?;
        Ok(())
    }
}
pub fn normalize_repo(path: impl AsRef<Path>) -> anyhow::Result<PathBuf> {
    let path = path.as_ref();
    Ok(if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    }
    .canonicalize()?)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn status_roundtrip() {
        for status in [
            TaskStatus::Queued,
            TaskStatus::WaitingApproval,
            TaskStatus::Completed,
        ] {
            assert_eq!(status, status.to_string().parse().unwrap());
        }
    }
}
