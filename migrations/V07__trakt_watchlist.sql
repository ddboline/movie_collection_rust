CREATE SEQUENCE IF NOT EXISTS trakt_watchlist_id_seq;

CREATE TABLE IF NOT EXISTS trakt_watchlist (
    id INTEGER NOT NULL PRIMARY KEY DEFAULT nextval('trakt_watchlist_id_seq'::regclass),
    link text not null,
    title text,
    year INTEGER
);
