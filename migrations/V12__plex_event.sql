CREATE SEQUENCE IF NOT EXISTS plex_event_id_seq;

CREATE TABLE plex_event (
    id INTEGER NOT NULL PRIMARY KEY DEFAULT nextval('plex_event_id_seq'::regclass),
    event TEXT NOT NULL,
    account TEXT NOT NULL,
    server TEXT NOT NULL,
    player_title TEXT NOT NULL,
    title TEXT,
    parent_title TEXT,
    grandparent_title TEXT,
    added_at TIMESTAMP WITH TIME ZONE,
    updated_at TIMESTAMP WITH TIME ZONE,
    created_at TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT now()
);
