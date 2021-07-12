CREATE SEQUENCE IF NOT EXISTS movie_collection_on_dvd_id_seq;

CREATE TABLE IF NOT EXISTS movie_collection_on_dvd (
    id INTEGER NOT NULL PRIMARY KEY DEFAULT nextval('movie_collection_on_dvd_id_seq'::regclass),
    path text NOT NULL,
    file_size INTEGER
);
