use anyhow::{Result, bail};
use chrono::Utc;
use clap::{Parser, Subcommand};
use kairos_core::{AppConfig, TaskEvent, TaskStatus, normalize_repo};
use kairos_provider::{Message, OpenRouter};
use kairos_runner::Runner;
use kairos_store::Store;
use std::{path::PathBuf, process::Stdio};
use tokio::process::Command;
use uuid::Uuid;

#[derive(Parser)]
#[command(
    name = "kairos",
    version,
    about = "Persistent, terminal-first personal agent"
)]
struct Cli {
    #[command(subcommand)]
    command: CommandKind,
}
#[derive(Subcommand)]
enum CommandKind {
    Chat,
    Task {
        prompt: String,
        #[arg(long)]
        repo: Option<PathBuf>,
        #[arg(long)]
        detach: bool,
        #[arg(long)]
        budget: Option<f64>,
    },
    Status,
    Watch {
        id: Option<Uuid>,
    },
    Logs {
        id: Uuid,
    },
    Resume {
        id: Uuid,
        #[arg(long, hide = true)]
        background: bool,
    },
    Pause {
        id: Uuid,
    },
    Cancel {
        id: Uuid,
    },
    Approve {
        task_id: Uuid,
        action_id: Uuid,
    },
    Diff {
        id: Uuid,
    },
    Cost {
        #[command(subcommand)]
        command: CostCommand,
    },
    Doctor,
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
}
#[derive(Subcommand)]
enum CostCommand {
    Today,
}
#[derive(Subcommand)]
enum ConfigCommand {
    Init,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    let cli = Cli::parse();
    let config = AppConfig::load()?;
    let store = Store::connect(&config.database_url).await?;
    match cli.command {
        CommandKind::Task {
            prompt,
            repo,
            detach,
            budget,
        } => {
            let repo = normalize_repo(repo.unwrap_or(std::env::current_dir()?))?;
            let task = store
                .create_task(&prompt, &repo.to_string_lossy(), &config.model, budget)
                .await?;
            println!("Created #{} {}", &task.id.to_string()[..8], task.title);
            if detach {
                Command::new(std::env::current_exe()?)
                    .arg("resume")
                    .arg(task.id.to_string())
                    .arg("--background")
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()?;
                println!(
                    "Detached execution started; follow with `kairos watch {}`.",
                    task.id
                );
            } else {
                println!("Use `kairos resume {}` to start it.", task.id);
            }
        }
        CommandKind::Status => {
            for t in store.list_tasks().await? {
                println!(
                    "{}  {:<18} {}  ${:.4}  {}",
                    &t.id.to_string()[..8],
                    t.status,
                    t.title,
                    t.cost_usd,
                    t.repo.display()
                );
            }
        }
        CommandKind::Logs { id } => {
            for e in store.events(id).await? {
                println!(
                    "{} [{}] {}{}",
                    e.created_at.format("%H:%M:%S"),
                    e.kind,
                    e.message,
                    e.output.map(|s| format!("\n{s}")).unwrap_or_default()
                );
            }
        }
        CommandKind::Resume { id, background } => {
            execute_task(&store, &config, id, background).await?
        }
        CommandKind::Pause { id } => {
            store.set_status(id, TaskStatus::Paused).await?;
            println!("Paused {id}");
        }
        CommandKind::Cancel { id } => {
            store.set_status(id, TaskStatus::Cancelled).await?;
            println!("Cancelled {id}");
        }
        CommandKind::Approve { task_id, action_id } => {
            if store
                .approvals(task_id)
                .await?
                .iter()
                .any(|a| a.id == action_id)
            {
                store.resolve_approval(action_id, "approved").await?;
                println!("Approved {action_id}");
            } else {
                bail!("pending approval not found for task");
            }
        }
        CommandKind::Diff { id } => {
            let t = store
                .get_task(id)
                .await?
                .ok_or_else(|| anyhow::anyhow!("task not found"))?;
            println!(
                "{}",
                Runner::new(t.worktree.unwrap_or(t.repo)).git_diff().await?
            );
        }
        CommandKind::Cost {
            command: CostCommand::Today,
        } => println!("${:.6}", store.cost_today().await?),
        CommandKind::Watch { id } => kairos_tui::run(store, id).await?,
        CommandKind::Chat => kairos_tui::run(store, None).await?,
        CommandKind::Doctor => println!(
            "Kairos doctor\n  database: ok\n  OPENROUTER_API_KEY: {}\n  model: {}",
            if std::env::var_os("OPENROUTER_API_KEY").is_some() {
                "set"
            } else {
                "missing"
            },
            config.model
        ),
        CommandKind::Config {
            command: ConfigCommand::Init,
        } => {
            config.save()?;
            println!("Configuration written to {}", AppConfig::path()?.display());
        }
    }
    Ok(())
}

