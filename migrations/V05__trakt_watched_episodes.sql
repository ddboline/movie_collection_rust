CREATE SEQUENCE trakt_watched_episodes_id_seq;

CREATE TABLE trakt_watched_episodes (
    id INTEGER NOT NULL PRIMARY KEY DEFAULT nextval('trakt_watched_episodes_id_seq'::regclass),
    link text not null,
    season INTEGER,
    episode INTEGER
);
