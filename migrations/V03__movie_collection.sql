CREATE TABLE movie_collection (
    idx INTEGER NOT NULL PRIMARY KEY,
    path TEXT UNIQUE,
    show TEXT,
    show_id INTEGER REFERENCES imdb_ratings (index),
    last_modified timestamp with time zone
);
