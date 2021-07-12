CREATE SEQUENCE IF NOT EXISTS movie_collection_idx_seq;

ALTER TABLE movie_collection ALTER COLUMN idx SET DEFAULT nextval('movie_collection_idx_seq'::regclass);
ALTER TABLE movie_collection ADD COLUMN IF NOT EXISTS is_deleted BOOLEAN NOT NULL DEFAULT false;