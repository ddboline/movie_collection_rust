CREATE SEQUENCE imdb_ratings_id_seq;

CREATE TABLE IF NOT EXISTS imdb_ratings (
    index INTEGER NOT NULL PRIMARY KEY DEFAULT nextval('imdb_ratings_id_seq'::regclass),
    show text NOT NULL UNIQUE,
    title text,
    link text,
    rating double precision,
    istv bool,
    source text,
    last_modified timestamp with time zone
);
