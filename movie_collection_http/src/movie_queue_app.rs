#![allow(clippy::needless_pass_by_value)]

use anyhow::Error;
use stack_string::StackString;
use std::{net::SocketAddr, path::Path, time::Duration};
use tokio::{
    fs::{create_dir, remove_dir_all},
    time::interval,
};
use warp::Filter;

use movie_collection_lib::{
    config::Config, pgpool::PgPool, trakt_connection::TraktConnection, trakt_utils::TraktActions,
};

use super::{
    errors::error_response,
    logged_user::{fill_from_db, get_secrets, TRIGGER_DB_UPDATE},
    movie_queue_routes::{
        find_new_episodes, frontpage, imdb_episodes_route, imdb_episodes_update,
        imdb_ratings_route, imdb_ratings_set_source, imdb_ratings_update, imdb_show,
        last_modified_route, movie_collection_route, movie_collection_update, movie_queue,
        movie_queue_delete, movie_queue_play, movie_queue_remcom_directory_file,
        movie_queue_remcom_file, movie_queue_route, movie_queue_show, movie_queue_transcode,
        movie_queue_transcode_cleanup, movie_queue_transcode_directory, movie_queue_transcode_file,
        movie_queue_transcode_status, movie_queue_update, refresh_auth, trakt_auth_url, trakt_cal,
        trakt_callback, trakt_watched_action, trakt_watched_list, trakt_watched_seasons,
        trakt_watchlist, trakt_watchlist_action, tvshows, user,
    },
};

#[derive(Clone)]
pub struct AppState {
    pub config: Config,
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
    let config = Config::with_config()?;
    get_secrets(&config.secret_path, &config.jwt_secret_path).await?;

    let partial_path = Path::new("/var/www/html/videos/partial");
    if partial_path.exists() {
        remove_dir_all(&partial_path).await?;
        create_dir(&partial_path).await?;
    }

    let pool = PgPool::new(&config.pgurl);
    let trakt = TraktConnection::new(config.clone());

    tokio::task::spawn(_update_db(pool.clone()));

    run_app(config, pool, trakt).await
}

