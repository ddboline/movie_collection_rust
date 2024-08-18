ALTER TABLE imdb_episodes ALTER COLUMN last_modified SET DEFAULT now();
ALTER TABLE imdb_ratings ALTER COLUMN last_modified SET DEFAULT now();
UPDATE imdb_ratings SET last_modified=now() WHERE last_modified IS NULL;
ALTER TABLE imdb_ratings ALTER COLUMN last_modified SET NOT NULL;
ALTER TABLE music_collection ALTER COLUMN last_modified SET DEFAULT now();
UPDATE music_collection SET last_modified=now() WHERE last_modified IS NULL;
ALTER TABLE music_collection ALTER COLUMN last_modified SET NOT NULL;