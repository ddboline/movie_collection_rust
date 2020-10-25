#![allow(clippy::needless_pass_by_value)]

use actix_identity::{CookieIdentityPolicy, IdentityService};
use actix_web::{web, App, HttpServer};
use anyhow::Error;
use lazy_static::lazy_static;
use std::{path::Path, time::Duration};
use tokio::{
    fs::{create_dir, remove_dir_all},
    time::interval,
};

use movie_collection_lib::{config::Config, pgpool::PgPool};

use super::{
    logged_user::{fill_from_db, get_secrets, SECRET_KEY, TRIGGER_DB_UPDATE},
    routes::{
        imdb_episodes_route, imdb_episodes_update,
        imdb_ratings_route, imdb_ratings_update, 
        last_modified_route, movie_collection_route, movie_collection_update, movie_queue,
         movie_queue_route, movie_queue_show,
        movie_queue_update,
    },
};

lazy_static! {
    pub static ref CONFIG: Config = Config::with_config().expect("Config init failed");
}

pub struct AppState {
    pub db: PgPool,
}

pub async fn start_app() -> Result<(), Error> {
    async fn _update_db(pool: PgPool) {
        let mut i = interval(Duration::from_secs(60));
        loop {
            fill_from_db(&pool).await.unwrap_or(());
            i.tick().await;
        }
    }
    TRIGGER_DB_UPDATE.set();
    get_secrets(&CONFIG.secret_path, &CONFIG.jwt_secret_path).await?;

    let partial_path = Path::new("/var/www/html/videos/partial");
    if partial_path.exists() {
        remove_dir_all(&partial_path).await?;
        create_dir(&partial_path).await?;
    }

    let domain = CONFIG.domain.to_string();
    let port = CONFIG.port;
    let pool = PgPool::new(&CONFIG.pgurl);

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
                web::scope("/sync")
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
                .service(
                    web::resource("/last_modified").route(web::get().to(last_modified_route)),
                )
                .service(web::resource("/full_queue").route(web::get().to(movie_queue)))
                .service(web::resource("/queue/{show}").route(web::get().to(movie_queue_show)))
                .service(
                    web::resource("/imdb_episodes")
                        .route(web::get().to(imdb_episodes_route))
                        .route(web::post().to(imdb_episodes_update)),
                )
                .service(
                    web::resource("/imdb_ratings")
                        .route(web::get().to(imdb_ratings_route))
                        .route(web::post().to(imdb_ratings_update)),
                ),
            )
    })
    .bind(&format!("127.0.0.1:{}", port))?
    .run()
    .await
    .map_err(Into::into)
}
