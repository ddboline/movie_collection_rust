CREATE EXTENSION IF NOT EXISTS pgcrypto;

ALTER TABLE plex_filename DROP CONSTRAINT IF EXISTS plex_filename_collection_id_fkey;

CREATE TABLE IF NOT EXISTS music_collection (
    id UUID NOT NULL PRIMARY KEY DEFAULT gen_random_uuid(),
    path TEXT UNIQUE NOT NULL,
    artist TEXT,
    album TEXT,
    title TEXT,
    last_modified timestamp with time zone
);

ALTER TABLE plex_filename ADD COLUMN music_collection_id UUID;
