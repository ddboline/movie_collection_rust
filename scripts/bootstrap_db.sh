#!/bin/bash

if [ -z "$PASSWORD" ]; then
    PASSWORD=`head -c1000 /dev/urandom | tr -dc [:alpha:][:digit:] | head -c 16; echo ;`
fi
DB="movie_queue"

sudo apt-get install -y postgresql postgresql-client-common

sudo -u postgres createuser -E -e $USER
sudo -u postgres psql -c "CREATE ROLE $USER PASSWORD '$PASSWORD' NOSUPERUSER NOCREATEDB NOCREATEROLE INHERIT LOGIN;"
sudo -u postgres psql -c "ALTER ROLE $USER PASSWORD '$PASSWORD' NOSUPERUSER NOCREATEDB NOCREATEROLE INHERIT LOGIN;"
sudo -u postgres createdb $DB

mkdir -p ${HOME}/.config/movie_collection_rust/

cat > ${HOME}/.config/movie_collection_rust/config.env <<EOL
PGURL=postgresql://$USER:$PASSWORD@localhost:5432/$DB
MOVIEDIRS=$MOVIDIRS
PREFERED_DISK=$PREFERED_DISK
SECRET_PATH=${HOME}/.config/auth_server_rust/secret.bin
JWT_SECRET_PATH=${HOME}/.config/auth_server_rust/jwt_secret.bin
DOMAIN=$DOMAIN
SPARKPOST_API_KEY=$SPARKPOST_API_KEY
SENDING_EMAIL_ADDRESS=$SENDING_EMAIL_ADDRESS
CALLBACK_URL=https://${DOMAIN}/auth/register.html
EOL

cat > ${HOME}/.config/movie_collection_rust/postgres.toml <<EOL
[movie_collection_rust]
database_url = 'postgresql://$USER:$PASSWORD@localhost:5432/$DB'
destination = 'file://${HOME}/setup_files/build/movie_collection_rust/backup'
tables = ['imdb_ratings', 'imdb_episodes', 'movie_collection_on_dvd', 'movie_collection', 'movie_queue', 'trakt_watched_episodes', 'trakt_watched_movies', 'trakt_watchlist']
sequences = {imdb_ratings_id_seq=['imdb_ratings', 'index'], imdb_episodes_id_seq=['imdb_episodes', 'id'], trakt_watched_episodes_id_seq=['trakt_watched_episodes', 'id'], trakt_watched_movies_id_seq=['trakt_watched_movies', 'id'], trakt_watchlist_id_seq=['trakt_watchlist', 'id']}
EOL

psql $DB < ./scripts/authorized_users.sql
psql $DB < ./scripts/imdb_ratings.sql
psql $DB < ./scripts/imdb_episodes.sql
psql $DB < ./scripts/movie_collection_on_dvd.sql
psql $DB < ./scripts/movie_collection.sql
psql $DB < ./scripts/movie_queue.sql
psql $DB < ./scripts/trakt_watched_episodes.sql
psql $DB < ./scripts/trakt_watched_movies.sql
psql $DB < ./scripts/trakt_watchlist.sql
