-- Blog comments. Anonymous; nothing shows until the owner approves it.
-- sender is an HMAC of the poster's IP address with a server secret, kept only for
-- rate limits and "remove everything from this sender"; the address itself is never stored.
CREATE TABLE comments (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  post TEXT NOT NULL,
  name TEXT NOT NULL,
  body TEXT NOT NULL,
  created INTEGER NOT NULL,
  approved INTEGER NOT NULL DEFAULT 0,
  sender TEXT NOT NULL
);
CREATE INDEX comments_post ON comments(post, approved, created);
CREATE INDEX comments_sender ON comments(sender, created);
CREATE INDEX comments_queue ON comments(approved, created);
