#![allow(clippy::needless_pass_by_value)]

use actix::sync::SyncArbiter;
use actix::Addr;
use actix_identity::{CookieIdentityPolicy, IdentityService};
use actix_web::{web, App, HttpServer};
use chrono::Duration;
use futures::future::Future;
use futures::stream::Stream;
use std::time;
use tokio_timer::Interval;

use super::logged_user::AUTHORIZED_USERS;
use super::movie_queue_routes::{
    find_new_episodes, frontpage, imdb_episodes_route, imdb_episodes_update, imdb_ratings_route,
    imdb_ratings_update, imdb_show, last_modified_route, movie_collection_route,
    movie_collection_update, movie_queue, movie_queue_delete, movie_queue_play, movie_queue_route,
    movie_queue_show, movie_queue_transcode, movie_queue_transcode_directory, movie_queue_update,
};
use super::trakt_routes::{
    trakt_cal, trakt_watched_action, trakt_watched_list, trakt_watched_seasons, trakt_watchlist,
    trakt_watchlist_action,
};
use super::tvshows_route::tvshows;
use movie_collection_lib::common::config::Config;
use movie_collection_lib::common::pgpool::PgPool;

pub struct AppState {
    pub db: Addr<PgPool>,
}

pub fn start_app(config: Config) {
    let secret: String = std::env::var("SECRET_KEY").unwrap_or_else(|_| "0123".repeat(8));
    let domain = config.domain.to_string();
    let port = config.port;
    let nconn = config.n_db_workers;
    let pool = PgPool::new(&config.pgurl);

    let _u = AUTHORIZED_USERS.clone();
    let _p = pool.clone();

    actix_rt::spawn(
        Interval::new(time::Instant::now(), time::Duration::from_secs(60))
            .for_each(move |_| {
                _u.fill_from_db(&_p).unwrap_or(());
                Ok(())
            })
            .map_err(|e| panic!("error {:?}", e)),
    );

    let addr: Addr<PgPool> = SyncArbiter::start(nconn, move || pool.clone());

    HttpServer::new(move || {
        App::new()
            .data(AppState { db: addr.clone() })
            .wrap(IdentityService::new(
                CookieIdentityPolicy::new(secret.as_bytes())
                    .name("auth")
                    .path("/")
                    .domain(domain.as_str())
                    .max_age_time(Duration::days(1))
                    .secure(false), // this can only be true if you have https
            ))
            .service(web::resource("/list/index.html").route(web::get().to(frontpage)))
            .service(web::resource("/list/cal").route(web::get().to_async(find_new_episodes)))
            .service(web::resource("/list/tvshows").route(web::get().to_async(tvshows)))
            .service(
                web::resource("/list/delete/{path}").route(web::get().to_async(movie_queue_delete)),
            )
            .service(
                web::resource("/list/transcode/{file}")
                    .route(web::get().to_async(movie_queue_transcode)),
            )
            .service(
                web::resource("/list/transcode/{directory}/{file}")
                    .route(web::get().to_async(movie_queue_transcode_directory)),
            )
            .service(
                web::resource("/list/play/{index}").route(web::get().to_async(movie_queue_play)),
            )
            .service(web::resource("/list/trakt/cal").route(web::get().to_async(trakt_cal)))
            .service(
                web::resource("/list/trakt/watchlist").route(web::get().to_async(trakt_watchlist)),
            )
            .service(
                web::resource("/list/trakt/watchlist/{action}/{imdb_url}")
                    .route(web::get().to_async(trakt_watchlist_action)),
            )
            .service(
                web::resource("/list/trakt/watched/list/{imdb_url}")
                    .route(web::get().to_async(trakt_watched_seasons)),
            )
            .service(
                web::resource("/list/trakt/watched/list/{imdb_url}/{season}")
                    .route(web::get().to_async(trakt_watched_list)),
            )
            .service(
                web::resource("/list/trakt/watched/{action}/{imdb_url}/{season}/{episode}")
                    .route(web::get().to_async(trakt_watched_action)),
            )
            .service(
                web::resource("/list/imdb_episodes")
                    .route(web::get().to_async(imdb_episodes_route))
                    .route(web::post().to_async(imdb_episodes_update)),
            )
            .service(
                web::resource("/list/imdb_ratings")
                    .route(web::get().to_async(imdb_ratings_route))
                    .route(web::post().to_async(imdb_ratings_update)),
            )
            .service(
                web::resource("/list/movie_queue")
                    .route(web::get().to_async(movie_queue_route))
                    .route(web::post().to_async(movie_queue_update)),
            )
            .service(
                web::resource("/list/movie_collection")
                    .route(web::get().to_async(movie_collection_route))
                    .route(web::post().to_async(movie_collection_update)),
            )
            .service(web::resource("/list/imdb/{show}").route(web::get().to_async(imdb_show)))
            .service(
                web::resource("/list/last_modified")
                    .route(web::get().to_async(last_modified_route)),
            )
            .service(web::resource("/list/{show}").route(web::get().to_async(movie_queue_show)))
            .service(web::resource("/list/").route(web::get().to_async(movie_queue)))
            .service(web::resource("/list").route(web::get().to_async(movie_queue)))
    })
    .bind(&format!("127.0.0.1:{}", port))
    .unwrap_or_else(|_| panic!("Failed to bind to port {}", port))
    .start();
}
