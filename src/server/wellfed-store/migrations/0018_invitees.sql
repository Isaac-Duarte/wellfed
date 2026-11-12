-- A phone key that joined through another phone key's invite: the first
-- 7 bytes of the key that invited it, whose account and token it is
-- recorded under. Revoking that key, however it is revoked, revokes it.
ALTER TABLE phone_keys ADD COLUMN invited_by BLOB;

-- The newest `invitations` id when the key was recorded: only a join
-- recorded after it (a greater id) can be this key's. Insertion order, not
-- a clock, so a server clock stepped back never drops a real join, and a
-- phone claimed later never takes the joins it made before. A key recorded
-- before this table existed came before every join in it.
ALTER TABLE phone_keys ADD COLUMN last_invitation INTEGER NOT NULL DEFAULT 0;

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
