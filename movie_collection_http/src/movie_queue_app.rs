#![allow(clippy::needless_pass_by_value)]

use actix_identity::{CookieIdentityPolicy, IdentityService};
use actix_web::{web, App, HttpServer};
use anyhow::Error;
use std::{path::Path, time::Duration};
use tokio::{
    fs::{create_dir, remove_dir_all},
    time::interval,
};

use super::{
    logged_user::{fill_from_db, get_secrets, SECRET_KEY, TRIGGER_DB_UPDATE},
    movie_queue_routes::{
        find_new_episodes, frontpage, imdb_episodes_route, imdb_episodes_update,
        imdb_ratings_route, imdb_ratings_update, imdb_show, last_modified_route,
        movie_collection_route, movie_collection_update, movie_queue, movie_queue_delete,
        movie_queue_play, movie_queue_route, movie_queue_show, movie_queue_transcode,
        movie_queue_transcode_directory, movie_queue_update, refresh_auth, trakt_auth_url,
        trakt_cal, trakt_callback, trakt_watched_action, trakt_watched_list, trakt_watched_seasons,
        trakt_watchlist, trakt_watchlist_action, tvshows, user,
    },
};
use movie_collection_lib::{config::Config, pgpool::PgPool};

pub struct AppState {
    pub db: PgPool,
}

pub async fn start_app(config: Config) -> Result<(), Error> {
    async fn _update_db(pool: PgPool) {
        let mut i = interval(Duration::from_secs(60));
        loop {
            fill_from_db(&pool).await.unwrap_or(());
            i.tick().await;
        }
    }
    TRIGGER_DB_UPDATE.set();
    get_secrets(&config.secret_path, &config.jwt_secret_path).await?;

    let partial_path = Path::new("/var/www/html/videos/partial");
    if partial_path.exists() {
        remove_dir_all(&partial_path).await?;
        create_dir(&partial_path).await?;
    }

    let domain = config.domain.to_string();
    let port = config.port;
    let pool = PgPool::new(&config.pgurl);

    actix_rt::spawn(_update_db(pool.clone()));

    HttpServer::new(move || {
        App::new()
            .data(AppState { db: pool.clone() })
            .wrap(IdentityService::new(
                CookieIdentityPolicy::new(&SECRET_KEY.get())
                    .name("auth")
                    .path("/")
                    .domain(domain.as_str())
                    .max_age(24 * 3600)
                    .secure(false), // this can only be true if you have https
            ))
            .service(
                web::scope("/list")
                    .service(web::resource("/index.html").route(web::get().to(frontpage)))
                    .service(web::resource("/cal").route(web::get().to(find_new_episodes)))
                    .service(web::resource("/tvshows").route(web::get().to(tvshows)))
                    .service(
                        web::resource("/delete/{path}").route(web::get().to(movie_queue_delete)),
                    )
                    .service(
                        web::resource("/transcode/{file}")
                            .route(web::get().to(movie_queue_transcode)),
                    )
                    .service(
                        web::resource("/transcode/{directory}/{file}")
                            .route(web::get().to(movie_queue_transcode_directory)),
                    )
                    .service(web::resource("/play/{index}").route(web::get().to(movie_queue_play)))
                    .service(web::resource("/trakt/auth_url").route(web::get().to(trakt_auth_url)))
                    .service(web::resource("/trakt/callback").route(web::get().to(trakt_callback)))
                    .service(
                        web::resource("/trakt/refresh_auth").route(web::get().to(refresh_auth)),
                    )
                    .service(web::resource("/trakt/cal").route(web::get().to(trakt_cal)))
                    .service(
                        web::resource("/trakt/watchlist").route(web::get().to(trakt_watchlist)),
                    )
                    .service(
                        web::resource("/trakt/watchlist/{action}/{imdb_url}")
                            .route(web::get().to(trakt_watchlist_action)),
                    )
                    .service(
                        web::resource("/trakt/watched/list/{imdb_url}")
                            .route(web::get().to(trakt_watched_seasons)),
                    )
                    .service(
                        web::resource("/trakt/watched/list/{imdb_url}/{season}")
                            .route(web::get().to(trakt_watched_list)),
                    )
                    .service(
                        web::resource("/trakt/watched/{action}/{imdb_url}/{season}/{episode}")
                            .route(web::get().to(trakt_watched_action)),
                    )
                    .service(
                        web::resource("/imdb_episodes")
                            .route(web::get().to(imdb_episodes_route))
                            .route(web::post().to(imdb_episodes_update)),
                    )
                    .service(
                        web::resource("/imdb_ratings")
                            .route(web::get().to(imdb_ratings_route))
                            .route(web::post().to(imdb_ratings_update)),
                    )
                    .service(
                        web::resource("/movie_queue")
                            .route(web::get().to(movie_queue_route))
                            .route(web::post().to(movie_queue_update)),
                    )
                    .service(
                        web::resource("/movie_collection")
                            .route(web::get().to(movie_collection_route))
                            .route(web::post().to(movie_collection_update)),
                    )
                    .service(web::resource("/imdb/{show}").route(web::get().to(imdb_show)))
                    .service(
                        web::resource("/last_modified").route(web::get().to(last_modified_route)),
                    )
                    .service(web::resource("/user").route(web::get().to(user)))
                    .service(web::resource("/full_queue").route(web::get().to(movie_queue)))
                    .service(web::resource("/{show}").route(web::get().to(movie_queue_show))),
            )
    })
    .bind(&format!("127.0.0.1:{}", port))
    .unwrap_or_else(|_| panic!("Failed to bind to port {}", port))
    .run()
    .await
    .map_err(Into::into)
}
