-- Keyboard and mouse can be allowed apart. NULL means follow the single input setting, so rows made
-- before this keep their meaning and older clients and hosts that only read input still work.
ALTER TABLE friend_settings ADD COLUMN keyboard INTEGER CHECK (keyboard IN (0,1));
ALTER TABLE friend_settings ADD COLUMN mouse INTEGER CHECK (mouse IN (0,1));
