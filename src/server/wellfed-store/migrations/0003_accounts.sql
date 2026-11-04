-- Accounts that expire, and sessions that last from their last use.

-- Past `expires_ms` an account is disabled, not deleted.
ALTER TABLE users ADD COLUMN expires_ms INTEGER;

-- A session ends a fixed time after `last_used_ms`. Sessions held nothing
-- worth keeping across this change, so the table is made again.
DROP TABLE sessions;
CREATE TABLE sessions (
    token_hash BLOB PRIMARY KEY NOT NULL,
    user_id BLOB NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    created_ms INTEGER NOT NULL,
    last_used_ms INTEGER NOT NULL
) STRICT;

CREATE INDEX sessions_by_user ON sessions (user_id);

