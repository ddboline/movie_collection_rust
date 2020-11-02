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

use super::{
    logged_user::{fill_from_db, get_secrets, SECRET_KEY, TRIGGER_DB_UPDATE},
    routes::{
        refresh_auth, trakt_auth_url, trakt_cal, trakt_callback, trakt_watched_action,
        trakt_watched_list, trakt_watched_seasons, trakt_watchlist, trakt_watchlist_action,
    },
};
use movie_collection_lib::{config::Config, pgpool::PgPool, trakt_connection::TraktConnection};

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
    let port = CONFIG.port;
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
                web::scope("/trakt")
                    .service(web::resource("/auth_url").route(web::get().to(trakt_auth_url)))
                    .service(web::resource("/callback").route(web::get().to(trakt_callback)))
                    .service(web::resource("/refresh_auth").route(web::get().to(refresh_auth)))
                    .service(web::resource("/cal").route(web::get().to(trakt_cal)))
                    .service(web::resource("/watchlist").route(web::get().to(trakt_watchlist)))
                    .service(
                        web::resource("/watchlist/{action}/{imdb_url}")
                            .route(web::get().to(trakt_watchlist_action)),
                    )
                    .service(
                        web::resource("/watched/list/{imdb_url}")
                            .route(web::get().to(trakt_watched_seasons)),
                    )
                    .service(
                        web::resource("/watched/list/{imdb_url}/{season}")
                            .route(web::get().to(trakt_watched_list)),
                    )
                    .service(
                        web::resource("/watched/{action}/{imdb_url}/{season}/{episode}")
                            .route(web::get().to(trakt_watched_action)),
                    ),
            )
    })
    .bind(&format!("127.0.0.1:{}", port))
    .unwrap_or_else(|_| panic!("Failed to bind to port {}", port))
    .run()
    .await
    .map_err(Into::into)
}
