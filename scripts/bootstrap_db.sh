#!/bin/bash

sudo bash -c "echo deb [trusted=yes] https://py2deb-repo.s3.amazonaws.com/deb/bionic/python3 bionic main > /etc/apt/sources.list.d/py2deb3.list"
sudo apt-key adv --receive-keys 25508FAF711C1DEB
sudo apt-get update
sudo apt-get install movie-collection-rust

PASSWORD=`head -c1000 /dev/urandom | tr -dc [:alpha:][:digit:] | head -c 16; echo ;`
JWT_SECRET=`head -c1000 /dev/urandom | tr -dc [:alpha:][:digit:] | head -c 32; echo ;`
SECRET_KEY=`head -c1000 /dev/urandom | tr -dc [:alpha:][:digit:] | head -c 32; echo ;`

sudo apt-get install -y postgresql postgresql-client-common

sudo -u postgres createuser -E -e $USER
sudo -u postgres psql -c "CREATE ROLE $USER PASSWORD '$PASSWORD' NOSUPERUSER NOCREATEDB NOCREATEROLE INHERIT LOGIN;"
sudo -u postgres psql -c "ALTER ROLE $USER PASSWORD '$PASSWORD' NOSUPERUSER NOCREATEDB NOCREATEROLE INHERIT LOGIN;"
sudo -u postgres createdb movie_queue

mkdir -p ${HOME}/.config/movie_collection_rust/

cat > ${HOME}/.config/movie_collection_rust/config.env <<EOL
PGURL=postgresql://$USER:$PASSWORD@localhost:5432/movie_queue
AUTHDB=postgresql://$USER:$PASSWORD@localhost:5432/auth
MOVIEDIRS=$MOVIDIRS
PREFERED_DISK=$PREFERED_DISK
JWT_SECRET=$JWT_SECRET
SECRET_KEY=$SECRET_KEY
DOMAIN=$DOMAIN
SPARKPOST_API_KEY=$SPARKPOST_API_KEY
SENDING_EMAIL_ADDRESS=$SENDING_EMAIL_ADDRESS
CALLBACK_URL=https://${DOMAIN}/auth/register.html
EOL

psql movie_queue < ./scripts/imdb_ratings.sql
psql movie_queue < ./scripts/imdb_episodes.sql
psql movie_queue < ./scripts/movie_collection_on_dvd.sql
psql movie_queue < ./scripts/movie_collection.sql
psql movie_queue < ./scripts/movie_queue.sql
psql movie_queue < ./scripts/trakt_watched_episodes.sql
psql movie_queue < ./scripts/trakt_watched_movies.sql
psql movie_queue < ./scripts/trakt_watchlist.sql
