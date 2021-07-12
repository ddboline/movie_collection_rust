#![allow(clippy::needless_pass_by_value)]

use anyhow::Error;
use handlebars::Handlebars;
use rweb::{
    filters::BoxedFilter,
    http::header::CONTENT_TYPE,
    openapi::{self, Info},
    Filter, Reply,
};
use std::{net::SocketAddr, sync::Arc, time::Duration};
use tokio::{
    fs::{create_dir, remove_dir_all},
    time::interval,
};

use movie_collection_lib::{
    config::Config, pgpool::PgPool, trakt_connection::TraktConnection, utils::get_templates,
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
        movie_queue_transcode_status, movie_queue_update, plex_webhook, refresh_auth,
        trakt_auth_url, trakt_cal, trakt_callback, trakt_watched_action, trakt_watched_list,
        trakt_watched_seasons, trakt_watchlist, trakt_watchlist_action, tvshows, user,
    },
};

#[derive(Clone)]
pub struct AppState {
    pub config: Config,
    pub db: PgPool,
    pub trakt: TraktConnection,
    pub hbr: Arc<Handlebars<'static>>,
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

    if let Some(partial_path) = &config.video_playback_path {
        let partial_path = partial_path.join("videos").join("partial");
        if partial_path.exists() {
            remove_dir_all(&partial_path).await?;
            create_dir(&partial_path).await?;
        }
    }

    let pool = PgPool::new(&config.pgurl);
    let trakt = TraktConnection::new(config.clone());

    tokio::task::spawn(_update_db(pool.clone()));

    run_app(config, pool, trakt).await
}

fn get_full_path(app: &AppState) -> BoxedFilter<(impl Reply,)> {
    let frontpage_path = frontpage().boxed();
    let find_new_episodes_path = find_new_episodes(app.clone()).boxed();
    let tvshows_path = tvshows(app.clone()).boxed();
    let movie_queue_delete_path = movie_queue_delete(app.clone()).boxed();
    let movie_queue_transcode_status_path = movie_queue_transcode_status(app.clone()).boxed();
    let movie_queue_transcode_file_path = movie_queue_transcode_file(app.clone()).boxed();
    let movie_queue_remcom_file_path = movie_queue_remcom_file(app.clone()).boxed();
    let movie_queue_remcom_directory_file_path =
        movie_queue_remcom_directory_file(app.clone()).boxed();
    let movie_queue_transcode_path = movie_queue_transcode(app.clone()).boxed();
    let movie_queue_transcode_directory_path = movie_queue_transcode_directory(app.clone()).boxed();
    let movie_queue_transcode_cleanup_path = movie_queue_transcode_cleanup(app.clone()).boxed();
    let transcode_path = movie_queue_transcode_status_path
        .or(movie_queue_transcode_file_path)
        .or(movie_queue_remcom_file_path)
        .or(movie_queue_remcom_directory_file_path)
        .or(movie_queue_transcode_path)
        .or(movie_queue_transcode_directory_path)
        .or(movie_queue_transcode_cleanup_path)
        .boxed();
    let movie_queue_play_path = movie_queue_play(app.clone()).boxed();
    let imdb_episodes_get = imdb_episodes_route(app.clone());
    let imdb_episodes_post = imdb_episodes_update(app.clone());
    let imdb_episodes_path = imdb_episodes_get.or(imdb_episodes_post).boxed();
    let imdb_ratings_set_source_path = imdb_ratings_set_source(app.clone()).boxed();
    let imdb_ratings_get = imdb_ratings_route(app.clone());
    let imdb_ratings_post = imdb_ratings_update(app.clone());
    let imdb_ratings_path = imdb_ratings_get.or(imdb_ratings_post).boxed();
    let movie_queue_get = movie_queue_route(app.clone());
    let movie_queue_post = movie_queue_update(app.clone());
    let movie_queue_path = movie_queue_get.or(movie_queue_post).boxed();
    let movie_collection_get = movie_collection_route(app.clone());
    let movie_collection_post = movie_collection_update(app.clone());
    let movie_collection_path = movie_collection_get.or(movie_collection_post).boxed();
    let imdb_show_path = imdb_show(app.clone()).boxed();
    let last_modified_path = last_modified_route(app.clone()).boxed();
    let user_path = user().boxed();
    let full_queue_path = movie_queue(app.clone()).boxed();
    let movie_queue_show_path = movie_queue_show(app.clone()).boxed();
    let plex_webhook_path = plex_webhook(app.clone()).boxed();
    let list_path = frontpage_path
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
        .or(movie_queue_show_path)
        .or(plex_webhook_path);
    let auth_url_path = trakt_auth_url(app.clone()).boxed();
    let trakt_callback_path = trakt_callback(app.clone()).boxed();
    let refresh_auth_path = refresh_auth(app.clone()).boxed();
    let trakt_cal_path = trakt_cal(app.clone()).boxed();
    let trakt_watchlist_path = trakt_watchlist(app.clone()).boxed();
    let trakt_watchlist_action_path = trakt_watchlist_action(app.clone()).boxed();
    let trakt_watched_seasons_path = trakt_watched_seasons(app.clone()).boxed();
    let trakt_watched_list_path = trakt_watched_list(app.clone()).boxed();
    let trakt_watched_action_path = trakt_watched_action(app.clone()).boxed();
    let trakt_path = auth_url_path
        .or(trakt_callback_path)
        .or(refresh_auth_path)
        .or(trakt_cal_path)
        .or(trakt_watchlist_path)
        .or(trakt_watchlist_action_path)
        .or(trakt_watched_seasons_path)
        .or(trakt_watched_list_path)
        .or(trakt_watched_action_path)
        .boxed();

    list_path.or(trakt_path).boxed()
}

async fn run_app(config: Config, pool: PgPool, trakt: TraktConnection) -> Result<(), Error> {
    let port = config.port;
    let app = AppState {
        config,
        db: pool,
        trakt,
        hbr: Arc::new(get_templates()?),
    };

    let (spec, full_path) = openapi::spec()
        .info(Info {
            title: "Movie Queue WebApp".into(),
            description: "Web Frontend for Movie Queue".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            ..Info::default()
        })
        .build(|| get_full_path(&app));
    let spec = Arc::new(spec);
    let spec_json_path = rweb::path!("list" / "openapi" / "json")
        .and(rweb::path::end())
        .map({
            let spec = spec.clone();
            move || rweb::reply::json(spec.as_ref())
        });

    let spec_yaml = serde_yaml::to_string(spec.as_ref())?;
    let spec_yaml_path = rweb::path!("list" / "openapi" / "yaml")
        .and(rweb::path::end())
        .map(move || {
            let reply = rweb::reply::html(spec_yaml.clone());
            rweb::reply::with_header(reply, CONTENT_TYPE, "text/yaml")
        });

    let routes = full_path
        .or(spec_json_path)
        .or(spec_yaml_path)
        .recover(error_response);
    let addr: SocketAddr = format!("127.0.0.1:{}", port).parse()?;
    rweb::serve(routes).bind(addr).await;
    Ok(())
}
