CREATE TABLE plex_metadata (
    metadata_key TEXT PRIMARY KEY,
    object_type TEXT NOT NULL,
    title TEXT NOT NULL,
    parent_key TEXT,
    grandparent_key TEXT,
    show TEXT,
    last_modified TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT now(),
    created_at TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT now()
);