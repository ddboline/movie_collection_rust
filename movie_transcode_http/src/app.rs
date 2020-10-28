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

use movie_collection_lib::{config::Config, pgpool::PgPool, trakt_connection::TraktConnection};

use super::{
    logged_user::{fill_from_db, get_secrets, SECRET_KEY, TRIGGER_DB_UPDATE},
    routes::{
        movie_queue_remcom_directory_file, movie_queue_remcom_file, movie_queue_transcode,
        movie_queue_transcode_cleanup, movie_queue_transcode_directory, movie_queue_transcode_file,
        movie_queue_transcode_status,
    },
};

lazy_static! {
    pub static ref CONFIG: Config = Config::with_config().expect("Config init failed");
}

pub struct AppState {
    pub db: PgPool,
    pub trakt: TraktConnection,
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
    let port = CONFIG.port + 2;
    let pool = PgPool::new(&CONFIG.pgurl);
    let trakt = TraktConnection::new(CONFIG.clone());

    actix_rt::spawn(_update_db(pool.clone()));

    HttpServer::new(move || {
        App::new()
            .data(AppState {
                db: pool.clone(),
                trakt: trakt.clone(),
            })
            .wrap(IdentityService::new(
                CookieIdentityPolicy::new(&SECRET_KEY.get())
                    .name("auth")
                    .path("/")
                    .domain(domain.as_str())
                    .max_age(24 * 3600)
                    .secure(false), // this can only be true if you have https
            ))
            .service(
                web::scope("/transcode")
                    .service(
                        web::resource("/status").route(web::get().to(movie_queue_transcode_status)),
                    )
                    .service(
                        web::resource("/file/{file}")
                            .route(web::get().to(movie_queue_transcode_file)),
                    )
                    .service(
                        web::resource("/remcom/file/{file}")
                            .route(web::get().to(movie_queue_remcom_file)),
                    )
                    .service(
                        web::resource("/remcom/directory/{directory}/{file}")
                            .route(web::get().to(movie_queue_remcom_directory_file)),
                    )
                    .service(
                        web::resource("/queue/{file}").route(web::get().to(movie_queue_transcode)),
                    )
                    .service(
                        web::resource("/queue/{directory}/{file}")
                            .route(web::get().to(movie_queue_transcode_directory)),
                    )
                    .service(
                        web::resource("/cleanup/{file}")
                            .route(web::get().to(movie_queue_transcode_cleanup)),
                    ),
            )
    })
    .bind(&format!("127.0.0.1:{}", port))?
    .run()
    .await
    .map_err(Into::into)
}
