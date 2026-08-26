CREATE TABLE approvals (
 id TEXT PRIMARY KEY, task_id TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
 action TEXT NOT NULL, detail TEXT NOT NULL, status TEXT NOT NULL DEFAULT 'pending',
 created_at TEXT NOT NULL, resolved_at TEXT
);
CREATE INDEX approvals_task_status ON approvals(task_id, status);
