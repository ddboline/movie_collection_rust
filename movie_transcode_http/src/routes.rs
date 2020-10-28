#![allow(clippy::needless_pass_by_value)]

use actix_web::{
    web::{Data, Json, Path, Query},
    HttpResponse,
};
use anyhow::format_err;
use chrono::{DateTime, Utc};
use maplit::hashmap;
use serde::{Deserialize, Serialize};
use stack_string::StackString;
use std::{os::unix::fs::symlink, path, path::PathBuf};
use tokio::{fs::remove_file, task::spawn_blocking};

use movie_collection_lib::{
    imdb_episodes::ImdbEpisodes,
    imdb_ratings::ImdbRatings,
    make_list::FileLists,
    make_queue::movie_queue_http,
    movie_collection::{find_new_episodes_http_worker, LastModifiedResponse, MovieCollection},
    movie_queue::{MovieQueueDB, MovieQueueResult},
    pgpool::PgPool,
    trakt_utils::{
        get_watchlist_shows_db_map, trakt_cal_http_worker, trakt_watched_seasons_worker,
        tvshows_worker, watch_list_http_worker, watched_action_http_worker,
        watchlist_action_worker, watchlist_worker,
    },
    transcode_service::{transcode_status, TranscodeService, TranscodeServiceRequest},
    tv_show_source::TvShowSource,
    utils::HBR,
};

