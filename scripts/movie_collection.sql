CREATE TABLE IF NOT EXISTS movie_collection (
    idx INTEGER NOT NULL PRIMARY KEY,
    path TEXT UNIQUE,
    show TEXT,
    show_id INTEGER REFERENCES imdb_ratings (id)
);