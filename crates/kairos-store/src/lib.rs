use anyhow::Result;
use chrono::Utc;
use kairos_core::{Approval, Task, TaskEvent, TaskStatus};
use sqlx::{Row, SqlitePool, sqlite::SqlitePoolOptions};
use uuid::Uuid;
pub struct Store {
    pub pool: SqlitePool,
}
impl Store {
    pub async fn connect(url: &str) -> Result<Self> {
        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect(url)
            .await?;
        sqlx::migrate!().run(&pool).await?;
        Ok(Self { pool })
    }
    pub async fn create_task(
        &self,
        title: &str,
        repo: &str,
        model: &str,
        budget: Option<f64>,
    ) -> Result<Task> {
        let now = Utc::now();
        let task = Task {
            id: Uuid::new_v4(),
            title: title.into(),
            repo: repo.into(),
            status: TaskStatus::Queued,
            model: model.into(),
            provider: "openrouter".into(),
            session_id: Uuid::new_v4().to_string(),
            budget_usd: budget,
            plan: Vec::new(),
            checkpoint: None,
            worktree: None,
            cost_usd: 0.0,
            created_at: now,
            updated_at: now,
        };
        sqlx::query("INSERT INTO tasks (id,title,repo,status,model,provider,session_id,budget_usd,plan,created_at,updated_at) VALUES (?,?,?,?,?,?,?,?,?,?,?)").bind(task.id.to_string()).bind(&task.title).bind(task.repo.to_string_lossy().to_string()).bind(task.status.to_string()).bind(&task.model).bind(&task.provider).bind(&task.session_id).bind(task.budget_usd).bind("[]").bind(now).bind(now).execute(&self.pool).await?;
        Ok(task)
    }
    pub async fn list_tasks(&self) -> Result<Vec<Task>> {
        let rows = sqlx::query("SELECT * FROM tasks ORDER BY updated_at DESC")
            .fetch_all(&self.pool)
            .await?;
        rows.into_iter().map(row_task).collect()
    }
    pub async fn get_task(&self, id: Uuid) -> Result<Option<Task>> {
        sqlx::query("SELECT * FROM tasks WHERE id=?")
            .bind(id.to_string())
            .fetch_optional(&self.pool)
            .await?
            .map(row_task)
            .transpose()
    }
    pub async fn set_status(&self, id: Uuid, status: TaskStatus) -> Result<()> {
        sqlx::query("UPDATE tasks SET status=?, updated_at=? WHERE id=?")
            .bind(status.to_string())
            .bind(Utc::now())
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }
    pub async fn set_worktree(&self, id: Uuid, path: &str) -> Result<()> {
        sqlx::query("UPDATE tasks SET worktree=?, updated_at=? WHERE id=?")
            .bind(path)
            .bind(Utc::now())
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }
    pub async fn set_plan(&self, id: Uuid, plan: &[String]) -> Result<()> {
        sqlx::query("UPDATE tasks SET plan=?, updated_at=? WHERE id=?")
            .bind(serde_json::to_string(plan)?)
            .bind(Utc::now())
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }
    pub async fn add_cost(&self, id: Uuid, cost: f64) -> Result<()> {
        sqlx::query("UPDATE tasks SET cost_usd=cost_usd+?, updated_at=? WHERE id=?")
            .bind(cost)
            .bind(Utc::now())
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }
    pub async fn create_approval(
        &self,
        task_id: Uuid,
        action: &str,
        detail: &str,
    ) -> Result<Approval> {
        let approval = Approval {
            id: Uuid::new_v4(),
            task_id,
            action: action.into(),
            detail: detail.into(),
            status: "pending".into(),
            created_at: Utc::now(),
            resolved_at: None,
        };
        sqlx::query("INSERT INTO approvals (id,task_id,action,detail,status,created_at) VALUES (?,?,?,?,?,?)").bind(approval.id.to_string()).bind(task_id.to_string()).bind(&approval.action).bind(&approval.detail).bind(&approval.status).bind(approval.created_at).execute(&self.pool).await?;
        Ok(approval)
    }
    pub async fn approvals(&self, task_id: Uuid) -> Result<Vec<Approval>> {
        let rows = sqlx::query(
            "SELECT * FROM approvals WHERE task_id=? AND status='pending' ORDER BY created_at",
        )
        .bind(task_id.to_string())
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|r| {
                Ok(Approval {
                    id: Uuid::parse_str(r.get::<String, _>("id").as_str())?,
                    task_id,
                    action: r.get("action"),
                    detail: r.get("detail"),
                    status: r.get("status"),
                    created_at: r.get("created_at"),
                    resolved_at: r.get("resolved_at"),
                })
            })
            .collect()
    }
    pub async fn resolve_approval(&self, id: Uuid, status: &str) -> Result<()> {
        sqlx::query("UPDATE approvals SET status=?, resolved_at=? WHERE id=?")
            .bind(status)
            .bind(Utc::now())
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }
    pub async fn cost_today(&self) -> Result<f64> {
        let start = Utc::now()
            .date_naive()
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_utc();
        Ok(sqlx::query_scalar(
            "SELECT COALESCE(SUM(cost_usd),0.0) FROM task_events WHERE created_at >= ?",
        )
        .bind(start)
        .fetch_one(&self.pool)
        .await?)
    }
    pub async fn add_event(&self, event: &TaskEvent) -> Result<()> {
        sqlx::query("INSERT INTO task_events (id,task_id,kind,message,output,duration_ms,input_tokens,output_tokens,cache_read_tokens,cache_write_tokens,cost_usd,created_at) VALUES (?,?,?,?,?,?,?,?,?,?,?,?)").bind(event.id.to_string()).bind(event.task_id.to_string()).bind(&event.kind).bind(&event.message).bind(&event.output).bind(event.duration_ms).bind(event.input_tokens).bind(event.output_tokens).bind(event.cache_read_tokens).bind(event.cache_write_tokens).bind(event.cost_usd).bind(event.created_at).execute(&self.pool).await?;
        Ok(())
    }
    pub async fn events(&self, id: Uuid) -> Result<Vec<TaskEvent>> {
        let rows = sqlx::query("SELECT * FROM task_events WHERE task_id=? ORDER BY created_at")
            .bind(id.to_string())
            .fetch_all(&self.pool)
            .await?;
        rows.into_iter()
            .map(|r| {
                Ok(TaskEvent {
                    id: Uuid::parse_str(r.get::<String, _>("id").as_str())?,
                    task_id: id,
                    kind: r.get("kind"),
                    message: r.get("message"),
                    output: r.get("output"),
                    duration_ms: r.get("duration_ms"),
                    input_tokens: r.get("input_tokens"),
                    output_tokens: r.get("output_tokens"),
                    cache_read_tokens: r.get("cache_read_tokens"),
                    cache_write_tokens: r.get("cache_write_tokens"),
                    cost_usd: r.get("cost_usd"),
                    created_at: r.get("created_at"),
                })
            })
            .collect()
    }
}
fn row_task(r: sqlx::sqlite::SqliteRow) -> Result<Task> {
    Ok(Task {
        id: Uuid::parse_str(r.get::<String, _>("id").as_str())?,
        title: r.get("title"),
        repo: r.get::<String, _>("repo").into(),
        status: r.get::<String, _>("status").parse()?,
        model: r.get("model"),
        provider: r.get("provider"),
        session_id: r.get("session_id"),
        budget_usd: r.get("budget_usd"),
        plan: serde_json::from_str(r.get::<String, _>("plan").as_str())?,
        checkpoint: r.get("checkpoint"),
        worktree: r.get::<Option<String>, _>("worktree").map(Into::into),
        cost_usd: r.get("cost_usd"),
        created_at: r.get("created_at"),
        updated_at: r.get("updated_at"),
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn persists_task() {
        let s = Store::connect("sqlite::memory:").await.unwrap();
        let t = s.create_task("test", ".", "model", None).await.unwrap();
        assert_eq!(s.list_tasks().await.unwrap().len(), 1);
        s.set_status(t.id, TaskStatus::Running).await.unwrap();
        assert_eq!(
            s.get_task(t.id).await.unwrap().unwrap().status,
            TaskStatus::Running
        );
    }
}
