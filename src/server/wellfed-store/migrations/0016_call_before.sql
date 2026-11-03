-- The meal call a call write found on the feeder before its first send,
-- as JSON (`none`, or the clip's length and CRC), and for an upload
-- whether the feeder took its start: a retry sends again only while
-- neither the feeder nor the upload moved past that, so it never undoes
-- a later change. NULL and 0 for every other write.
ALTER TABLE commands ADD COLUMN call_before TEXT;
ALTER TABLE commands ADD COLUMN call_started INTEGER NOT NULL DEFAULT 0;
