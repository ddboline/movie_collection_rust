#![allow(clippy::needless_pass_by_value)]

use anyhow::Error;
use async_graphql::{
    dataloader::DataLoader,
    http::{playground_source, GraphQLPlaygroundConfig},
    EmptyMutation, EmptySubscription, Schema,
};
use async_graphql_warp::GraphQLResponse;
use rweb::{
    filters::BoxedFilter,
    http::{header::CONTENT_TYPE, Response as HttpResponse},
    openapi::{self, Info},
    Filter, Reply,
};
use stack_string::format_sstr;
use std::{convert::Infallible, net::SocketAddr, sync::Arc, time::Duration};
use tokio::{
    fs::{create_dir, remove_dir_all},
    time::interval,
};

use movie_collection_lib::{config::Config, pgpool::PgPool, trakt_connection::TraktConnection};

use super::{
    errors::error_response,
    graphql::{ItemLoader, QueryRoot},
    logged_user::{fill_from_db, get_secrets},
    movie_queue_routes::{
        find_new_episodes, frontpage, imdb_episodes_route, imdb_episodes_update,
        imdb_ratings_route, imdb_ratings_set_source, imdb_ratings_update, imdb_show,
        last_modified_route, movie_collection_route, movie_collection_update, movie_queue,
        movie_queue_delete, movie_queue_extract_subtitle, movie_queue_play,
        movie_queue_remcom_directory_file, movie_queue_remcom_file, movie_queue_route,
        movie_queue_show, movie_queue_transcode, movie_queue_transcode_cleanup,
        movie_queue_transcode_directory, movie_queue_transcode_file, movie_queue_transcode_status,
        movie_queue_transcode_status_file_list, movie_queue_transcode_status_procs,
        movie_queue_update, music_collection, music_collection_update, plex_detail, plex_events,
        plex_events_update, plex_filename, plex_filename_update, plex_list, plex_metadata,
        plex_metadata_update, plex_webhook, refresh_auth, scripts_js, trakt_auth_url, trakt_cal,
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

/// # Errors
/// Return error if app fails to start
pub async fn start_app() -> Result<(), Error> {
    async fn update_db(pool: PgPool) {
        let mut i = interval(Duration::from_secs(60));
        loop {
            fill_from_db(&pool).await.unwrap_or(());
            i.tick().await;
        }
    }
    let config = Config::with_config()?;
    get_secrets(&config.secret_path, &config.jwt_secret_path).await?;

    if let Some(partial_path) = &config.video_playback_path {
        let partial_path = partial_path.join("videos").join("partial");
        if partial_path.exists() {
            remove_dir_all(&partial_path).await?;
            create_dir(&partial_path).await?;
        }
    }

    let pool = PgPool::new(&config.pgurl)?;
    let trakt = TraktConnection::new(config.clone());

    tokio::task::spawn(update_db(pool.clone()));

    run_app(config, pool, trakt).await
}

fn get_full_path(app: &AppState) -> BoxedFilter<(impl Reply,)> {
    let frontpage_path = frontpage().boxed();
    let scripts_js_path = scripts_js().boxed();
    let find_new_episodes_path = find_new_episodes(app.clone()).boxed();
    let tvshows_path = tvshows(app.clone()).boxed();
    let movie_queue_delete_path = movie_queue_delete(app.clone()).boxed();
    let movie_queue_transcode_status_path = movie_queue_transcode_status(app.clone()).boxed();
    let movie_queue_transcode_status_file_list_path =
        movie_queue_transcode_status_file_list(app.clone()).boxed();
    let movie_queue_transcode_status_procs_path =
        movie_queue_transcode_status_procs(app.clone()).boxed();
    let movie_queue_transcode_file_path = movie_queue_transcode_file(app.clone()).boxed();
    let movie_queue_remcom_file_path = movie_queue_remcom_file(app.clone()).boxed();
    let movie_queue_remcom_directory_file_path =
        movie_queue_remcom_directory_file(app.clone()).boxed();
    let movie_queue_transcode_path = movie_queue_transcode(app.clone()).boxed();
    let movie_queue_transcode_directory_path = movie_queue_transcode_directory(app.clone()).boxed();
    let movie_queue_transcode_cleanup_path = movie_queue_transcode_cleanup(app.clone()).boxed();
    let transcode_path = movie_queue_transcode_status_path
        .or(movie_queue_transcode_status_file_list_path)
        .or(movie_queue_transcode_status_procs_path)
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
    let plex_events_path = plex_events(app.clone()).boxed();
    let plex_events_update_path = plex_events_update(app.clone()).boxed();
    let plex_list_path = plex_list(app.clone()).boxed();
    let plex_detail_path = plex_detail(app.clone()).boxed();
    let plex_filename_path = plex_filename(app.clone()).boxed();
    let plex_filename_update_path = plex_filename_update(app.clone()).boxed();
    let plex_metadata_path = plex_metadata(app.clone()).boxed();
    let plex_metadata_update_path = plex_metadata_update(app.clone()).boxed();
    let music_collection_path = music_collection(app.clone()).boxed();
    let music_collection_update_path = music_collection_update(app.clone()).boxed();
    let movie_queue_extract_subtitle_path = movie_queue_extract_subtitle(app.clone()).boxed();

    let list_path = frontpage_path
        .or(scripts_js_path)
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
        .or(plex_webhook_path)
        .or(plex_events_path)
        .or(plex_events_update_path)
        .or(plex_list_path)
        .or(plex_detail_path)
        .or(plex_filename_path)
        .or(plex_filename_update_path)
        .or(plex_metadata_path)
        .or(plex_metadata_update_path)
        .or(music_collection_path)
        .or(music_collection_update_path)
        .or(movie_queue_extract_subtitle_path);
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
    let schema = Schema::build(QueryRoot, EmptyMutation, EmptySubscription)
        .data(DataLoader::new(
            ItemLoader::new(pool.clone()),
            tokio::task::spawn,
        ))
        .finish();
    let graphql_post = rweb::path!("list" / "graphql" / "graphql")
        .and(async_graphql_warp::graphql(schema))
        .and_then(
            |(schema, request): (
                Schema<QueryRoot, EmptyMutation, EmptySubscription>,
                async_graphql::Request,
            )| async move {
                Ok::<_, Infallible>(GraphQLResponse::from(schema.execute(request).await))
            },
        );
    let graphql_playground = rweb::path!("list" / "graphql" / "playground")
        .and(rweb::path::end())
        .and(rweb::get())
        .map(|| {
            HttpResponse::builder()
                .header("content-type", "text/html")
                .body(playground_source(GraphQLPlaygroundConfig::new("/")))
        })
        .boxed();

    let port = config.port;
    let app = AppState {
        config,
        db: pool,
        trakt,
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
        .or(graphql_playground)
        .or(graphql_post)
        .recover(error_response);
    let host = &app.config.host;
    let addr: SocketAddr = format_sstr!("{host}:{port}").parse()?;
    rweb::serve(routes).bind(addr).await;
    Ok(())
}
