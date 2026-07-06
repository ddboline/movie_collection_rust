ALTER TABLE trakt_watchlist ADD COLUMN slug TEXT;
ALTER TABLE trakt_watchlist ADD COLUMN imdb_link TEXT;
ALTER TABLE trakt_watched_movies ADD COLUMN slug TEXT;
ALTER TABLE trakt_watched_movies ADD COLUMN imdb_link TEXT;
ALTER TABLE trakt_watched_episodes ADD COLUMN title TEXT;
ALTER TABLE trakt_watched_episodes ADD COLUMN imdb_link TEXT;

CREATE TABLE IF NOT EXISTS trakt_watched_shows (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    title TEXT NOT NULL,
    link TEXT NOT NULL,
    slug TEXT NOT NULL,
    last_watched_at TIMESTAMP WITH TIME ZONE NOT NULL,
    show TEXT,
    imdb_link TEXT
);