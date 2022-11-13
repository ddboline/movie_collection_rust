ALTER TABLE imdb_episodes ALTER COLUMN rating DROP NOT NULL;

UPDATE imdb_episodes
SET airdate = null
WHERE airdate = '1970-01-01';

UPDATE imdb_episodes
SET rating = null
WHERE rating = -1.0;
