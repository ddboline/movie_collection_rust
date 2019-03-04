#!/bin/bash

DB="movie_queue"

TABLES="
imdb_episodes
movie_collection_on_dvd
movie_queue
trakt_watched_episodes
trakt_watched_movies
trakt_watchlist
movie_collection
imdb_ratings
"

for T in $TABLES;
do
    psql $DB -c "DELETE FROM $T";
done
