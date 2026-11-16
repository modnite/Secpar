-- Whether a friend may browse this owner's files, not only send to them.
ALTER TABLE friend_settings ADD COLUMN browse INTEGER NOT NULL DEFAULT 0 CHECK (browse IN (0,1));
