ALTER TABLE plex_filename ADD COLUMN collection_id INTEGER REFERENCES movie_collection (idx);
