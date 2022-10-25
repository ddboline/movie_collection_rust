ALTER TABLE movie_collection DROP COLUMN show_id;

ALTER TABLE imdb_ratings DROP COLUMN index;
ALTER TABLE imdb_ratings ADD COLUMN index UUID PRIMARY KEY DEFAULT gen_random_uuid();
ALTER TABLE movie_collection ADD COLUMN show_id UUID REFERENCES imdb_ratings (index);
UPDATE movie_collection
SET show_id = (
    SELECT index
    FROM imdb_ratings
    WHERE show = movie_collection.show
) WHERE show_id IS NULL;

ALTER TABLE imdb_episodes DROP COLUMN id;
ALTER TABLE imdb_episodes ADD COLUMN id UUID PRIMARY KEY DEFAULT gen_random_uuid();

ALTER TABLE trakt_watched_episodes DROP COLUMN id;
ALTER TABLE trakt_watched_episodes ADD COLUMN id UUID PRIMARY KEY DEFAULT gen_random_uuid();

ALTER TABLE trakt_watched_movies DROP COLUMN id;
ALTER TABLE trakt_watched_movies ADD COLUMN id UUID PRIMARY KEY DEFAULT gen_random_uuid();

ALTER TABLE trakt_watchlist DROP COLUMN id;
ALTER TABLE trakt_watchlist ADD COLUMN id UUID PRIMARY KEY DEFAULT gen_random_uuid();

ALTER TABLE movie_collection ADD COLUMN idx_temp UUID NOT NULL DEFAULT gen_random_uuid();
ALTER TABLE movie_collection DROP CONSTRAINT movie_collection_show_id_fkey;
ALTER TABLE movie_queue DROP CONSTRAINT movie_queue_collection_idx_fkey;
ALTER TABLE movie_queue ADD COLUMN collection_idx_temp UUID;
UPDATE movie_queue
SET collection_idx_temp = (
    SELECT idx_temp
    FROM movie_collection
    WHERE idx = movie_queue.collection_idx
)
WHERE collection_idx IS NOT NULL;
ALTER TABLE plex_filename ADD COLUMN collection_id_temp UUID;
UPDATE plex_filename
SET collection_id_temp = (
    SELECT idx_temp
    FROM movie_collection
    WHERE idx = plex_filename.collection_id
)
WHERE collection_id IS NOT NULL;

ALTER TABLE movie_collection DROP COLUMN idx;
ALTER TABLE movie_collection ADD COLUMN idx UUID PRIMARY KEY DEFAULT gen_random_uuid();
UPDATE movie_collection SET idx=idx_temp;
ALTER TABLE movie_collection DROP COLUMN idx_temp;

ALTER TABLE movie_queue DROP COLUMN collection_idx;
ALTER TABLE movie_queue ADD COLUMN collection_idx UUID REFERENCES movie_collection (idx);
UPDATE movie_queue SET collection_idx=collection_idx_temp;
ALTER TABLE movie_queue DROP COLUMN collection_idx_temp;
ALTER TABLE movie_queue ADD PRIMARY KEY (collection_idx);
ALTER TABLE movie_queue ADD UNIQUE (collection_idx);

ALTER TABLE plex_filename DROP COLUMN collection_id;
ALTER TABLE plex_filename ADD COLUMN collection_id UUID;
UPDATE plex_filename SET collection_id=collection_id_temp;
ALTER TABLE plex_filename DROP COLUMN collection_id_temp;

ALTER TABLE plex_event DROP COLUMN id;
ALTER TABLE plex_event ADD COLUMN id UUID PRIMARY KEY DEFAULT gen_random_uuid();
