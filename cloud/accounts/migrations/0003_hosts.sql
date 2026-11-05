CREATE TABLE host_challenges(hash TEXT PRIMARY KEY, owner TEXT NOT NULL REFERENCES accounts(id), expires INTEGER NOT NULL);
CREATE INDEX host_challenges_expiry ON host_challenges(expires);
CREATE TABLE host_keys(
  computer TEXT PRIMARY KEY REFERENCES computers(id),
  thumbprint TEXT NOT NULL UNIQUE,
  certificate_fp TEXT NOT NULL,
  credential_hash TEXT NOT NULL UNIQUE,
  last_seen INTEGER NOT NULL DEFAULT 0,
  sharing INTEGER NOT NULL DEFAULT 0 CHECK(sharing IN (0,1))
);
