-- Secpar 4: one row per installation, known by the SHA-256 of its certificate.
-- The same certificate enrolled again updates its row, so reinstalling the app or
-- signing in again never creates a second device.
CREATE TABLE devices (
  id TEXT PRIMARY KEY,
  owner TEXT NOT NULL REFERENCES accounts(id),
  fingerprint TEXT NOT NULL UNIQUE,
  name TEXT NOT NULL,
  platform TEXT NOT NULL CHECK (platform IN ('windows','linux','android')),
  hosts INTEGER NOT NULL DEFAULT 0 CHECK (hosts IN (0,1)),
  created INTEGER NOT NULL,
  last_seen INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX devices_owner ON devices(owner);
CREATE TABLE device_challenges (hash TEXT PRIMARY KEY, owner TEXT NOT NULL REFERENCES accounts(id), expires INTEGER NOT NULL);
CREATE INDEX device_challenges_expiry ON device_challenges(expires);
