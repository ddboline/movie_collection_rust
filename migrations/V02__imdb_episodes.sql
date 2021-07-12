CREATE SEQUENCE IF NOT EXISTS imdb_episodes_id_seq;

CREATE TABLE IF NOT EXISTS imdb_episodes (
    id INTEGER NOT NULL PRIMARY KEY DEFAULT nextval('imdb_episodes_id_seq'::regclass),
    show text NOT NULL REFERENCES imdb_ratings (show),
    season INTEGER,
    episode INTEGER,
    epurl text,
    airdate date,
    rating numeric(3, 1),
    eptitle text,
    last_modified timestamp with time zone
);