async fn execute_task(store: &Store, config: &AppConfig, id: Uuid, background: bool) -> Result<()> {
    let task = store
        .get_task(id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("task not found"))?;
    if matches!(task.status, TaskStatus::Cancelled | TaskStatus::Completed) {
        bail!("task is not resumable from {}", task.status);
    }
    let worktree = if let Some(path) = task.worktree.clone() {
        path
    } else {
        let result = Runner::new(&task.repo)
            .create_worktree(&format!("kairos-{}", &id.to_string()[..8]))
            .await;
        let path = match result {
            Ok(path) => path,
            Err(error) => {
                store.set_status(id, TaskStatus::Failed).await?;
                add_event(
                    store,
                    id,
                    "error",
                    "worktree creation failed",
                    Some(error.to_string()),
                    0,
                    0,
                    0,
                    0,
                    0.0,
                )
                .await?;
                return Err(error);
            }
        };
        store.set_worktree(id, &path.to_string_lossy()).await?;
        path
    };
    store.set_status(id, TaskStatus::Planning).await?;
    add_event(
        store,
        id,
        "system",
        "planning started",
        None,
        0,
        0,
        0,
        0,
        0.0,
    )
    .await?;
    let provider = match OpenRouter::from_env(task.model.clone(), config.fallbacks.clone()) {
        Ok(p) => p,
        Err(e) => {
            store.set_status(id, TaskStatus::Failed).await?;
            add_event(
                store,
                id,
                "error",
                "provider unavailable",
                Some(e.to_string()),
                0,
                0,
                0,
                0,
                0.0,
            )
            .await?;
            return Err(e);
        }
    };
    let messages=vec![Message{role:"system".into(),content:"You are Kairos, a persistent terminal agent. Rules: use repository context first; be concise; never claim an action you did not execute; propose a numbered plan before changes. Dynamic task information follows this stable prefix.".into()},Message{role:"user".into(),content:format!("Task: {}\nRepository: {}\nWorktree: {}\nResume from checkpoint: {}",task.title,task.repo.display(),worktree.display(),task.checkpoint.as_deref().unwrap_or("none"))}];
    store.set_status(id, TaskStatus::Running).await?;
    match provider.prompt(messages, &task.session_id).await {
        Ok((output, usage)) => {
            let plan = output
                .lines()
                .filter(|l| {
                    let s = l.trim_start();
                    s.starts_with(char::is_numeric) || s.starts_with('-')
                })
                .take(12)
                .map(|s| {
                    s.trim()
                        .trim_start_matches(|c: char| {
                            c.is_numeric() || c == '.' || c == '-' || c == ' '
                        })
                        .to_string()
                })
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>();
            if !plan.is_empty() {
                store.set_plan(id, &plan).await?;
            }
            let cost = usage.cost.unwrap_or(0.0);
            store.add_cost(id, cost).await?;
            add_event(
                store,
                id,
                "model",
                "response received",
                Some(output),
                usage.prompt_tokens as i64,
                usage.completion_tokens as i64,
                usage
                    .prompt_tokens_details
                    .and_then(|d| d.cached_tokens)
                    .unwrap_or(0) as i64,
                0,
                cost,
            )
            .await?;
            store.set_status(id, TaskStatus::Verifying).await?;
            let status = Runner::new(&worktree)
                .git_status()
                .await
                .unwrap_or_else(|e| e.to_string());
            add_event(
                store,
                id,
                "verification",
                "git status captured",
                Some(status),
                0,
                0,
                0,
                0,
                0.0,
            )
            .await?;
            store.set_status(id, TaskStatus::Completed).await?;
            if !background {
                println!("Completed #{}", &id.to_string()[..8]);
            }
        }
        Err(e) => {
            store.set_status(id, TaskStatus::Failed).await?;
            add_event(
                store,
                id,
                "error",
                "provider request failed",
                Some(e.to_string()),
                0,
                0,
                0,
                0,
                0.0,
            )
            .await?;
            return Err(e);
        }
    }
    Ok(())
}
#[allow(clippy::too_many_arguments)]
async fn add_event(
    store: &Store,
    id: Uuid,
    kind: &str,
    message: &str,
    output: Option<String>,
    input: i64,
    output_tokens: i64,
    cache_read: i64,
    cache_write: i64,
    cost: f64,
) -> Result<()> {
    store
        .add_event(&TaskEvent {
            id: Uuid::new_v4(),
            task_id: id,
            kind: kind.into(),
            message: message.into(),
            output,
            duration_ms: None,
            input_tokens: input,
            output_tokens,
            cache_read_tokens: cache_read,
            cache_write_tokens: cache_write,
            cost_usd: cost,
            created_at: Utc::now(),
        })
        .await
}