use super::{
    app::{AppState, CONFIG},
    errors::ServiceError as Error,
    logged_user::LoggedUser,
    requests::{
        ImdbRatingsSetSourceRequest, ImdbShowRequest, MovieCollectionUpdateRequest,
        MovieQueueRequest, MovieQueueUpdateRequest, ParseImdbRequest, WatchlistActionRequest,
    },
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

pub async fn frontpage(_: LoggedUser) -> HttpResult {
    form_http_response(HBR.render("index.html", &hashmap! {"BODY" => ""})?)
}

#[derive(Serialize, Deserialize)]
pub struct FindNewEpisodeRequest {
    pub source: Option<TvShowSource>,
    pub shows: Option<StackString>,
}

pub async fn find_new_episodes(
    query: Query<FindNewEpisodeRequest>,
    _: LoggedUser,
    state: Data<AppState>,
) -> HttpResult {
    let req = query.into_inner();
    let entries = find_new_episodes_http_worker(&CONFIG, &state.db, req.shows, req.source).await?;
    form_http_response(new_episode_worker(&entries).into())
}

fn new_episode_worker(entries: &[StackString]) -> StackString {
    let previous = r#"
        <a href="javascript:updateMainArticle('/list/tvshows')">Go Back</a><br>
        <input type="button" name="list_cal" value="TVCalendar" onclick="updateMainArticle('/list/cal');"/>
        <input type="button" name="list_cal" value="NetflixCalendar" onclick="updateMainArticle('/list/cal?source=netflix');"/>
        <input type="button" name="list_cal" value="AmazonCalendar" onclick="updateMainArticle('/list/cal?source=amazon');"/>
        <input type="button" name="list_cal" value="HuluCalendar" onclick="updateMainArticle('/list/cal?source=hulu');"/><br>
        <button name="remcomout" id="remcomoutput"> &nbsp; </button>
    "#;
    format!(
        r#"{}<table border="0">{}</table>"#,
        previous,
        entries.join("")
    )
    .into()
}

pub async fn tvshows(_: LoggedUser, state: Data<AppState>) -> HttpResult {
    let shows = MovieCollection::with_pool(&CONFIG, &state.db)?
        .print_tv_shows()
        .await?;
    let tvshowsmap = get_watchlist_shows_db_map(&state.db).await?;
    let entries = tvshows_worker(tvshowsmap, shows)?;
    form_http_response(entries.into())
}

pub async fn user(user: LoggedUser) -> HttpResult {
    to_json(user)
}

pub async fn movie_queue_delete(
    path: Path<StackString>,
    _: LoggedUser,
    state: Data<AppState>,
) -> HttpResult {
    let path = path.into_inner();
    if std::path::Path::new(path.as_str()).exists() {
        MovieQueueDB::with_pool(&CONFIG, &state.db)
            .remove_from_queue_by_path(&path)
            .await?;
    }
    form_http_response(path.into())
}

pub async fn movie_queue_play(idx: Path<i32>, _: LoggedUser, state: Data<AppState>) -> HttpResult {
    let idx = idx.into_inner();

    let movie_path = MovieCollection::with_pool(&CONFIG, &state.db)?
        .get_collection_path(idx)
        .await?;
    let movie_path = std::path::Path::new(movie_path.as_str());
    let body = play_worker(&movie_path)?;
    form_http_response(body)
}

fn play_worker(full_path: &path::Path) -> Result<String, Error> {
    let file_name = full_path
        .file_name()
        .ok_or_else(|| format_err!("Invalid path"))?
        .to_string_lossy();
    let url = format!("/videos/partial/{}", file_name);

    let body = format!(
        r#"{}<br>
            <video width="720" controls>
            <source src="{}" type="video/mp4">
            Your browser does not support HTML5 video.
            </video>
        "#,
        file_name, url
    );

    let partial_path =
        std::path::Path::new("/var/www/html/videos/partial").join(file_name.as_ref());
    if partial_path.exists() {
        std::fs::remove_file(&partial_path)?;
    }

    symlink(&full_path, &partial_path)?;
    Ok(body)
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

pub async fn imdb_show(
    path: Path<StackString>,
    query: Query<ParseImdbRequest>,
    _: LoggedUser,
    state: Data<AppState>,
) -> HttpResult {
    let show = path.into_inner();
    let query = query.into_inner();

    let req = ImdbShowRequest { show, query };
    let body = req.handle(&state.db, &CONFIG).await?;
    form_http_response(body.into())
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

pub async fn imdb_ratings_set_source(
    query: Query<ImdbRatingsSetSourceRequest>,
    _: LoggedUser,
    state: Data<AppState>,
) -> HttpResult {
    let query = query.into_inner();
    query.handle(&state.db).await?;
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

pub async fn trakt_auth_url(_: LoggedUser, state: Data<AppState>) -> HttpResult {
    state.trakt.init().await;
    let url = state.trakt.get_auth_url().await?;
    form_http_response(url.to_string())
}

#[derive(Serialize, Deserialize)]
pub struct TraktCallbackRequest {
    pub code: StackString,
    pub state: StackString,
}

pub async fn trakt_callback(
    query: Query<TraktCallbackRequest>,
    _: LoggedUser,
    state: Data<AppState>,
) -> HttpResult {
    state.trakt.init().await;
    state
        .trakt
        .exchange_code_for_auth_token(query.code.as_str(), query.state.as_str())
        .await?;
    let body = r#"
        <title>Trakt auth code received!</title>
        This window can be closed.
        <script language="JavaScript" type="text/javascript">window.close()</script>"#;
    form_http_response(body.to_string())
}

pub async fn refresh_auth(_: LoggedUser, state: Data<AppState>) -> HttpResult {
    state.trakt.init().await;
    state.trakt.exchange_refresh_token().await?;
    form_http_response("finished".to_string())
}

pub async fn trakt_cal(_: LoggedUser, state: Data<AppState>) -> HttpResult {
    let entries = trakt_cal_http_worker(&state.trakt, &state.db).await?;
    form_http_response(trakt_cal_worker(&entries).into())
}

fn trakt_cal_worker(entries: &[StackString]) -> StackString {
    let previous = r#"<a href="javascript:updateMainArticle('/list/tvshows')">Go Back</a><br>"#;
    format!(
        r#"{}<table border="0">{}</table>"#,
        previous,
        entries.join("")
    )
    .into()
}

pub async fn trakt_watchlist(_: LoggedUser, state: Data<AppState>) -> HttpResult {
    let shows = get_watchlist_shows_db_map(&state.db).await?;
    let entries = watchlist_worker(shows);
    form_http_response(entries)
}

pub async fn trakt_watchlist_action(
    path: Path<(StackString, StackString)>,
    _: LoggedUser,
    state: Data<AppState>,
) -> HttpResult {
    let (action, imdb_url) = path.into_inner();
    let action = action.parse().expect("impossible");

    let req = WatchlistActionRequest { action, imdb_url };
    let imdb_url = req.handle(&state.db, &state.trakt).await?;
    let body = watchlist_action_worker(&state.trakt, action, &imdb_url).await?;
    form_http_response(body)
}

pub async fn trakt_watched_seasons(
    path: Path<StackString>,
    _: LoggedUser,
    state: Data<AppState>,
) -> HttpResult {
    let imdb_url = path.into_inner();
    let show_opt = ImdbRatings::get_show_by_link(&imdb_url, &state.db)
        .await?
        .map(|sh| (imdb_url, sh));
    let empty = || ("".into(), "".into(), "".into());
    let (imdb_url, show, link) =
        show_opt.map_or_else(empty, |(imdb_url, t)| (imdb_url, t.show, t.link));

    let entries = if &show == "" {
        Vec::new()
    } else {
        MovieCollection::with_pool(&CONFIG, &state.db)?
            .print_imdb_all_seasons(&show)
            .await?
    };

    let entries = trakt_watched_seasons_worker(&link, &imdb_url, &entries)?;
    form_http_response(entries.into())
}

pub async fn trakt_watched_list(
    path: Path<(StackString, i32)>,
    _: LoggedUser,
    state: Data<AppState>,
) -> HttpResult {
    let (imdb_url, season) = path.into_inner();

    let body = watch_list_http_worker(&CONFIG, &state.db, &imdb_url, season).await?;
    form_http_response(body.into())
}

pub async fn trakt_watched_action(
    path: Path<(StackString, StackString, i32, i32)>,
    _: LoggedUser,
    state: Data<AppState>,
) -> HttpResult {
    let (action, imdb_url, season, episode) = path.into_inner();
    let body = watched_action_http_worker(
        &CONFIG,
        &state.trakt,
        &state.db,
        action.parse().expect("impossible"),
        &imdb_url,
        season,
        episode,
    )
    .await?;
    form_http_response(body.into())
}

pub async fn movie_queue_transcode_status(_: LoggedUser) -> HttpResult {
    let task = spawn_blocking(move || FileLists::get_file_lists(&CONFIG));
    let status = transcode_status(&CONFIG).await?;
    let file_lists = task.await.unwrap()?;
    form_http_response(status.get_html(&file_lists, &CONFIG).join(""))
}

pub async fn movie_queue_transcode_file(path: Path<StackString>, _: LoggedUser) -> HttpResult {
    let filename = path.into_inner();
    let transcode_service = TranscodeService::new(CONFIG.clone(), &CONFIG.transcode_queue);
    let input_path = CONFIG
        .home_dir
        .join("Documents")
        .join("movies")
        .join(&filename);
    let req = TranscodeServiceRequest::create_transcode_request(&CONFIG, &input_path)?;
    transcode_service.publish_transcode_job(&req).await?;
    form_http_response("".to_string())
}

pub async fn movie_queue_remcom_file(path: Path<StackString>, _: LoggedUser) -> HttpResult {
    let filename = path.into_inner();
    let transcode_service = TranscodeService::new(CONFIG.clone(), &CONFIG.remcom_queue);
    let input_path = CONFIG
        .home_dir
        .join("Documents")
        .join("movies")
        .join(&filename);
    let directory: Option<PathBuf> = None;
    let req =
        TranscodeServiceRequest::create_remcom_request(&CONFIG, &input_path, directory, false)
            .await?;
    transcode_service.publish_transcode_job(&req).await?;
    form_http_response("".to_string())
}

pub async fn movie_queue_remcom_directory_file(
    path: Path<(StackString, StackString)>,
    _: LoggedUser,
) -> HttpResult {
    let (directory, filename) = path.into_inner();
    let transcode_service = TranscodeService::new(CONFIG.clone(), &CONFIG.remcom_queue);
    let input_path = CONFIG
        .home_dir
        .join("Documents")
        .join("movies")
        .join(&filename);
    let req = TranscodeServiceRequest::create_remcom_request(
        &CONFIG,
        &input_path,
        Some(directory),
        false,
    )
    .await?;
    transcode_service.publish_transcode_job(&req).await?;
    form_http_response("".to_string())
}

pub async fn movie_queue_transcode_cleanup(path: Path<StackString>, _: LoggedUser) -> HttpResult {
    let path = path.into_inner();
    let movie_path = CONFIG.home_dir.join("Documents").join("movies").join(&path);
    let tmp_path = CONFIG.home_dir.join("tmp_avi").join(&path);
    if movie_path.exists() {
        remove_file(&movie_path).await?;
        form_http_response(format!("Removed {}", movie_path.to_string_lossy()))
    } else if tmp_path.exists() {
        remove_file(&tmp_path).await?;
        form_http_response(format!("Removed {}", tmp_path.to_string_lossy()))
    } else {
        form_http_response(format!("File not found {}", path))
    }
}

async fn transcode_worker(
    directory: Option<&path::Path>,
    entries: &[MovieQueueResult],
) -> HttpResult {
    let remcom_service = TranscodeService::new(CONFIG.clone(), &CONFIG.remcom_queue);
    let mut output = Vec::new();
    for entry in entries {
        let payload = TranscodeServiceRequest::create_remcom_request(
            &CONFIG,
            &path::Path::new(entry.path.as_str()),
            directory,
            false,
        )
        .await?;
        remcom_service.publish_transcode_job(&payload).await?;
        output.push(format!("{:?}", payload));
    }
    form_http_response(output.join(""))
}

pub async fn movie_queue_transcode(
    path: Path<StackString>,
    _: LoggedUser,
    state: Data<AppState>,
) -> HttpResult {
    let path = path.into_inner();
    let patterns = vec![path];

    let req = MovieQueueRequest { patterns };
    let (entries, _) = req.handle(&state.db, &CONFIG).await?;
    transcode_worker(None, &entries).await
}

pub async fn movie_queue_transcode_directory(
    path: Path<(StackString, StackString)>,
    _: LoggedUser,
    state: Data<AppState>,
) -> HttpResult {
    let (directory, file) = path.into_inner();
    let patterns = vec![file];

    let req = MovieQueueRequest { patterns };
    let (entries, _) = req.handle(&state.db, &CONFIG).await?;
    transcode_worker(Some(&path::Path::new(directory.as_str())), &entries).await
}
