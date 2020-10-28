#![allow(clippy::needless_pass_by_value)]

use actix_web::{
    web::{Data, Json, Path, Query},
    HttpResponse,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use stack_string::StackString;

use movie_collection_lib::{
    imdb_episodes::ImdbEpisodes,
    imdb_ratings::ImdbRatings,
    make_queue::movie_queue_http,
    movie_collection::{LastModifiedResponse, MovieCollection},
    movie_queue::{MovieQueueDB, MovieQueueResult},
    pgpool::PgPool,
};

use super::{
    app::{AppState, CONFIG},
    errors::ServiceError as Error,
    logged_user::LoggedUser,
    requests::{MovieCollectionUpdateRequest, MovieQueueRequest, MovieQueueUpdateRequest},
};

pub type HttpResult = Result<HttpResponse, Error>;

fn form_http_response(body: String) -> HttpResult {
    Ok(HttpResponse::Ok()
        .content_type("text/html; charset=utf-8")
        .body(body))
}

fn to_json(js: impl Serialize) -> HttpResult {
    Ok(HttpResponse::Ok().json(js))
}

#[derive(Serialize, Deserialize, Clone, Copy)]
pub struct MovieQueueSyncRequest {
    pub start_timestamp: DateTime<Utc>,
}

pub async fn movie_queue_route(
    query: Query<MovieQueueSyncRequest>,
    _: LoggedUser,
    state: Data<AppState>,
) -> HttpResult {
    let req = query.into_inner();
    let mq = MovieQueueDB::with_pool(&CONFIG, &state.db);
    let queue = mq.get_queue_after_timestamp(req.start_timestamp).await?;
    to_json(queue)
}

pub async fn movie_queue_update(
    data: Json<MovieQueueUpdateRequest>,
    _: LoggedUser,
    state: Data<AppState>,
) -> HttpResult {
    let queue = data.into_inner();
    let req = queue;
    req.handle(&state.db, &CONFIG).await?;
    form_http_response("Success".to_string())
}

#[derive(Serialize, Deserialize, Clone, Copy)]
pub struct MovieCollectionSyncRequest {
    pub start_timestamp: DateTime<Utc>,
}

pub async fn movie_collection_route(
    query: Query<MovieCollectionSyncRequest>,
    _: LoggedUser,
    state: Data<AppState>,
) -> HttpResult {
    let req = query.into_inner();
    let mc = MovieCollection::with_pool(&CONFIG, &state.db)?;
    let x = mc
        .get_collection_after_timestamp(req.start_timestamp)
        .await?;
    to_json(x)
}

pub async fn movie_collection_update(
    data: Json<MovieCollectionUpdateRequest>,
    _: LoggedUser,
    state: Data<AppState>,
) -> HttpResult {
    let collection = data.into_inner();

    let req = collection;
    req.handle(&state.db, &CONFIG).await?;
    form_http_response("Success".to_string())
}

pub async fn last_modified_route(_: LoggedUser, state: Data<AppState>) -> HttpResult {
    let last_mods = LastModifiedResponse::get_last_modified(&state.db).await?;
    to_json(last_mods)
}

pub async fn movie_queue(_: LoggedUser, state: Data<AppState>) -> HttpResult {
    let req = MovieQueueRequest {
        patterns: Vec::new(),
    };
    let (queue, _) = req.handle(&state.db, &CONFIG).await?;
    let body = queue_body_resp(Vec::new(), queue, &state.db).await?;
    form_http_response(body.into())
}

pub async fn movie_queue_show(
    path: Path<StackString>,
    _: LoggedUser,
    state: Data<AppState>,
) -> HttpResult {
    let path = path.into_inner();
    let patterns = vec![path];

    let req = MovieQueueRequest { patterns };
    let (queue, patterns) = req.handle(&state.db, &CONFIG).await?;
    let body = queue_body_resp(patterns, queue, &state.db).await?;
    form_http_response(body.into())
}

async fn queue_body_resp(
    patterns: Vec<StackString>,
    queue: Vec<MovieQueueResult>,
    pool: &PgPool,
) -> Result<StackString, Error> {
    let entries = movie_queue_http(&CONFIG, &queue, pool).await?;
    let body = movie_queue_body(&patterns, &entries);
    Ok(body)
}

fn movie_queue_body(patterns: &[StackString], entries: &[StackString]) -> StackString {
    let previous = r#"<a href="javascript:updateMainArticle('/list/tvshows')">Go Back</a><br>"#;

    let watchlist_url = if patterns.is_empty() {
        "/list/trakt/watchlist".to_string()
    } else {
        format!("/list/trakt/watched/list/{}", patterns.join("_"))
    };

    let entries = format!(
        r#"{}<a href="javascript:updateMainArticle('{}')">Watch List</a><table border="0">{}</table>"#,
        previous,
        watchlist_url,
        entries.join("")
    );

    entries.into()
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy)]
pub struct ImdbEpisodesSyncRequest {
    pub start_timestamp: DateTime<Utc>,
}

pub async fn imdb_episodes_route(
    query: Query<ImdbEpisodesSyncRequest>,
    _: LoggedUser,
    state: Data<AppState>,
) -> HttpResult {
    let req = query.into_inner();
    let episodes =
        ImdbEpisodes::get_episodes_after_timestamp(req.start_timestamp, &state.db).await?;
    to_json(episodes)
}

#[derive(Serialize, Deserialize)]
pub struct ImdbEpisodesUpdateRequest {
    pub episodes: Vec<ImdbEpisodes>,
}

pub async fn imdb_episodes_update(
    data: Json<ImdbEpisodesUpdateRequest>,
    _: LoggedUser,
    state: Data<AppState>,
) -> HttpResult {
    ImdbEpisodes::update_episodes(&data.episodes, &state.db).await?;
    form_http_response("Success".to_string())
}

#[derive(Serialize, Deserialize, Clone, Copy)]
pub struct ImdbRatingsSyncRequest {
    pub start_timestamp: DateTime<Utc>,
}

pub async fn imdb_ratings_route(
    query: Query<ImdbRatingsSyncRequest>,
    _: LoggedUser,
    state: Data<AppState>,
) -> HttpResult {
    let req = query.into_inner();
    let shows = ImdbRatings::get_shows_after_timestamp(req.start_timestamp, &state.db).await?;
    to_json(shows)
}

#[derive(Serialize, Deserialize)]
pub struct ImdbRatingsUpdateRequest {
    pub shows: Vec<ImdbRatings>,
}

pub async fn imdb_ratings_update(
    data: Json<ImdbRatingsUpdateRequest>,
    _: LoggedUser,
    state: Data<AppState>,
) -> HttpResult {
    ImdbRatings::update_ratings(&data.shows, &state.db).await?;
    form_http_response("Success".to_string())
}
