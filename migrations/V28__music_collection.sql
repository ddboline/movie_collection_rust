CREATE TABLE IF NOT EXISTS music_collection (
    id INTEGER NOT NULL PRIMARY KEY,
    path TEXT UNIQUE NOT NULL,
    artist TEXT,
    album TEXT,
    title TEXT,
    last_modified timestamp with time zone
);