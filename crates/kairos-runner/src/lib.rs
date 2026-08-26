use anyhow::{Result, bail};
use std::path::{Path, PathBuf};
use tokio::process::Command;
pub struct Runner {
    pub repo: PathBuf,
    pub max_output: usize,
}
impl Runner {
    pub fn new(repo: impl Into<PathBuf>) -> Self {
        Self {
            repo: repo.into(),
            max_output: 32_000,
        }
    }
    pub async fn git_status(&self) -> Result<String> {
        self.git(&["status", "--short"]).await
    }
    pub async fn git_diff(&self) -> Result<String> {
        self.git(&["diff", "--no-ext-diff"]).await
    }
    pub async fn create_worktree(&self, name: &str) -> Result<PathBuf> {
        let root = self.repo.join(".kairos-worktrees");
        tokio::fs::create_dir_all(&root).await?;
        let path = root.join(name);
        let path_str = path.to_string_lossy().to_string();
        self.git(&["worktree", "add", "-b", name, &path_str])
            .await?;
        Ok(path)
    }
    pub async fn run_command(
        &self,
        program: &str,
        args: &[String],
        cwd: Option<&Path>,
    ) -> Result<(String, i32)> {
        if program == "sudo" || args.iter().any(|a| a == "sudo") {
            bail!("sudo commands require approval")
        }
        let dir = cwd.unwrap_or(&self.repo);
        let out = Command::new(program)
            .args(args)
            .current_dir(dir)
            .output()
            .await?;
        let mut text = String::from_utf8_lossy(&out.stdout).to_string();
        text.push_str(&String::from_utf8_lossy(&out.stderr));
        text.truncate(self.max_output);
        Ok((text, out.status.code().unwrap_or(-1)))
    }
    async fn git(&self, args: &[&str]) -> Result<String> {
        let (out, code) = self
            .run_command(
                "git",
                &args.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
                None,
            )
            .await?;
        if code != 0 {
            bail!("git failed ({code}): {out}")
        }
        Ok(out)
    }
}
