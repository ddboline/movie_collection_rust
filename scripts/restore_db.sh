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
    gzip -dc backup/${T}.sql.gz | psql movie_queue -c "COPY $T FROM STDIN"
done
