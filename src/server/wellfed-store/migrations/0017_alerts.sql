-- Each feeder's alerts as the server keeps them: the table it last read
-- and what it has made of the tables since (the alerts it holds open, the
-- feeder's reach, and what the webhook was told), as one JSON document
-- the mirror writes and versions by `format`. `version` counts the row's
-- writes, so two writers never overwrite each other unseen: each writes
-- over the version it read, and reads again when that moved.
CREATE TABLE feeder_alerts (
    feeder_id BLOB PRIMARY KEY NOT NULL REFERENCES feeders (id) ON DELETE CASCADE,
    format INTEGER NOT NULL,
    state TEXT NOT NULL,
    version INTEGER NOT NULL,
    updated_ms INTEGER NOT NULL
) STRICT;
