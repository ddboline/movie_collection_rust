ALTER TABLE plex_event ALTER COLUMN title SET NOT NULL;
ALTER TABLE plex_event ALTER COLUMN added_at SET NOT NULL;
ALTER TABLE plex_event DROP COLUMN created_at;
ALTER TABLE plex_event ALTER COLUMN player_address SET NOT NULL;
