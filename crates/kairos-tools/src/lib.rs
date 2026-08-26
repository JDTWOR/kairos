use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
    pub requires_approval: bool,
}
pub const INITIAL_TOOLS: &[&str] = &[
    "read_file",
    "write_file",
    "patch_file",
    "list_files",
    "search_text",
    "run_command",
    "git_status",
    "git_diff",
    "git_commit",
    "run_tests",
    "docker_status",
    "docker_logs",
    "ssh_run",
];
pub fn requires_approval(name: &str) -> bool {
    matches!(
        name,
        "write_file"
            | "patch_file"
            | "run_command"
            | "git_commit"
            | "docker_status"
            | "docker_logs"
            | "ssh_run"
    )
}
