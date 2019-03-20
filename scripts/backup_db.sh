#!/bin/bash

DB="movie_queue"

TABLES="
imdb_ratings
imdb_episodes
movie_collection_on_dvd
movie_collection
movie_queue
trakt_watched_episodes
trakt_watched_movies
trakt_watchlist"

for T in $TABLES;
do
    psql $DB -c "COPY $T TO STDOUT" | gzip > backup/${T}.sql.gz
    aws s3 cp backup/${T}.sql.gz s3://movie-queue-db-backup/${T}.sql.gz
done
