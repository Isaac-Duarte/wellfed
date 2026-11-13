-- The schema. Until the first release, changes are folded into this file;
-- after it, a released migration is never edited and a change is a new
-- file. Times are Unix milliseconds (UTC); IDs are UUIDv7 bytes.

-- Past `expires_ms` an account is disabled, not deleted.
CREATE TABLE users (
    id BLOB PRIMARY KEY NOT NULL,
    name TEXT NOT NULL UNIQUE,
    password_hash TEXT NOT NULL,
    role TEXT NOT NULL,
    created_ms INTEGER NOT NULL,
    expires_ms INTEGER
) STRICT;

-- A browser session's token is stored hashed, so a copied database holds
-- no live session. A session ends a fixed time after `last_used_ms`.
CREATE TABLE sessions (
    token_hash BLOB PRIMARY KEY NOT NULL,
    user_id BLOB NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    created_ms INTEGER NOT NULL,
    last_used_ms INTEGER NOT NULL
) STRICT;

CREATE INDEX sessions_by_user ON sessions (user_id);

-- Bearer tokens for phones and the CLI. Like a session's, a token is
-- stored as the SHA-256 of its 32 random bytes, so a copied database
-- holds no live token. A token is its user's, not a role's: the role is
-- read from `users` on each use. Deleting the user deletes their tokens;
-- an account past its expiry keeps them, refused until an owner extends
-- it.
CREATE TABLE tokens (
    id BLOB PRIMARY KEY NOT NULL,
    token_hash BLOB NOT NULL UNIQUE,
    user_id BLOB NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    created_ms INTEGER NOT NULL,
    last_used_ms INTEGER,
    expires_ms INTEGER
) STRICT;

CREATE INDEX tokens_by_user ON tokens (user_id);

-- The home the server serves, written with the first owner, which every
-- device belongs to. `zone` is an IANA name. `single` holds a server to one
-- home for now; dropping it lets one serve more.
CREATE TABLE homes (
    id BLOB PRIMARY KEY NOT NULL,
    single INTEGER NOT NULL DEFAULT 1 UNIQUE CHECK (single = 1),
    name TEXT NOT NULL,
    units TEXT NOT NULL,
    time_format TEXT NOT NULL,
    zone TEXT NOT NULL,
    created_ms INTEGER NOT NULL
) STRICT;

-- The home's devices, of every kind: the table keeps its first kind's
-- name. `kind` and `model` are the spellings `wellfed_proto::device`
-- gives them, from the session the server opens at adoption; `kind` is
-- `model`'s, kept beside it so a query can filter by it. The server's own
-- key for each device is sealed with the config file's key, so the
-- database alone cannot impersonate the server. `synced` is the newest
-- event held: the cursor a reconnect subscribes after. It counts in
-- `epoch`, the device's ring epoch, which a new ring draws at random: a
-- ring re-created by a storage wipe counts from 1 again under another
-- epoch. A removed feeder is archived (`archived_ms`), not deleted: its
-- history, samples and journal stay readable, and adopting the same
-- identity again clears this and continues them.
CREATE TABLE feeders (
    id BLOB PRIMARY KEY NOT NULL,
    home_id BLOB NOT NULL REFERENCES homes (id),
    kind TEXT NOT NULL,
    model TEXT NOT NULL,
    identity BLOB NOT NULL UNIQUE,
    name TEXT NOT NULL,
    role TEXT NOT NULL,
    address TEXT,
    key_nonce BLOB NOT NULL,
    key_sealed BLOB NOT NULL,
    synced INTEGER,
    epoch INTEGER,
    created_ms INTEGER NOT NULL,
    archived_ms INTEGER
) STRICT;

-- The last status, settings and feeding plans read, as JSON, and when:
-- served while the feeder is offline, and the next meal is reckoned from
-- the plans meanwhile.
CREATE TABLE feeder_state (
    feeder_id BLOB PRIMARY KEY NOT NULL REFERENCES feeders (id) ON DELETE CASCADE,
    status TEXT,
    status_ms INTEGER,
    settings TEXT,
    settings_ms INTEGER,
    plans TEXT,
    plans_ms INTEGER
) STRICT;

