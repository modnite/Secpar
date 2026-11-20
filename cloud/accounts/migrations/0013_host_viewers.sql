-- Who is using a computer now, as its host last reported (a JSON list of short names).
ALTER TABLE host_keys ADD COLUMN viewers TEXT;
