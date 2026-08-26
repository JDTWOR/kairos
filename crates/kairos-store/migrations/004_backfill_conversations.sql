INSERT INTO conversations (id, title, repo, session_id, created_at, updated_at)
SELECT
    lower(hex(randomblob(16))),
    'Imported conversation',
    repo,
    lower(hex(randomblob(16))),
    MIN(created_at),
    MAX(updated_at)
FROM tasks
WHERE conversation_id IS NULL
GROUP BY repo;

UPDATE tasks
SET conversation_id = (
    SELECT conversations.id
    FROM conversations
    WHERE conversations.repo = tasks.repo
    ORDER BY conversations.updated_at DESC
    LIMIT 1
)
WHERE conversation_id IS NULL;

INSERT INTO messages (id, conversation_id, role, content, created_at)
SELECT
    lower(hex(randomblob(16))),
    conversation_id,
    'user',
    title,
    created_at
FROM tasks
WHERE conversation_id IS NOT NULL
  AND NOT EXISTS (
      SELECT 1 FROM messages
      WHERE messages.conversation_id = tasks.conversation_id
        AND messages.content = tasks.title
  );
