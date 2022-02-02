ALTER TABLE movie_queue DROP CONSTRAINT movie_queue_pkey;
ALTER TABLE movie_queue ADD PRIMARY KEY (collection_idx);