#!/bin/bash

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
    psql movie_queue -c "COPY $T FROM STDIN" < $(gzip -d backup/${T}.sql.gz)
done
