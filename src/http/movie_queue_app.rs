#![allow(clippy::needless_pass_by_value)]

use actix::sync::SyncArbiter;
use actix::Addr;
use actix_web::middleware::identity::{CookieIdentityPolicy, IdentityService};
use actix_web::{http::Method, server, App};
use chrono::Duration;

use super::logged_user::AuthorizedUsers;
use super::movie_queue_routes::{
    find_new_episodes, imdb_episodes_route, imdb_episodes_update, imdb_ratings_route,
    imdb_ratings_update, imdb_show, last_modified_route, movie_collection_route,
    movie_collection_update, movie_queue, movie_queue_delete, movie_queue_play, movie_queue_route,
    movie_queue_show, movie_queue_transcode, movie_queue_transcode_directory, movie_queue_update,
};
use super::trakt_routes::{
    trakt_cal, trakt_watched_action, trakt_watched_list, trakt_watched_seasons, trakt_watchlist,
    trakt_watchlist_action,
};
use super::tvshows_route::tvshows;
use crate::common::config::Config;
use crate::common::pgpool::PgPool;

pub struct AppState {
    pub db: Addr<PgPool>,
    pub user_list: AuthorizedUsers,
}

pub fn start_app(config: Config) {
    let secret: String = std::env::var("SECRET_KEY").unwrap_or_else(|_| "0123".repeat(8));
    let domain = config.domain.clone();
    let port = config.port;
    let nconn = config.n_db_workers;
    let pool = PgPool::new(&config.pgurl);
    let user_list = AuthorizedUsers::new();

    let addr: Addr<PgPool> = SyncArbiter::start(nconn, move || pool.clone());

    server::new(move || {
        App::with_state(AppState {
            db: addr.clone(),
            user_list: user_list.clone(),
        })
        .middleware(IdentityService::new(
            CookieIdentityPolicy::new(secret.as_bytes())
                .name("auth")
                .path("/")
                .domain(domain.as_str())
                .max_age(Duration::days(1))
                .secure(false), // this can only be true if you have https
        ))
        .resource("/list/cal", |r| {
            r.method(Method::GET).with(find_new_episodes)
        })
        .resource("/list/tvshows", |r| r.method(Method::GET).with(tvshows))
        .resource("/list/delete/{path}", |r| {
            r.method(Method::GET).with(movie_queue_delete)
        })
        .resource("/list/transcode/{file}", |r| {
            r.method(Method::GET).with(movie_queue_transcode)
        })
        .resource("/list/transcode/{directory}/{file}", |r| {
            r.method(Method::GET).with(movie_queue_transcode_directory)
        })
        .resource("/list/play/{index}", |r| {
            r.method(Method::GET).with(movie_queue_play)
        })
        .resource("/list/trakt/cal", |r| r.method(Method::GET).with(trakt_cal))
        .resource("/list/trakt/watchlist", |r| {
            r.method(Method::GET).with(trakt_watchlist)
        })
        .resource("/list/trakt/watchlist/{action}/{imdb_url}", |r| {
            r.method(Method::GET).with(trakt_watchlist_action)
        })
        .resource("/list/trakt/watched/list/{imdb_url}", |r| {
            r.method(Method::GET).with(trakt_watched_seasons)
        })
        .resource("/list/trakt/watched/list/{imdb_url}/{season}", |r| {
            r.method(Method::GET).with(trakt_watched_list)
        })
        .resource(
            "/list/trakt/watched/{action}/{imdb_url}/{season}/{episode}",
            |r| r.method(Method::GET).with(trakt_watched_action),
        )
        .resource("/list/imdb_episodes", |r| {
            r.method(Method::GET).with(imdb_episodes_route);
            r.method(Method::POST).with(imdb_episodes_update);
        })
        .resource("/list/imdb_ratings", |r| {
            r.method(Method::GET).with(imdb_ratings_route);
            r.method(Method::POST).with(imdb_ratings_update);
        })
        .resource("/list/movie_queue", |r| {
            r.method(Method::GET).with(movie_queue_route);
            r.method(Method::POST).with(movie_queue_update);
        })
        .resource("/list/movie_collection", |r| {
            r.method(Method::GET).with(movie_collection_route);
            r.method(Method::POST).with(movie_collection_update);
        })
        .resource("/list/imdb/{show}", |r| {
            r.method(Method::GET).with(imdb_show)
        })
        .resource("/list/last_modified", |r| {
            r.method(Method::GET).with(last_modified_route)
        })
        .resource("/list/{show}", |r| {
            r.method(Method::GET).with(movie_queue_show)
        })
        .resource("/list/", |r| r.method(Method::GET).with(movie_queue))
        .resource("/list", |r| r.method(Method::GET).with(movie_queue))
    })
    .bind(&format!("127.0.0.1:{}", port))
    .unwrap_or_else(|_| panic!("Failed to bind to port {}", port))
    .start();
}
