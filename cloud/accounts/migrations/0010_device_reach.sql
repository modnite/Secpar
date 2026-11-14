-- Where a hosting device said it can be reached: its tunnel port and the
-- addresses it has on its own network, as JSON, and when it last said so.
-- Only the account's own devices are given this.
ALTER TABLE devices ADD COLUMN reach TEXT;
ALTER TABLE devices ADD COLUMN reach_seen INTEGER NOT NULL DEFAULT 0;
