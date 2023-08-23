#![allow(clippy::too_many_lines)]
#![allow(clippy::module_name_repetitions)]
#![allow(clippy::cast_precision_loss)]
#![allow(clippy::cast_sign_loss)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_possible_wrap)]
#![allow(clippy::similar_names)]
#![allow(clippy::struct_excessive_bools)]
#![allow(clippy::unsafe_derive_deserialize)]

pub mod config;
pub mod date_time_wrapper;
pub mod imdb_episodes;
pub mod imdb_ratings;
pub mod imdb_utils;
pub mod make_list;
pub mod make_queue;
pub mod movie_collection;
pub mod movie_queue;
pub mod music_collection;
pub mod parse_imdb;
pub mod pgpool;
pub mod plex_events;
pub mod timezone;
pub mod trakt_connection;
pub mod trakt_utils;
pub mod transcode_service;
pub mod tv_show_source;
pub mod utils;

pub fn init_env() {
    use std::env::set_var;

    set_var(
        "PGURL",
        "postgresql://USER:PASSWORD@localhost:5432/movie_queue",
    );
    set_var("AUTHDB", "postgresql://USER:PASSWORD@localhost:5432/auth");
    set_var("MOVIE_DIRS", "/tmp");
    set_var("MUSIC_DIRS", "/tmp");
    set_var("PREFERED_DISK", "/tmp");
    set_var("JWT_SECRET", "JWT_SECRET");
    set_var("SECRET_KEY", "SECRET_KEY");
    set_var("DOMAIN", "DOMAIN");
    set_var("SPARKPOST_API_KEY", "SPARKPOST_API_KEY");
    set_var("SENDING_EMAIL_ADDRESS", "SENDING_EMAIL_ADDRESS");
    set_var("CALLBACK_URL", "https://{DOMAIN}/auth/register.html");
    set_var("TRAKT_CLIENT_ID", "8675309");
    set_var("TRAKT_CLIENT_SECRET", "8675309");
    set_var("SECRET_PATH", "/tmp/secret.bin");
    set_var("JWT_SECRET_PATH", "/tmp/jwt_secret.bin");
    set_var("VIDEO_PLAYBACK_PATH", "/tmp/html");
    set_var("PLEX_WEBHOOK_KEY", "6f609260-4fd2-4a2c-919c-9c7766bd6400");
}
