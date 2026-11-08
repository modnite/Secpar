-- Visibility in the user directory. Accounts stay hidden until the owner turns this on.
ALTER TABLE accounts ADD COLUMN visible INTEGER NOT NULL DEFAULT 0;
CREATE INDEX accounts_visible ON accounts(handle_lower) WHERE visible=1;

-- What a host owner allows each friend. A missing row means the defaults: view only, ask each time.
CREATE TABLE friend_settings (
  owner TEXT NOT NULL REFERENCES accounts(id),
  friend TEXT NOT NULL REFERENCES accounts(id),
  input TEXT NOT NULL DEFAULT 'view' CHECK (input IN ('view','full')),
  connect TEXT NOT NULL DEFAULT 'ask' CHECK (connect IN ('ask','always')),
  PRIMARY KEY (owner, friend)
);
