#![allow(clippy::needless_pass_by_value)]

extern crate actix;
extern crate actix_web;
extern crate rust_auth_server;
extern crate subprocess;

use actix::sync::SyncArbiter;
use actix::Addr;
use actix_web::middleware::identity::{CookieIdentityPolicy, IdentityService};
use actix_web::{http::Method, server, App};
use chrono::Duration;

use super::movie_queue_routes::{
    imdb_show, movie_queue, movie_queue_delete, movie_queue_play, movie_queue_show,
    movie_queue_transcode, movie_queue_transcode_directory, trakt_cal, trakt_watched_action,
    trakt_watched_list, trakt_watched_seasons, trakt_watchlist, trakt_watchlist_action, tvshows,
};
use crate::common::config::Config;
use crate::common::pgpool::PgPool;

pub struct AppState {
    pub db: Addr<PgPool>,
}

pub fn start_app(config: Config) {
    let secret: String = std::env::var("SECRET_KEY").unwrap_or_else(|_| "0123".repeat(8));
    let domain = config.domain.clone();
    let port = config.port;
    let nconn = config.n_db_workers;
    let pool = PgPool::new(&config.pgurl);

    let addr: Addr<PgPool> = SyncArbiter::start(nconn, move || pool.clone());

    server::new(move || {
        App::with_state(AppState { db: addr.clone() })
            .middleware(IdentityService::new(
                CookieIdentityPolicy::new(secret.as_bytes())
                    .name("auth")
                    .path("/")
                    .domain(domain.as_str())
                    .max_age(Duration::days(1))
                    .secure(false), // this can only be true if you have https
            ))
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
            .resource("/list/{show}", |r| {
                r.method(Method::GET).with(movie_queue_show)
            })
            .resource("/list/imdb/{show}", |r| {
                r.method(Method::GET).with(imdb_show)
            })
            .resource("/list", |r| r.method(Method::GET).with(movie_queue))
    })
    .bind(&format!("127.0.0.1:{}", port))
    .unwrap_or_else(|_| panic!("Failed to bind to port {}", port))
    .start();
}
