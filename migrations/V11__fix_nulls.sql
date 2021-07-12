ALTER TABLE imdb_ratings ALTER COLUMN title SET NOT NULL;
ALTER TABLE imdb_episodes ALTER COLUMN season SET NOT NULL;
ALTER TABLE imdb_episodes ALTER COLUMN episode SET NOT NULL;
ALTER TABLE imdb_episodes ALTER COLUMN epurl SET NOT NULL;
ALTER TABLE imdb_episodes ALTER COLUMN rating SET NOT NULL;
ALTER TABLE imdb_episodes ALTER COLUMN eptitle SET NOT NULL;
ALTER TABLE imdb_episodes ALTER COLUMN last_modified SET NOT NULL;
ALTER TABLE movie_collection ALTER COLUMN "path" SET NOT NULL;
ALTER TABLE movie_collection ALTER COLUMN show SET NOT NULL;
ALTER TABLE movie_collection ALTER COLUMN last_modified SET NOT NULL;
ALTER TABLE movie_collection ALTER COLUMN last_modified SET DEFAULT now();
ALTER TABLE movie_queue ALTER COLUMN last_modified SET NOT NULL;
ALTER TABLE movie_queue ALTER COLUMN last_modified SET DEFAULT now();
ALTER TABLE trakt_watchlist ALTER COLUMN title SET NOT NULL;
ALTER TABLE trakt_watchlist ALTER COLUMN year SET NOT NULL;
