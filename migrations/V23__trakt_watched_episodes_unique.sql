DELETE FROM trakt_watched_episodes
WHERE id NOT IN (
    SELECT DISTINCT ON (link, season, episode) id
    FROM trakt_watched_episodes
    ORDER BY link, season, episode, id
);

ALTER TABLE trakt_watched_episodes ADD UNIQUE (link, season, episode);