async fn run_app(config: Config, pool: PgPool, trakt: TraktConnection) -> Result<(), Error> {
    let port = config.port;
    let data = AppState {
        config,
        db: pool,
        trakt,
    };

    let data = warp::any().map(move || data.clone());

    let frontpage_path = warp::path("index.html")
        .and(warp::path::end())
        .and(warp::get())
        .and(warp::cookie("jwt"))
        .and(data.clone())
        .and_then(frontpage)
        .boxed();
    let find_new_episodes_path = warp::path("cal")
        .and(warp::path::end())
        .and(warp::get())
        .and(warp::query())
        .and(warp::cookie("jwt"))
        .and(data.clone())
        .and_then(find_new_episodes)
        .boxed();
    let tvshows_path = warp::path("tvshows")
        .and(warp::path::end())
        .and(warp::get())
        .and(warp::cookie("jwt"))
        .and(data.clone())
        .and_then(tvshows)
        .boxed();
    let movie_queue_delete_path = warp::path!("delete" / StackString)
        .and(warp::path::end())
        .and(warp::get())
        .and(warp::cookie("jwt"))
        .and(data.clone())
        .and_then(movie_queue_delete)
        .boxed();
    //                 .service(
    //                     web::scope("/transcode")
    let movie_queue_transcode_status_path = warp::path("transcode")
        .and(warp::path::end())
        .and(warp::get())
        .and(warp::cookie("jwt"))
        .and(data.clone())
        .and_then(movie_queue_transcode_status)
        .boxed();
    let movie_queue_transcode_file_path = warp::path!("file" / StackString)
        .and(warp::path::end())
        .and(warp::get())
        .and(warp::cookie("jwt"))
        .and(data.clone())
        .and_then(movie_queue_transcode_file)
        .boxed();
    let movie_queue_remcom_file_path = warp::path!("remcom" / "file" / StackString)
        .and(warp::path::end())
        .and(warp::get())
        .and(warp::cookie("jwt"))
        .and(data.clone())
        .and_then(movie_queue_remcom_file)
        .boxed();
    let movie_queue_remcom_directory_file_path =
        warp::path!("remcom" / "directory" / StackString / StackString)
            .and(warp::path::end())
            .and(warp::get())
            .and(warp::cookie("jwt"))
            .and(data.clone())
            .and_then(movie_queue_remcom_directory_file)
            .boxed();
    let movie_queue_transcode_path = warp::path!("queue" / StackString)
        .and(warp::path::end())
        .and(warp::get())
        .and(warp::cookie("jwt"))
        .and(data.clone())
        .and_then(movie_queue_transcode)
        .boxed();
    let movie_queue_transcode_directory_path = warp::path!("queue" / StackString / StackString)
        .and(warp::path::end())
        .and(warp::get())
        .and(warp::cookie("jwt"))
        .and(data.clone())
        .and_then(movie_queue_transcode_directory)
        .boxed();
    let movie_queue_transcode_cleanup_path = warp::path!("cleanup" / StackString)
        .and(warp::path::end())
        .and(warp::get())
        .and(warp::cookie("jwt"))
        .and(data.clone())
        .and_then(movie_queue_transcode_cleanup)
        .boxed();
    let transcode_path = warp::path("transcode")
        .and(
            movie_queue_transcode_status_path
                .or(movie_queue_transcode_file_path)
                .or(movie_queue_remcom_file_path)
                .or(movie_queue_remcom_directory_file_path)
                .or(movie_queue_transcode_path)
                .or(movie_queue_transcode_directory_path)
                .or(movie_queue_transcode_cleanup_path),
        )
        .boxed();
    let movie_queue_play_path = warp::path!("play" / i32)
        .and(warp::path::end())
        .and(warp::get())
        .and(warp::cookie("jwt"))
        .and(data.clone())
        .and_then(movie_queue_play)
        .boxed();
    let imdb_episodes_get = warp::get()
        .and(warp::path::end())
        .and(warp::query())
        .and(warp::cookie("jwt"))
        .and(data.clone())
        .and_then(imdb_episodes_route);
    let imdb_episodes_post = warp::post()
        .and(warp::body::json())
        .and(warp::cookie("jwt"))
        .and(data.clone())
        .and_then(imdb_episodes_update);
    let imdb_episodes_path = warp::path("imdb_episodes")
        .and(imdb_episodes_get.or(imdb_episodes_post))
        .boxed();
    let imdb_ratings_set_source_path = warp::path!("imdb_ratings" / "set_source")
        .and(warp::path::end())
        .and(warp::get())
        .and(warp::query())
        .and(warp::cookie("jwt"))
        .and(data.clone())
        .and_then(imdb_ratings_set_source)
        .boxed();
    let imdb_ratings_get = warp::get()
        .and(warp::path::end())
        .and(warp::query())
        .and(warp::cookie("jwt"))
        .and(data.clone())
        .and_then(imdb_ratings_route);
    let imdb_ratings_post = warp::post()
        .and(warp::body::json())
        .and(warp::cookie("jwt"))
        .and(data.clone())
        .and_then(imdb_ratings_update);
    let imdb_ratings_path = warp::path("imdb_ratings")
        .and(imdb_ratings_get.or(imdb_ratings_post))
        .boxed();
    let movie_queue_get = warp::get()
        .and(warp::path::end())
        .and(warp::query())
        .and(warp::cookie("jwt"))
        .and(data.clone())
        .and_then(movie_queue_route);
    let movie_queue_post = warp::post()
        .and(warp::body::json())
        .and(warp::cookie("jwt"))
        .and(data.clone())
        .and_then(movie_queue_update);
    let movie_queue_path = warp::path("imdb_ratings")
        .and(movie_queue_get.or(movie_queue_post))
        .boxed();
    let movie_collection_get = warp::get()
        .and(warp::path::end())
        .and(warp::query())
        .and(warp::cookie("jwt"))
        .and(data.clone())
        .and_then(movie_collection_route);
    let movie_collection_post = warp::post()
        .and(warp::body::json())
        .and(warp::cookie("jwt"))
        .and(data.clone())
        .and_then(movie_collection_update);
    let movie_collection_path = warp::path("imdb_ratings")
        .and(movie_collection_get.or(movie_collection_post))
        .boxed();
    let imdb_show_path = warp::path!("imdb" / StackString)
        .and(warp::path::end())
        .and(warp::get())
        .and(warp::query())
        .and(warp::cookie("jwt"))
        .and(data.clone())
        .and_then(imdb_show)
        .boxed();
    let last_modified_path = warp::path("last_modified")
        .and(warp::path::end())
        .and(warp::get())
        .and(warp::cookie("jwt"))
        .and(data.clone())
        .and_then(last_modified_route)
        .boxed();
    let user_path = warp::path("user")
        .and(warp::path::end())
        .and(warp::get())
        .and(warp::cookie("jwt"))
        .and_then(user)
        .boxed();
    let full_queue_path = warp::path("full_queue")
        .and(warp::path::end())
        .and(warp::get())
        .and(warp::cookie("jwt"))
        .and(data.clone())
        .and_then(movie_queue)
        .boxed();
    let movie_queue_show_path = warp::path!(StackString)
        .and(warp::path::end())
        .and(warp::get())
        .and(warp::cookie("jwt"))
        .and(data.clone())
        .and_then(movie_queue_show)
        .boxed();
    let list_path = warp::path("list").and(
        frontpage_path
            .or(find_new_episodes_path)
            .or(tvshows_path)
            .or(movie_queue_delete_path)
            .or(transcode_path)
            .or(movie_queue_play_path)
            .or(imdb_episodes_path)
            .or(imdb_ratings_set_source_path)
            .or(imdb_ratings_path)
            .or(movie_queue_path)
            .or(movie_collection_path)
            .or(imdb_show_path)
            .or(last_modified_path)
            .or(user_path)
            .or(full_queue_path)
            .or(movie_queue_show_path),
    );
    //         .service(
    //             web::scope("/trakt")
    let auth_url_path = warp::path("auth_url")
        .and(warp::path::end())
        .and(warp::get())
        .and(warp::cookie("jwt"))
        .and(data.clone())
        .and_then(trakt_auth_url)
        .boxed();
    // .service(web::resource("/callback").route(web::get().to(trakt_callback)))
    let trakt_callback_path = warp::path("callback")
        .and(warp::path::end())
        .and(warp::get())
        .and(warp::query())
        .and(warp::cookie("jwt"))
        .and(data.clone())
        .and_then(trakt_callback)
        .boxed();
    let refresh_auth_path = warp::path("refresh_auth")
        .and(warp::path::end())
        .and(warp::get())
        .and(warp::cookie("jwt"))
        .and(data.clone())
        .and_then(refresh_auth)
        .boxed();
    let trakt_cal_path = warp::path("cal")
        .and(warp::path::end())
        .and(warp::get())
        .and(warp::cookie("jwt"))
        .and(data.clone())
        .and_then(trakt_cal)
        .boxed();
    let trakt_watchlist_path = warp::path("watchlist")
        .and(warp::path::end())
        .and(warp::get())
        .and(warp::cookie("jwt"))
        .and(data.clone())
        .and_then(trakt_watchlist)
        .boxed();
    let trakt_watchlist_action_path = warp::path!("watchlist" / TraktActions / StackString)
        .and(warp::path::end())
        .and(warp::get())
        .and(warp::cookie("jwt"))
        .and(data.clone())
        .and_then(trakt_watchlist_action)
        .boxed();
    let trakt_watched_seasons_path = warp::path!("watched" / "list" / StackString)
        .and(warp::path::end())
        .and(warp::get())
        .and(warp::cookie("jwt"))
        .and(data.clone())
        .and_then(trakt_watched_seasons)
        .boxed();
    let trakt_watched_list_path = warp::path!("watched" / "list" / StackString / i32)
        .and(warp::path::end())
        .and(warp::get())
        .and(warp::cookie("jwt"))
        .and(data.clone())
        .and_then(trakt_watched_list)
        .boxed();
    let trakt_watched_action_path = warp::path!("watched" / TraktActions / StackString / i32 / i32)
        .and(warp::path::end())
        .and(warp::get())
        .and(warp::cookie("jwt"))
        .and(data.clone())
        .and_then(trakt_watched_action)
        .boxed();
    let trakt_path = warp::path("trakt")
        .and(
            auth_url_path
                .or(trakt_callback_path)
                .or(refresh_auth_path)
                .or(trakt_cal_path)
                .or(trakt_watchlist_path)
                .or(trakt_watchlist_action_path)
                .or(trakt_watched_seasons_path)
                .or(trakt_watched_list_path)
                .or(trakt_watched_action_path),
        )
        .boxed();

    let full_path = list_path.or(trakt_path);

    let routes = full_path.recover(error_response);
    let addr: SocketAddr = format!("127.0.0.1:{}", port).parse()?;
    warp::serve(routes).bind(addr).await;
    Ok(())
}
