-- A request a friend's app makes for a computer's card thumbnail, not to use the computer.
ALTER TABLE connection_requests ADD COLUMN preview INTEGER NOT NULL DEFAULT 0;
