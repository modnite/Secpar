-- Usernames are unique without regard to case.
-- handle_lower stays nullable so a Worker from before this change can still insert rows
-- during the deploy window. The new Worker always sets it.
ALTER TABLE accounts ADD COLUMN handle_lower TEXT;

-- Bring old handles into the username charset: letters, digits and underscore, 3 to 30 characters.
UPDATE accounts SET handle = replace(substr(handle, 1, 30), '-', '_');
UPDATE accounts SET handle = handle || substr('___', 1, 3 - length(handle)) WHERE length(handle) < 3;
UPDATE accounts SET handle_lower = lower(handle);

-- The oldest account keeps a shared name. Later ones become name_2, name_3 and so on.
-- If that name is already taken, the account gets the first eight characters of its id instead.
CREATE TABLE handle_fix AS
  SELECT id, handle_lower AS base,
         ROW_NUMBER() OVER (PARTITION BY handle_lower ORDER BY rowid) AS rn
  FROM accounts;
ALTER TABLE handle_fix ADD COLUMN cand TEXT;
UPDATE handle_fix
  SET cand = substr(base, 1, 30 - length('_' || rn)) || '_' || rn
  WHERE rn > 1;
UPDATE handle_fix
  SET cand = substr(base, 1, 21) || '_' || substr(replace(id, '-', ''), 1, 8)
  WHERE rn > 1
    AND (cand IN (SELECT base FROM handle_fix WHERE rn = 1)
         OR (SELECT count(*) FROM handle_fix h WHERE h.cand = handle_fix.cand) > 1);
UPDATE accounts
  SET handle = (SELECT cand FROM handle_fix WHERE handle_fix.id = accounts.id),
      handle_lower = (SELECT cand FROM handle_fix WHERE handle_fix.id = accounts.id)
  WHERE id IN (SELECT id FROM handle_fix WHERE rn > 1);
DROP TABLE handle_fix;

CREATE UNIQUE INDEX accounts_handle_lower ON accounts(handle_lower);
