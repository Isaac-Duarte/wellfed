-- An install of a release is followed through its trial: `state` gains
-- `confirming` (the trial followed) and `confirmed`, and `failure`
-- `rolled_back` and `wrong_build`. Nothing in the schema changes; the
-- migration is there so that an older server, which cannot read those
-- codes, opens a database holding them read-only.
SELECT 1;
