-- A device that cannot reach a hosting device directly leaves its relay address
-- here; the hosting device picks it up, answers with its own, and both then meet
-- through the relay. Rows live for seconds.
CREATE TABLE knocks(
  host TEXT NOT NULL,
  device TEXT NOT NULL,
  candidates TEXT NOT NULL,
  answer TEXT,
  taken INTEGER NOT NULL DEFAULT 0,
  created INTEGER NOT NULL,
  PRIMARY KEY(host, device)
);
