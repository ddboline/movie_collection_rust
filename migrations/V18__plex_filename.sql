CREATE TABLE plex_filename (
    metadata_key TEXT PRIMARY KEY,
    filename TEXT NOT NULL,
    created_at TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT now(),
    last_modified TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT now()
);
