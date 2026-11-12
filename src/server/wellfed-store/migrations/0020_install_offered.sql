-- Whether the feeder took the install's offer: once it has, bank 1 may be
-- partly written, or activated with its answer lost, so an install a
-- stopped server left running is no longer known to have changed nothing.
-- A start marks such a row failed `unreachable` with `offered`, the
-- uncertain end, and only a row whose offer never took `interrupted`.
ALTER TABLE installs ADD COLUMN offered INTEGER NOT NULL DEFAULT 0;