-- Every event as the feeder recorded it: `body` is its postcard bytes,
-- decoded on read. The key makes a replayed event a no-op; event numbers
-- count within an epoch, so the epoch is part of it.
CREATE TABLE events (
    feeder_id BLOB NOT NULL REFERENCES feeders (id) ON DELETE CASCADE,
    epoch INTEGER NOT NULL,
    seq INTEGER NOT NULL,
    at_s INTEGER,
    received_ms INTEGER NOT NULL,
    kind TEXT NOT NULL,
    body BLOB NOT NULL,
    PRIMARY KEY (feeder_id, epoch, seq)
) STRICT, WITHOUT ROWID;

CREATE INDEX events_by_kind ON events (feeder_id, epoch, kind, seq);

-- Events the feeder's ring dropped before the server read them.
CREATE TABLE gaps (
    feeder_id BLOB NOT NULL REFERENCES feeders (id) ON DELETE CASCADE,
    epoch INTEGER NOT NULL,
    from_seq INTEGER NOT NULL,
    to_seq INTEGER NOT NULL,
    received_ms INTEGER NOT NULL,
    PRIMARY KEY (feeder_id, epoch, from_seq)
) STRICT, WITHOUT ROWID;

-- Live samples, raw: `value` is the metric's postcard bytes, and hourly
-- rollups of their numbers outlive the raw rows. `id` is AUTOINCREMENT so
-- an id is never reused, even after retention empties the table: the
-- rollups' mark and the page cursor rely on it. `number` is the one figure
-- the metric is charted by, decoded once at ingest; NULL when the sample
-- has none (no battery reading, no gateway round trip).
CREATE TABLE samples (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    feeder_id BLOB NOT NULL REFERENCES feeders (id) ON DELETE CASCADE,
    received_ms INTEGER NOT NULL,
    uptime_ms INTEGER NOT NULL,
    metric TEXT NOT NULL,
    number INTEGER,
    value BLOB NOT NULL
) STRICT;

CREATE INDEX samples_by_metric ON samples (feeder_id, metric, received_ms);

-- One row per feeder, metric and UTC hour (`hour_ms`, its first
-- millisecond) over the samples that carry a number. The mean is
-- `sum / count`. Kept forever.
CREATE TABLE sample_rollups (
    feeder_id BLOB NOT NULL REFERENCES feeders (id) ON DELETE CASCADE,
    metric TEXT NOT NULL,
    hour_ms INTEGER NOT NULL,
    count INTEGER NOT NULL,
    min INTEGER NOT NULL,
    max INTEGER NOT NULL,
    sum INTEGER NOT NULL,
    PRIMARY KEY (feeder_id, metric, hour_ms)
) STRICT, WITHOUT ROWID;

-- `rolled`: every sample with an id up to it is in the rollups, and none
-- above it is. `version`: the sample-number code the stored numbers were
-- decoded by; at 0 the start's refresh decodes them all.
CREATE TABLE sample_marks (
    id INTEGER PRIMARY KEY CHECK (id = 0),
    rolled INTEGER NOT NULL,
    version INTEGER NOT NULL
) STRICT;

INSERT INTO sample_marks (id, rolled, version) VALUES (0, 0, 0);

