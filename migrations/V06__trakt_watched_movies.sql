CREATE SEQUENCE IF NOT EXISTS trakt_watched_movies_id_seq;

CREATE TABLE IF NOT EXISTS trakt_watched_movies (
    id INTEGER NOT NULL PRIMARY KEY DEFAULT nextval('trakt_watched_movies_id_seq'::regclass),
    link text not null unique
);
