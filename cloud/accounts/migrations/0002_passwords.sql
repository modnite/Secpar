CREATE TABLE password_accounts(
  user_id TEXT PRIMARY KEY REFERENCES accounts(id),
  email TEXT NOT NULL UNIQUE,
  password_hash TEXT NOT NULL,
  verified INTEGER NOT NULL DEFAULT 0,
  created INTEGER NOT NULL
);
CREATE TABLE email_tokens(
  hash TEXT PRIMARY KEY,
  user_id TEXT NOT NULL REFERENCES accounts(id),
  purpose TEXT NOT NULL CHECK(purpose IN ('verify','reset')),
  expires INTEGER NOT NULL
);
CREATE INDEX email_tokens_expiry ON email_tokens(expires);