-- The command journal: every write the server sends a feeder, recorded
-- before it is sent and kept forever. `key` is the client's
-- `Idempotency-Key` (or one the server drew), so a retry finds its entry
-- whether or not the feeder's ring of 32 write ids still holds it.
-- `body_hash` is the SHA-256 of the request's canonical JSON, so a key
-- reused for another request is refused. `request_id` is the device
-- request id it went out under, and the one a retry resends. `seq` orders
-- the list and is its cursor. `user_id` names no foreign key: the journal
-- outlives the accounts it names.
--
-- For a feed, `history_epoch` and `history_seq` are the newest event the
-- server held of the feeder's history when the write was sent (both null
-- when it held none): a feed whose answer was lost can only have ended in
-- a `Fed` event after it. `feed` is the feeder's number for a feed, from
-- its answer or its `Fed` event, and `commands_by_feed` finds a feed's
-- entry by it.
--
-- `call_before` is the meal call a call write found on the feeder before
-- its first send, as JSON (`none`, or the clip's length and CRC), and
-- `call_started` for an upload whether the feeder took its start: a retry
-- sends again only while neither the feeder nor the upload moved past
-- that, so it never undoes a later change. NULL and 0 for every other
-- write.
CREATE TABLE commands (
    seq INTEGER PRIMARY KEY,
    key BLOB NOT NULL UNIQUE,
    user_id BLOB NOT NULL,
    token_id BLOB,
    feeder_id BLOB NOT NULL REFERENCES feeders (id),
    kind TEXT NOT NULL,
    body_hash BLOB NOT NULL,
    request_id BLOB NOT NULL,
    boot_count INTEGER,
    state TEXT NOT NULL,
    outcome TEXT,
    feed INTEGER,
    sent_ms INTEGER NOT NULL,
    updated_ms INTEGER NOT NULL,
    history_epoch INTEGER,
    history_seq INTEGER,
    call_before TEXT,
    call_started INTEGER NOT NULL DEFAULT 0
) STRICT;

CREATE INDEX commands_by_user ON commands (user_id, seq);
CREATE INDEX commands_by_feeder ON commands (feeder_id, seq);
CREATE INDEX commands_by_state ON commands (state, seq);
CREATE INDEX commands_by_feed ON commands (feeder_id, feed);

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

-- The audit log: who did what through the server. A row is either an
-- account change (`action`, the typed change as JSON) or a write in the
-- command journal (`command_seq`), whose state and outcome are read from
-- the journal, so they follow it until it settles. `seq` orders the log
-- and is its cursor. `user_id` names no foreign key and `user_name` keeps
-- the name it had: the log outlives the accounts it names. Kept forever,
-- like the journal.
CREATE TABLE audit (
    seq INTEGER PRIMARY KEY,
    at_ms INTEGER NOT NULL,
    user_id BLOB,
    user_name TEXT,
    token_id BLOB,
    action TEXT,
    command_seq INTEGER UNIQUE REFERENCES commands (seq) ON DELETE CASCADE,
    CHECK ((action IS NULL) != (command_seq IS NULL))
) STRICT;

CREATE INDEX audit_by_user ON audit (user_id, seq);

-- Every journal entry joins the log as it is recorded, in the same
-- statement, so no write reaches a feeder without its audit row. A row the
-- journal drops (a write that was never sent) leaves the log with it.
CREATE TRIGGER commands_audit AFTER INSERT ON commands
BEGIN
    INSERT INTO audit (at_ms, user_id, user_name, token_id, command_seq)
    VALUES (
        NEW.sent_ms,
        NEW.user_id,
        (SELECT name FROM users WHERE id = NEW.user_id),
        NEW.token_id,
        NEW.seq
    );
END;

-- Projections: typed rows decoded from the raw events, written in the
-- raw insert's transaction so SQL can filter, count and join them. The
-- `events` table stays the truth: every row here can be rebuilt from it,
-- and is when `projection_version` differs from the build's. Each row
-- keeps its event's epoch and number (`seq`), its feeder clock time
-- (`at_s`, absent while that clock was invalid) and when the server
-- received it. `source` is who caused it (`panel`, `client` or `feeder`),
-- with the client's fingerprint in `client` for `client` only.

-- The projection code's version the rows were built by, and the device
-- API revision of the server that last read the events: a body stored
-- unread (`kind` 'unknown', from a feeder newer than that server) is read
-- again once the revision rises. One row.
CREATE TABLE projection_version (
    id INTEGER PRIMARY KEY NOT NULL CHECK (id = 0),
    version INTEGER NOT NULL,
    api_revision INTEGER NOT NULL
) STRICT;

-- One row per feed: the feeder numbers its feeds within an epoch, and a
-- later event for the same feed replaces the row (the higher `seq` wins).
CREATE TABLE feeds (
    feeder_id BLOB NOT NULL REFERENCES feeders (id) ON DELETE CASCADE,
    epoch INTEGER NOT NULL,
    feed INTEGER NOT NULL,
    seq INTEGER NOT NULL,
    source TEXT NOT NULL,
    client BLOB,
    requested INTEGER NOT NULL,
    completed INTEGER NOT NULL,
    outcome TEXT NOT NULL,
    reason TEXT NOT NULL,
    at_s INTEGER,
    received_ms INTEGER NOT NULL,
    PRIMARY KEY (feeder_id, epoch, feed)
) STRICT, WITHOUT ROWID;

CREATE INDEX feeds_by_time ON feeds (feeder_id, received_ms);

-- The panel lock turned on or off.
CREATE TABLE lock_changes (
    feeder_id BLOB NOT NULL REFERENCES feeders (id) ON DELETE CASCADE,
    epoch INTEGER NOT NULL,
    seq INTEGER NOT NULL,
    locked INTEGER NOT NULL,
    source TEXT NOT NULL,
    client BLOB,
    at_s INTEGER,
    received_ms INTEGER NOT NULL,
    PRIMARY KEY (feeder_id, epoch, seq)
) STRICT, WITHOUT ROWID;

CREATE INDEX lock_changes_by_time ON lock_changes (feeder_id, received_ms);

-- Each boot: the bank it ran from and the reset flags behind it.
CREATE TABLE boots (
    feeder_id BLOB NOT NULL REFERENCES feeders (id) ON DELETE CASCADE,
    epoch INTEGER NOT NULL,
    seq INTEGER NOT NULL,
    bank INTEGER NOT NULL,
    reset_flags INTEGER NOT NULL,
    at_s INTEGER,
    received_ms INTEGER NOT NULL,
    PRIMARY KEY (feeder_id, epoch, seq)
) STRICT, WITHOUT ROWID;

CREATE INDEX boots_by_time ON boots (feeder_id, received_ms);

-- The feeder's clock was set: valid from then on. `jump_ms` is how far it
-- moved when it was valid before.
CREATE TABLE clock_syncs (
    feeder_id BLOB NOT NULL REFERENCES feeders (id) ON DELETE CASCADE,
    epoch INTEGER NOT NULL,
    seq INTEGER NOT NULL,
    source TEXT NOT NULL,
    client BLOB,
    jump_ms INTEGER,
    at_s INTEGER,
    received_ms INTEGER NOT NULL,
    PRIMARY KEY (feeder_id, epoch, seq)
) STRICT, WITHOUT ROWID;

CREATE INDEX clock_syncs_by_time ON clock_syncs (feeder_id, received_ms);

-- The lasting reason the feeder cannot feed changed: `reason` is the new
-- one, or absent once it can feed again.
CREATE TABLE blocked_changes (
    feeder_id BLOB NOT NULL REFERENCES feeders (id) ON DELETE CASCADE,
    epoch INTEGER NOT NULL,
    seq INTEGER NOT NULL,
    reason TEXT,
    at_s INTEGER,
    received_ms INTEGER NOT NULL,
    PRIMARY KEY (feeder_id, epoch, seq)
) STRICT, WITHOUT ROWID;

CREATE INDEX blocked_changes_by_time ON blocked_changes (feeder_id, received_ms);

-- What the mirror reads from a feeder beside its history: each feed's
-- trace, and its statistics. Neither is a projection: nothing in the raw
-- events rebuilds them, so a projection rebuild leaves them alone.

-- One feed's trace, fetched after its `Fed` event, keyed as `feeds` is.
-- `summary` is the trace's summary as JSON and `body` its entries in the
-- feeder's own encoding, `stored` whether the feeder had it in storage
-- rather than RAM. A feeder that had no trace for the feed (it was
-- overwritten, or a restart lost it) leaves `refusal`, its answer as JSON,
-- and no trace, so the feed is not asked for again.
CREATE TABLE feed_traces (
    feeder_id BLOB NOT NULL REFERENCES feeders (id) ON DELETE CASCADE,
    epoch INTEGER NOT NULL,
    feed INTEGER NOT NULL,
    fetched_ms INTEGER NOT NULL,
    summary TEXT,
    stored INTEGER,
    body BLOB,
    refusal TEXT,
    CHECK ((summary IS NULL) = (refusal IS NOT NULL)),
    PRIMARY KEY (feeder_id, epoch, feed)
) STRICT, WITHOUT ROWID;

-- The latest read of each statistics section, as JSON. `boot_count` is the
-- feeder's boot it was read in, when known: counters "since this boot"
-- start again at each.
CREATE TABLE feeder_stats (
    feeder_id BLOB NOT NULL REFERENCES feeders (id) ON DELETE CASCADE,
    section TEXT NOT NULL,
    received_ms INTEGER NOT NULL,
    boot_count INTEGER,
    stats TEXT NOT NULL,
    PRIMARY KEY (feeder_id, section)
) STRICT, WITHOUT ROWID;

-- Every read of the food section, typed, for the food views and alerts:
-- the food beam's last reading (`present`, absent while unknown), its
-- low and refill transitions and the feeds since a refill, as the
-- feeder counts them.
CREATE TABLE food_readings (
    feeder_id BLOB NOT NULL REFERENCES feeders (id) ON DELETE CASCADE,
    received_ms INTEGER NOT NULL,
    boot_count INTEGER,
    present INTEGER,
    ran_low INTEGER NOT NULL,
    refilled INTEGER NOT NULL,
    feeds_since_refill INTEGER
) STRICT;

CREATE INDEX food_readings_by_time ON food_readings (feeder_id, received_ms);

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

-- Each background job's last run, for the readiness view and for the
-- schedule: a job due while the server was down runs once when it starts.
CREATE TABLE job_runs (
    job TEXT PRIMARY KEY NOT NULL,
    started_ms INTEGER NOT NULL,
    finished_ms INTEGER NOT NULL,
    outcome TEXT NOT NULL
) STRICT;

-- Owner data the feeders do not hold: pets, the foods they eat, which
-- feeder serves which pet and food, and refills.

-- `species` is its stored spelling; `birth_date` an ISO date.
CREATE TABLE pets (
    id BLOB PRIMARY KEY NOT NULL,
    name TEXT NOT NULL,
    species TEXT NOT NULL,
    birth_date TEXT,
    daily_kcal INTEGER,
    created_ms INTEGER NOT NULL
) STRICT;

-- The weight log. An entry may be backdated (a vet's reading), so the log
-- is ordered by `at_ms`, with the id breaking ties.
CREATE TABLE pet_weights (
    id BLOB PRIMARY KEY NOT NULL,
    pet_id BLOB NOT NULL REFERENCES pets (id) ON DELETE CASCADE,
    at_ms INTEGER NOT NULL,
    grams INTEGER NOT NULL
) STRICT;

CREATE INDEX pet_weights_by_time ON pet_weights (pet_id, at_ms, id);

-- Pets and feeders are many-to-many, and the links keep their history: a
-- feeder's intake at a time is split among the pets it served then. A link
-- runs from `from_ms` until `to_ms`, open while that is NULL. A pair has at
-- most one open link, and a new one never starts before the pair's last
-- one ended, so a pair's links never overlap.
CREATE TABLE pet_feeders (
    seq INTEGER PRIMARY KEY,
    pet_id BLOB NOT NULL REFERENCES pets (id) ON DELETE CASCADE,
    feeder_id BLOB NOT NULL REFERENCES feeders (id) ON DELETE CASCADE,
    from_ms INTEGER NOT NULL,
    to_ms INTEGER,
    CHECK (to_ms IS NULL OR to_ms >= from_ms)
) STRICT;

CREATE UNIQUE INDEX pet_feeders_open ON pet_feeders (pet_id, feeder_id) WHERE to_ms IS NULL;
CREATE INDEX pet_feeders_by_feeder ON pet_feeders (feeder_id, from_ms);

-- Energy is kcal per kg, as labels print it. A portion's weight is the
-- feeder's own calibration, never the food's.
CREATE TABLE foods (
    id BLOB PRIMARY KEY NOT NULL,
    brand TEXT NOT NULL,
    name TEXT NOT NULL,
    kcal_per_kg INTEGER NOT NULL,
    bag_grams INTEGER NOT NULL,
    created_ms INTEGER NOT NULL
) STRICT;

-- Each feeder's food from `from_ms` on, until its next row. `from_ms`
-- never decreases along `seq` (a change is never dated before the one it
-- follows), so the food at a time is the last row that started by then.
-- A food named here cannot be deleted: past feeds' calories use it.
CREATE TABLE feeder_foods (
    seq INTEGER PRIMARY KEY,
    feeder_id BLOB NOT NULL REFERENCES feeders (id) ON DELETE CASCADE,
    food_id BLOB NOT NULL REFERENCES foods (id),
    from_ms INTEGER NOT NULL
) STRICT;

CREATE INDEX feeder_foods_by_feeder ON feeder_foods (feeder_id, seq);

-- `grams` is what went in: a full bag's is the food's bag size when it was
-- recorded. `food_id` is the feeder's food then, if it had one. `user_id`
-- names no foreign key: a refill outlives the account that recorded it.
CREATE TABLE refills (
    seq INTEGER PRIMARY KEY,
    id BLOB NOT NULL UNIQUE,
    feeder_id BLOB NOT NULL REFERENCES feeders (id) ON DELETE CASCADE,
    at_ms INTEGER NOT NULL,
    grams INTEGER NOT NULL,
    full_bag INTEGER NOT NULL,
    food_id BLOB REFERENCES foods (id),
    user_id BLOB NOT NULL
) STRICT;

CREATE INDEX refills_by_feeder ON refills (feeder_id, seq);

-- Installs: an owner's request to install a release, or to restore the
-- feeder's stock firmware, on one feeder, kept as it progresses and once
-- it ends. At most one row is `running`: the service runs one install at
-- a time, and every start marks a row a stopped server left running as
-- failed. `failure` is the API's `InstallFailure` as JSON. `user_id` names
-- no foreign key: the history outlives the accounts it names.
--
-- `target` names what an install writes (`release` or `stock`), and
-- `release_id` holds that target's name: the release's id, or the SHA-256
-- of the stock backup the server keeps in its data directory (never in
-- the database). An install of a release is followed through its trial,
-- through `confirming` to `confirmed`, or fails `rolled_back` or
-- `wrong_build`.
--
-- `offered` is whether the feeder took the install's offer: once it has,
-- bank 1 may be partly written, or activated with its answer lost, so an
-- install a stopped server left running is no longer known to have
-- changed nothing. A start marks such a row failed `unreachable` with
-- `offered`, the uncertain end, and only a row whose offer never took
-- `interrupted`.
CREATE TABLE installs (
    id BLOB PRIMARY KEY NOT NULL,
    feeder_id BLOB NOT NULL REFERENCES feeders (id) ON DELETE CASCADE,
    release_id BLOB NOT NULL,
    version INTEGER NOT NULL,
    user_id BLOB NOT NULL,
    started_ms INTEGER NOT NULL,
    state TEXT NOT NULL,
    written INTEGER NOT NULL,
    image_len INTEGER NOT NULL,
    failure TEXT,
    finished_ms INTEGER,
    target TEXT NOT NULL DEFAULT 'release',
    offered INTEGER NOT NULL DEFAULT 0
) STRICT;

CREATE INDEX installs_by_feeder ON installs (feeder_id, started_ms);
CREATE INDEX installs_by_state ON installs (state);

-- Each phone key the server made an invite for, recorded before the invite
-- is asked for, so a phone that joins with it is attributed. The feeder
-- does not bind an invite to a key, so until `invite_until_ms` (the last
-- invite's deadline, or a bound while its answer is unknown) the key may
-- still join: a revoke answered `not_found` before then is not settled.
-- `user_id` and `token_id` carry no foreign key: a deleted account's keys
-- must still be revoked. `revoke` is why the key is being revoked (null
-- while it stands); the row goes once the feeder has dropped the key.
-- `attempt` is the last attempt's outcome, as JSON.
--
-- `invited_by` is set for a phone key that joined through another phone
-- key's invite: the first 7 bytes of the key that invited it, whose
-- account and token it is recorded under. Revoking that key, however it
-- is revoked, revokes it. `last_invitation` is the newest `invitations` id
-- when the key was recorded: only a join recorded after it (a greater id)
-- can be this key's. Insertion order, not a clock, so a server clock
-- stepped back never drops a real join, and a phone claimed later never
-- takes the joins it made before.
CREATE TABLE phone_keys (
    feeder_id BLOB NOT NULL REFERENCES feeders (id),
    key BLOB NOT NULL,
    user_id BLOB NOT NULL,
    token_id BLOB,
    role TEXT NOT NULL,
    invited_ms INTEGER NOT NULL,
    invite_until_ms INTEGER NOT NULL,
    revoke TEXT,
    revoke_ms INTEGER,
    attempt TEXT,
    invited_by BLOB,
    last_invitation INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (feeder_id, key)
) STRICT;

CREATE INDEX phone_keys_by_user ON phone_keys (user_id);
CREATE INDEX phone_keys_by_token ON phone_keys (token_id);

-- Clients the server did not invite that an owner chose to keep.
CREATE TABLE kept_clients (
    feeder_id BLOB NOT NULL REFERENCES feeders (id),
    key BLOB NOT NULL,
    kept_ms INTEGER NOT NULL,
    PRIMARY KEY (feeder_id, key)
) STRICT;

-- Each join through an invite a feeder reported (an `invited` event):
-- the first 7 bytes of the key that joined and of the key that asked for
-- the invite. Written with the event, in its transaction, and kept until
-- the server has attributed the key or found it no phone's: `settled_ms`
-- is when. Not a projection: a rebuild of the projections leaves it, so
-- a join is never attributed twice.
-- `id` orders them as the server recorded them, never reused, and is
-- exported with each, so a restore keeps the order.
CREATE TABLE invitations (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    feeder_id BLOB NOT NULL REFERENCES feeders (id),
    epoch INTEGER NOT NULL,
    seq INTEGER NOT NULL,
    invitee BLOB NOT NULL,
    inviter BLOB NOT NULL,
    received_ms INTEGER NOT NULL,
    settled_ms INTEGER,
    UNIQUE (feeder_id, epoch, seq)
) STRICT;

-- Phone keys the feeder dropped, by their first 7 bytes, so a join
-- through one's invite that the server reads only after the key was
-- dropped is still revoked with it. `last_invitation` is the key's own, as
-- in `phone_keys`: a join recorded before the key was is not its.
CREATE TABLE dropped_phones (
    feeder_id BLOB NOT NULL REFERENCES feeders (id),
    prefix BLOB NOT NULL,
    user_id BLOB NOT NULL,
    token_id BLOB,
    last_invitation INTEGER NOT NULL,
    dropped_ms INTEGER NOT NULL,
    PRIMARY KEY (feeder_id, prefix)
) STRICT;
