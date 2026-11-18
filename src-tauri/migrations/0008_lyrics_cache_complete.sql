-- 0008: lyrics lookups where a source failed are retried sooner

ALTER TABLE lyrics_cache ADD COLUMN complete INTEGER NOT NULL DEFAULT 1;
