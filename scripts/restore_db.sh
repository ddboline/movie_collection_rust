#!/bin/bash

DB="movie_queue"
BUCKET=movie-queue-db-backup

TABLES="
imdb_ratings
imdb_episodes
movie_collection_on_dvd
movie_collection
movie_queue
trakt_watched_episodes
trakt_watched_movies
trakt_watchlist"

mkdir -p backup
for T in $TABLES;
do
    aws s3 cp s3://${BUCKET}/${T}.sql.gz backup/${T}.sql.gz
    gzip -dc backup/${T}.sql.gz | psql $DB -c "COPY $T FROM STDIN";
done

psql $DB -c "select setval('imdb_ratings_id_seq', (select max(index) from imdb_ratings), TRUE)"
psql $DB -c "select setval('imdb_episodes_id_seq', (select max(id) from imdb_episodes), TRUE)"
psql $DB -c "select setval('trakt_watched_episodes_id_seq', (select max(id) from trakt_watched_episodes), TRUE)"
psql $DB -c "select setval('trakt_watched_movies_id_seq', (select max(id) from trakt_watched_movies), TRUE)"
psql $DB -c "select setval('trakt_watchlist_id_seq', (select max(id) from trakt_watchlist), TRUE)"
