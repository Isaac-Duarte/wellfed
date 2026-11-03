-- The meal call a call write found on the feeder before its first send,
-- as JSON (`none`, or the clip's length and CRC): a retry sends again only
-- while the feeder still holds it, so it never undoes a later change.
-- NULL for every other write, and for one recorded before it was read.
ALTER TABLE commands ADD COLUMN call_before TEXT;
