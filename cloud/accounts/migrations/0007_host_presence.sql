-- What a host tells the service so a friend can find it. Both are optional and only set while
-- the owner has friends switched on. pc is the host's rendezvous id, relay its Worker origin.
ALTER TABLE host_keys ADD COLUMN pc TEXT;
ALTER TABLE host_keys ADD COLUMN relay TEXT;
