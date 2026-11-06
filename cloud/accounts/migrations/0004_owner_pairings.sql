CREATE TABLE owner_pairings (
  id TEXT PRIMARY KEY,
  computer TEXT NOT NULL REFERENCES computers(id),
  owner TEXT NOT NULL REFERENCES accounts(id),
  requester_hash TEXT NOT NULL,
  host_key_hash TEXT NOT NULL,
  certificate_fp TEXT NOT NULL,
  device_name TEXT NOT NULL,
  expires INTEGER NOT NULL,
  sealed TEXT
);
CREATE INDEX owner_pairings_host ON owner_pairings(computer, expires);
CREATE INDEX owner_pairings_owner ON owner_pairings(owner, expires);
