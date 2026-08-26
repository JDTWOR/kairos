CREATE TABLE conversations (
 id TEXT PRIMARY KEY,
 title TEXT NOT NULL,
 repo TEXT NOT NULL,
 session_id TEXT NOT NULL UNIQUE,
 created_at TEXT NOT NULL,
 updated_at TEXT NOT NULL
);
CREATE INDEX conversations_repo_updated_at ON conversations(repo, updated_at);

CREATE TABLE messages (
 id TEXT PRIMARY KEY,
 conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
 role TEXT NOT NULL,
 content TEXT NOT NULL,
 created_at TEXT NOT NULL
);
CREATE INDEX messages_conversation_created_at ON messages(conversation_id, created_at);

ALTER TABLE tasks ADD COLUMN conversation_id TEXT REFERENCES conversations(id);
CREATE INDEX tasks_conversation_id ON tasks(conversation_id);
