-- A phone key that joined through another phone key's invite: the first
-- 7 bytes of the key that invited it, whose account and token it is
-- recorded under. Revoking that key, however it is revoked, revokes it.
ALTER TABLE phone_keys ADD COLUMN invited_by BLOB;

-- Each join through an invite a feeder reported (an `invited` event):
-- the first 7 bytes of the key that joined and of the key that asked for
-- the invite. Written with the event, in its transaction, and kept until
-- the server has attributed the key or found it no phone's: `settled_ms`
-- is when. Not a projection: a rebuild of the projections leaves it, so
-- a join is never attributed twice.
CREATE TABLE invitations (
    feeder_id BLOB NOT NULL REFERENCES feeders (id),
    epoch INTEGER NOT NULL,
    seq INTEGER NOT NULL,
    invitee BLOB NOT NULL,
    inviter BLOB NOT NULL,
    received_ms INTEGER NOT NULL,
    settled_ms INTEGER,
    PRIMARY KEY (feeder_id, epoch, seq)
) STRICT;

-- Phone keys the feeder dropped, so a join through one's invite that the
-- server reads only after the key was dropped is still revoked with it.
CREATE TABLE dropped_phones (
    feeder_id BLOB NOT NULL REFERENCES feeders (id),
    key BLOB NOT NULL,
    user_id BLOB NOT NULL,
    token_id BLOB,
    dropped_ms INTEGER NOT NULL,
    PRIMARY KEY (feeder_id, key)
) STRICT;
