CREATE TABLE IF NOT EXISTS movie_queue (
    idx INTEGER NOT NULL PRIMARY KEY,
    collection_idx INTEGER NOT NULL REFERENCES movie_collection (idx),
    last_modified timestamp with time zone
);
