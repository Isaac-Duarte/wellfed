-- An Idempotency-Key a client read the journal for and found no write
-- under (`GET /commands/{key}` answered 404), with when. A client takes
-- that answer as never sent, so a write under the key that reaches the
-- journal later, after a stall in front of it, is refused rather than run:
-- it could feed after the owner fed again. Kept a week, which no request
-- outlasts, then dropped by the expiry job.
CREATE TABLE retired_keys (
    key BLOB PRIMARY KEY NOT NULL,
    retired_ms INTEGER NOT NULL
);
