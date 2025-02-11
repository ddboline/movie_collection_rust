#![allow(clippy::needless_pass_by_value)]

use anyhow::format_err;
use bytes::Buf;
use futures::TryStreamExt;
use log::error;
use rweb::{delete, get, multipart::FormData, post, Json, Query, Rejection, Schema};
use rweb_helper::{
    derive_rweb_schema, html_response::HtmlResponse as HtmlBase,
    json_response::JsonResponse as JsonBase, DateTimeType, RwebResponse, UuidWrapper,
};
use serde::{Deserialize, Serialize};
use stack_string::{format_sstr, StackString};
use std::{
    borrow::Borrow,
    collections::HashMap,
    fmt::Write,
    hash::{Hash, Hasher},
    path,
    path::PathBuf,
};
use stdout_channel::{MockStdout, StdoutChannel};
use time::OffsetDateTime;
use tokio::fs::remove_file;
use tokio_stream::StreamExt;

use movie_collection_lib::{
    config::Config,
    date_time_wrapper::DateTimeWrapper,
    imdb_episodes::ImdbEpisodes,
    imdb_ratings::ImdbRatings,
    make_list::FileLists,
    mkv_utils::MkvTrack,
    movie_collection::{LastModifiedResponse, MovieCollection, TvShowsResult},
    movie_queue::{MovieQueueDB, MovieQueueResult},
    music_collection::MusicCollection,
    pgpool::PgPool,
    plex_events::{PlexEvent, PlexFilename, PlexMetadata},
    trakt_connection::TraktConnection,
    trakt_utils::{
        get_watchlist_shows_db_map, watchlist_add, watchlist_rm, TraktActions, WatchListShow,
        WatchedEpisode, WatchedMovie,
    },
    transcode_service::{transcode_status, TranscodeService, TranscodeServiceRequest},
    tv_show_source::TvShowSource,
};
use std::convert::Infallible;

use crate::{
    errors::ServiceError as Error,
    logged_user::LoggedUser,
    movie_queue_app::AppState,
    movie_queue_elements::{
        find_new_episodes_body, index_body, local_file_body, movie_queue_body, play_worker_body,
        plex_body, plex_detail_body, procs_html_body, trakt_cal_http_body,
        trakt_watched_seasons_body, transcode_get_html_body, tvshows_body, watch_list_http_body,
        watchlist_body,
    },
    movie_queue_requests::{
        ImdbEpisodesUpdateRequest, ImdbRatingsSetSourceRequest, ImdbRatingsUpdateRequest,
        ImdbSeasonsRequest, ImdbShowRequest, MovieCollectionSyncRequest,
        MovieCollectionUpdateRequest, MovieQueueRequest, MovieQueueSyncRequest,
        MovieQueueUpdateRequest, ParseImdbRequest,
    },
    ImdbEpisodesWrapper, ImdbRatingsWrapper, LastModifiedResponseWrapper,
    MovieCollectionRowWrapper, MovieQueueRowWrapper, MusicCollectionWrapper, OrderByWrapper,
    PlexEventRequest, PlexEventWrapper, PlexFilenameRequest, PlexFilenameWrapper,
    PlexMetadataWrapper, TraktActionsWrapper, TraktWatchlistRequest, TvShowSourceWrapper,
};

pub type WarpResult<T> = Result<T, Rejection>;
pub type HttpResult<T> = Result<T, Error>;

#[derive(RwebResponse)]
#[response(description = "Scripts", content = "js")]
struct JsScriptsResponse(HtmlBase<&'static str, Infallible>);

#[get("/list/scripts.js")]
pub async fn scripts_js() -> WarpResult<JsScriptsResponse> {
    Ok(HtmlBase::new(include_str!("../../templates/scripts.js")).into())
}

#[derive(RwebResponse)]
#[response(description = "Movie Queue", content = "html")]
struct MovieQueueResponse(HtmlBase<StackString, Error>);

#[derive(Serialize, Deserialize, Debug, Schema)]
#[schema(component = "FullQueueRequest")]
pub struct FullQueueRequest {
    #[schema(description = "Search String")]
    pub q: Option<StackString>,
    #[schema(description = "Offset")]
    pub offset: Option<usize>,
    #[schema(description = "Limit")]
    pub limit: Option<usize>,
    #[schema(description = "Order By (asc/desc)")]
    pub order_by: Option<OrderByWrapper>,
}

#[get("/list/full_queue")]
pub async fn movie_queue(
    #[filter = "LoggedUser::filter"] user: LoggedUser,
    #[data] state: AppState,
    query: Query<FullQueueRequest>,
) -> WarpResult<MovieQueueResponse> {
    let query = query.into_inner();
    let task = user
        .store_url_task(state.trakt.get_client(), &state.config, "/list/full_queue")
        .await;
    let mut patterns = Vec::new();
    if let Some(q) = &query.q {
        patterns.push(q.clone());
    }
    let req = MovieQueueRequest {
        patterns,
        offset: query.offset,
        limit: query.limit,
        order_by: query.order_by.map(Into::into),
    };
    let (queue, _) = req.process(&state.db, &state.config).await?;

    let body = movie_queue_body(
        &state.config,
        &state.db,
        Vec::new(),
        queue,
        query.q,
        query.offset,
        query.limit,
        query.order_by.map(Into::into),
    )
    .await
    .map_err(Into::<Error>::into)?
    .into();
    task.await.ok();
    Ok(HtmlBase::new(body).into())
}

#[get("/list/queue/{path}")]
pub async fn movie_queue_show(
    path: StackString,
    #[filter = "LoggedUser::filter"] user: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<MovieQueueResponse> {
    let url = format_sstr!("/list/queue/{path}");
    let task = user
        .store_url_task(state.trakt.get_client(), &state.config, &url)
        .await;
    let patterns = vec![path];

    let req = MovieQueueRequest {
        patterns,
        ..MovieQueueRequest::default()
    };
    let (queue, patterns) = req.process(&state.db, &state.config).await?;
    let body = movie_queue_body(
        &state.config,
        &state.db,
        patterns,
        queue,
        None,
        None,
        None,
        None,
    )
    .await
    .map_err(Into::<Error>::into)?
    .into();
    task.await.ok();
    Ok(HtmlBase::new(body).into())
}

#[derive(RwebResponse)]
#[response(description = "Delete Queue Entry", content = "html")]
struct DeleteMovieQueueResponse(HtmlBase<StackString, Error>);

#[delete("/list/delete/{path}")]
pub async fn movie_queue_delete(
    path: StackString,
    #[filter = "LoggedUser::filter"] user: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<DeleteMovieQueueResponse> {
    let url = format_sstr!("/list/delete/{path}");
    let task = user
        .store_url_task(state.trakt.get_client(), &state.config, &url)
        .await;
    let mock_stdout = MockStdout::new();
    let stdout = StdoutChannel::with_mock_stdout(mock_stdout.clone(), mock_stdout.clone());

    let collection = MovieCollection::new(&state.config, &state.db, &stdout);

    let mut output = StackString::new();
    if path::Path::new(path.as_str()).exists()
        || collection
            .get_collection_index(&path)
            .await
            .map_err(Into::<Error>::into)?
            .is_some()
    {
        MovieQueueDB::new(&state.config, &state.db, &stdout)
            .remove_from_queue_by_path(&path)
            .await
            .map_err(Into::<Error>::into)?;
        write!(&mut output, "success {path}").map_err(Into::<Error>::into)?;
    } else {
        write!(&mut output, "failure").map_err(Into::<Error>::into)?;
    }
    task.await.ok();
    Ok(HtmlBase::new(output).into())
}

async fn transcode_worker(
    config: &Config,
    directory: Option<&path::Path>,
    entries: &[MovieQueueResult],
    pool: &PgPool,
) -> HttpResult<StackString> {
    let mock_stdout = MockStdout::new();
    let stdout = StdoutChannel::with_mock_stdout(mock_stdout.clone(), mock_stdout.clone());

    let remcom_service = TranscodeService::new(config, &config.remcom_queue, pool, &stdout);
    let mut output = Vec::new();
    for entry in entries {
        let payload = TranscodeServiceRequest::create_remcom_request(
            config,
            &path::Path::new(entry.path.as_str()),
            directory,
            false,
        )
        .await?;
        remcom_service
            .publish_transcode_job(&payload, |_| async move { Ok(()) })
            .await?;
        output.push(format_sstr!("{payload:?}"));
        output.push(payload.publish_to_cli(config).await?);
    }
    Ok(output.join("").into())
}

#[derive(RwebResponse)]
#[response(
    description = "Transcode Queue Item",
    content = "html",
    status = "CREATED"
)]
struct TranscodeQueueResponse(HtmlBase<StackString, Error>);

#[post("/list/transcode/queue/{path}")]
pub async fn movie_queue_transcode(
    path: StackString,
    #[filter = "LoggedUser::filter"] user: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<TranscodeQueueResponse> {
    let url = format_sstr!("/list/transcode/queue/{path}");
    let task = user
        .store_url_task(state.trakt.get_client(), &state.config, &url)
        .await;
    let patterns = vec![path];

    let req = MovieQueueRequest {
        patterns,
        ..MovieQueueRequest::default()
    };
    let (entries, _) = req.process(&state.db, &state.config).await?;
    let body = transcode_worker(&state.config, None, &entries, &state.db).await?;
    task.await.ok();
    Ok(HtmlBase::new(body).into())
}

#[post("/list/transcode/queue/{directory}/{file}")]
pub async fn movie_queue_transcode_directory(
    directory: StackString,
    file: StackString,
    #[filter = "LoggedUser::filter"] user: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<TranscodeQueueResponse> {
    let url = format_sstr!("/list/transcode/queue/{directory}/{file}");
    let task = user
        .store_url_task(state.trakt.get_client(), &state.config, &url)
        .await;
    let patterns = vec![file];

    let req = MovieQueueRequest {
        patterns,
        ..MovieQueueRequest::default()
    };
    let (entries, _) = req.process(&state.db, &state.config).await?;
    let body = transcode_worker(
        &state.config,
        Some(path::Path::new(directory.as_str())),
        &entries,
        &state.db,
    )
    .await?;
    task.await.ok();
    Ok(HtmlBase::new(body).into())
}

#[derive(RwebResponse)]
#[response(description = "Play Queue Item", content = "html")]
struct PlayQueueResponse(HtmlBase<StackString, Error>);

#[get("/list/play/{idx}")]
pub async fn movie_queue_play(
    idx: UuidWrapper,
    #[filter = "LoggedUser::filter"] user: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<PlayQueueResponse> {
    let last_url = user
        .get_url(state.trakt.get_client(), &state.config)
        .await
        .map_err(Into::<Error>::into)?;
    let url = format_sstr!("/list/play/{idx}");
    let task = user
        .store_url_task(state.trakt.get_client(), &state.config, &url)
        .await;

    let mock_stdout = MockStdout::new();
    let stdout = StdoutChannel::with_mock_stdout(mock_stdout.clone(), mock_stdout);

    let movie_path = MovieCollection::new(&state.config, &state.db, &stdout)
        .get_collection_path(idx.into())
        .await
        .map_err(Into::<Error>::into)?;

    let movie_path = path::Path::new(movie_path.as_str());
    println!("movie_path {movie_path:?}");
    let body = play_worker_body(&state.config, movie_path, last_url)
        .map_err(Into::<Error>::into)?
        .into();
    task.await.ok();
    Ok(HtmlBase::new(body).into())
}

#[derive(RwebResponse)]
#[response(description = "List Imdb Show", content = "html")]
struct ListImdbResponse(HtmlBase<StackString, Error>);

#[get("/list/imdb/{show}")]
pub async fn imdb_show(
    show: StackString,
    query: Query<ParseImdbRequest>,
    #[filter = "LoggedUser::filter"] user: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<ListImdbResponse> {
    let url = format_sstr!("/list/imdb/{show}");
    let task = user
        .store_url_task(state.trakt.get_client(), &state.config, &url)
        .await;
    let query = query.into_inner();
    let req = ImdbShowRequest { show, query };
    let body = req.process(&state.db, &state.config).await?;
    task.await.ok();
    Ok(HtmlBase::new(body).into())
}

#[derive(Serialize, Deserialize, Schema)]
#[schema(component = "FindNewEpisodeRequest")]
pub struct FindNewEpisodeRequest {
    #[schema(description = "TV Show Source")]
    pub source: Option<TvShowSourceWrapper>,
    #[schema(description = "TV Show")]
    pub shows: Option<StackString>,
}

#[derive(RwebResponse)]
#[response(description = "List Calendar", content = "html")]
struct ListCalendarResponse(HtmlBase<StackString, Error>);

#[get("/list/cal")]
pub async fn find_new_episodes(
    query: Query<FindNewEpisodeRequest>,
    #[filter = "LoggedUser::filter"] user: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<ListCalendarResponse> {
    let query = query.into_inner();

    let task = user
        .store_url_task(state.trakt.get_client(), &state.config, "/list/cal")
        .await;

    let mock_stdout = MockStdout::new();
    let stdout = StdoutChannel::with_mock_stdout(mock_stdout.clone(), mock_stdout);

    let body = find_new_episodes_body(
        &state.config,
        &state.db,
        &stdout,
        query.shows,
        query.source.map(Into::into),
    )
    .await
    .map_err(Into::<Error>::into)?
    .into();

    task.await.ok();
    Ok(HtmlBase::new(body).into())
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ImdbEpisodesSyncRequest {
    pub start_timestamp: Option<DateTimeWrapper>,
    pub offset: Option<usize>,
    pub limit: Option<usize>,
}

derive_rweb_schema!(ImdbEpisodesSyncRequest, _ImdbEpisodesSyncRequest);

#[derive(Schema)]
#[allow(dead_code)]
struct _ImdbEpisodesSyncRequest {
    #[schema(description = "Start Timestamp")]
    pub start_timestamp: Option<DateTimeType>,
    #[schema(description = "Offset")]
    pub offset: Option<usize>,
    #[schema(description = "Limit")]
    pub limit: Option<usize>,
}

#[derive(Debug, Serialize, Deserialize, Schema)]
#[schema(component = "Pagination")]
struct Pagination {
    #[schema(description = "Total Number of Entries")]
    total: usize,
    #[schema(description = "Number of Entries to Skip")]
    offset: usize,
    #[schema(description = "Number of Entries Returned")]
    limit: usize,
}

#[derive(Debug, Serialize, Deserialize, Schema)]
#[schema(component = "PaginatedImdbEpisodes")]
struct PaginatedImdbEpisodes {
    pagination: Pagination,
    data: Vec<ImdbEpisodesWrapper>,
}

#[derive(RwebResponse)]
#[response(description = "List Imdb Episodes")]
struct ListImdbEpisodesResponse(JsonBase<PaginatedImdbEpisodes, Error>);

#[get("/list/imdb_episodes")]
pub async fn imdb_episodes_route(
    query: Query<ImdbEpisodesSyncRequest>,
    #[filter = "LoggedUser::filter"] user: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<ListImdbEpisodesResponse> {
    let query = query.into_inner();

    let offset = query.offset.unwrap_or(0);
    let limit = query.limit.unwrap_or(10);
    let start_timestamp: Option<OffsetDateTime> = query.start_timestamp.map(Into::into);

    let task = user
        .store_url_task(
            state.trakt.get_client(),
            &state.config,
            "/list/imdb_episodes",
        )
        .await;

    let total = ImdbEpisodes::get_total(&state.db, start_timestamp, None, None, None)
        .await
        .map_err(Into::<Error>::into)?;
    let pagination = Pagination {
        total,
        offset,
        limit,
    };

    let data: Vec<_> = ImdbEpisodes::get_episodes_after_timestamp(
        &state.db,
        start_timestamp,
        Some(offset),
        Some(limit),
    )
    .await
    .map_err(Into::<Error>::into)?
    .map_ok(Into::into)
    .try_collect()
    .await
    .map_err(Into::<Error>::into)?;
    task.await.ok();
    Ok(JsonBase::new(PaginatedImdbEpisodes { pagination, data }).into())
}

#[derive(RwebResponse)]
#[response(
    description = "Imdb Episodes Update",
    content = "html",
    status = "CREATED"
)]
struct ImdbEpisodesUpdateResponse(HtmlBase<&'static str, Error>);

#[post("/list/imdb_episodes")]
pub async fn imdb_episodes_update(
    episodes: Json<ImdbEpisodesUpdateRequest>,
    #[filter = "LoggedUser::filter"] user: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<ImdbEpisodesUpdateResponse> {
    let task = user
        .store_url_task(
            state.trakt.get_client(),
            &state.config,
            "/list/imdb_episodes",
        )
        .await;
    episodes.into_inner().run_update(&state.db).await?;
    task.await.ok();
    Ok(HtmlBase::new("Success").into())
}

#[derive(Serialize, Deserialize)]
pub struct ImdbRatingsSyncRequest {
    pub start_timestamp: Option<DateTimeWrapper>,
    pub offset: Option<usize>,
    pub limit: Option<usize>,
}

derive_rweb_schema!(ImdbRatingsSyncRequest, _ImdbEpisodesSyncRequest);

#[derive(Debug, Serialize, Deserialize, Schema)]
#[schema(component = "PaginatedImdbRatings")]
struct PaginatedImdbRatings {
    pagination: Pagination,
    data: Vec<ImdbRatingsWrapper>,
}

#[derive(RwebResponse)]
#[response(description = "List Imdb Shows")]
struct ListImdbShowsResponse(JsonBase<PaginatedImdbRatings, Error>);

#[get("/list/imdb_ratings")]
pub async fn imdb_ratings_route(
    query: Query<ImdbRatingsSyncRequest>,
    #[filter = "LoggedUser::filter"] user: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<ListImdbShowsResponse> {
    let query = query.into_inner();

    let task = user
        .store_url_task(
            state.trakt.get_client(),
            &state.config,
            "/list/imdb_ratings",
        )
        .await;
    let start_timestamp = query.start_timestamp.map(Into::into);
    let total = ImdbRatings::get_total(&state.db, start_timestamp)
        .await
        .map_err(Into::<Error>::into)?;
    let offset = query.offset.unwrap_or(0);
    let limit = query.limit.unwrap_or(total);
    let pagination = Pagination {
        total,
        offset,
        limit,
    };

    let data = ImdbRatings::get_shows_after_timestamp(
        &state.db,
        start_timestamp,
        Some(offset),
        Some(limit),
    )
    .await
    .map_err(Into::<Error>::into)?
    .map_ok(Into::into)
    .try_collect()
    .await
    .map_err(Into::<Error>::into)?;
    task.await.ok();
    Ok(JsonBase::new(PaginatedImdbRatings { pagination, data }).into())
}

#[derive(RwebResponse)]
#[response(
    description = "Update Imdb Shows",
    content = "html",
    status = "CREATED"
)]
struct UpdateImdbShowsResponse(HtmlBase<&'static str, Error>);

#[post("/list/imdb_ratings")]
pub async fn imdb_ratings_update(
    shows: Json<ImdbRatingsUpdateRequest>,
    #[filter = "LoggedUser::filter"] user: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<UpdateImdbShowsResponse> {
    let task = user
        .store_url_task(
            state.trakt.get_client(),
            &state.config,
            "/list/imdb_ratings",
        )
        .await;
    shows.into_inner().run_update(&state.db).await?;
    task.await.ok();
    Ok(HtmlBase::new("Success").into())
}

#[derive(RwebResponse)]
#[response(description = "Imdb Show Set Source", content = "html")]
struct ImdbSetSourceResponse(HtmlBase<&'static str, Error>);

#[get("/list/imdb_ratings/set_source")]
pub async fn imdb_ratings_set_source(
    query: Query<ImdbRatingsSetSourceRequest>,
    #[filter = "LoggedUser::filter"] user: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<ImdbSetSourceResponse> {
    let task = user
        .store_url_task(
            state.trakt.get_client(),
            &state.config,
            "/list/imdb_ratings/set_source",
        )
        .await;
    query.into_inner().set_source(&state.db).await?;
    task.await.ok();
    Ok(HtmlBase::new("Success").into())
}

#[derive(Debug, Serialize, Deserialize, Schema)]
#[schema(component = "PaginatedMovieQueueRow")]
struct PaginatedMovieQueueRow {
    pagination: Pagination,
    data: Vec<MovieQueueRowWrapper>,
}

#[derive(RwebResponse)]
#[response(description = "List Movie Queue Entries")]
struct ListMovieQueueResponse(JsonBase<PaginatedMovieQueueRow, Error>);

#[get("/list/movie_queue")]
pub async fn movie_queue_route(
    query: Query<MovieQueueSyncRequest>,
    #[filter = "LoggedUser::filter"] user: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<ListMovieQueueResponse> {
    let query = query.into_inner();

    let task = user
        .store_url_task(state.trakt.get_client(), &state.config, "/list/movie_queue")
        .await;
    let (total, data) = query.get_queue(&state.db, &state.config).await?;
    let offset = query.offset.unwrap_or(0);
    let limit = query.limit.unwrap_or(total);
    let pagination = Pagination {
        total,
        offset,
        limit,
    };
    let data = data.into_iter().map(Into::into).collect();
    task.await.ok();
    Ok(JsonBase::new(PaginatedMovieQueueRow { pagination, data }).into())
}

#[derive(RwebResponse)]
#[response(
    description = "Update Movie Queue Entries",
    content = "html",
    status = "CREATED"
)]
struct UpdateMovieQueueResponse(HtmlBase<&'static str, Error>);

#[post("/list/movie_queue")]
pub async fn movie_queue_update(
    queue: Json<MovieQueueUpdateRequest>,
    #[filter = "LoggedUser::filter"] user: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<UpdateMovieQueueResponse> {
    let task = user
        .store_url_task(state.trakt.get_client(), &state.config, "/list/movie_queue")
        .await;
    queue
        .into_inner()
        .run_update(&state.db, &state.config)
        .await?;
    task.await.ok();
    Ok(HtmlBase::new("Success").into())
}

#[derive(Debug, Serialize, Deserialize, Schema)]
#[schema(component = "PaginatedMovieCollectionRow")]
struct PaginatedMovieCollectionRow {
    pagination: Pagination,
    data: Vec<MovieCollectionRowWrapper>,
}

#[derive(RwebResponse)]
#[response(description = "List Movie Collection Entries")]
struct ListMovieCollectionResponse(JsonBase<PaginatedMovieCollectionRow, Error>);

#[get("/list/movie_collection")]
pub async fn movie_collection_route(
    query: Query<MovieCollectionSyncRequest>,
    #[filter = "LoggedUser::filter"] user: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<ListMovieCollectionResponse> {
    let task = user
        .store_url_task(state.trakt.get_client(), &state.config, "/list/movie_queue")
        .await;
    let query = query.into_inner();
    let (total, data) = query.get_collection(&state.db, &state.config).await?;
    let offset = query.offset.unwrap_or(0);
    let limit = query.limit.unwrap_or(total);
    let pagination = Pagination {
        total,
        offset,
        limit,
    };
    let data = data.into_iter().map(Into::into).collect();
    task.await.ok();
    Ok(JsonBase::new(PaginatedMovieCollectionRow { pagination, data }).into())
}

#[derive(RwebResponse)]
#[response(
    description = "Update Movie Collection Entries",
    content = "html",
    status = "CREATED"
)]
struct UpdateMovieCollectionResponse(HtmlBase<&'static str, Error>);

#[post("/list/movie_collection")]
pub async fn movie_collection_update(
    collection: Json<MovieCollectionUpdateRequest>,
    #[filter = "LoggedUser::filter"] user: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<UpdateMovieCollectionResponse> {
    let task = user
        .store_url_task(
            state.trakt.get_client(),
            &state.config,
            "/list/movie_collection",
        )
        .await;
    collection
        .into_inner()
        .run_update(&state.db, &state.config)
        .await?;
    task.await.ok();
    Ok(HtmlBase::new("Success").into())
}

#[derive(RwebResponse)]
#[response(description = "Database Entries Last Modified Time")]
struct ListLastModifiedResponse(JsonBase<Vec<LastModifiedResponseWrapper>, Error>);

#[get("/list/last_modified")]
pub async fn last_modified_route(
    #[filter = "LoggedUser::filter"] user: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<ListLastModifiedResponse> {
    let task = user
        .store_url_task(
            state.trakt.get_client(),
            &state.config,
            "/list/last_modified",
        )
        .await;
    let last_modified = LastModifiedResponse::get_last_modified(&state.db)
        .await
        .map_err(Into::<Error>::into)?
        .into_iter()
        .map(Into::into)
        .collect();
    task.await.ok();
    Ok(JsonBase::new(last_modified).into())
}

#[derive(RwebResponse)]
#[response(description = "Frontpage", content = "html")]
struct FrontpageResponse(HtmlBase<StackString, Error>);

#[get("/list/index.html")]
#[allow(clippy::unused_async)]
pub async fn frontpage(
    #[filter = "LoggedUser::filter"] _: LoggedUser,
) -> WarpResult<FrontpageResponse> {
    let body = index_body().map_err(Into::<Error>::into)?.into();
    Ok(HtmlBase::new(body).into())
}

pub type TvShowsMap = HashMap<StackString, (StackString, WatchListShow, Option<TvShowSource>)>;

#[derive(Debug, Default, Eq, Clone)]
pub struct ProcessShowItem {
    pub show: StackString,
    pub title: StackString,
    pub link: StackString,
    pub source: Option<TvShowSource>,
}

impl PartialEq for ProcessShowItem {
    fn eq(&self, other: &Self) -> bool {
        self.link == other.link
    }
}

impl Hash for ProcessShowItem {
    fn hash<H>(&self, state: &mut H)
    where
        H: Hasher,
    {
        self.link.hash(state);
    }
}

impl Borrow<str> for ProcessShowItem {
    fn borrow(&self) -> &str {
        self.link.as_str()
    }
}

impl From<TvShowsResult> for ProcessShowItem {
    fn from(item: TvShowsResult) -> Self {
        Self {
            show: item.show,
            title: item.title,
            link: item.link,
            source: item.source,
        }
    }
}

#[derive(RwebResponse)]
#[response(description = "List TvShows", content = "html")]
struct ListTvShowsResponse(HtmlBase<StackString, Error>);

#[get("/list/tvshows")]
pub async fn tvshows(
    #[filter = "LoggedUser::filter"] user: LoggedUser,
    #[data] state: AppState,
    query: Query<TraktWatchlistRequest>,
) -> WarpResult<ListTvShowsResponse> {
    let query = query.into_inner();
    let task = user
        .store_url_task(state.trakt.get_client(), &state.config, "/list/tvshows")
        .await;
    let mock_stdout = MockStdout::new();
    let stdout = StdoutChannel::with_mock_stdout(mock_stdout.clone(), mock_stdout.clone());

    let mc = MovieCollection::new(&state.config, &state.db, &stdout);
    let shows: Vec<_> = mc
        .print_tv_shows(
            query.query.as_ref().map(StackString::as_str),
            query.source.map(Into::into),
            None,
            None,
        )
        .await
        .map_err(Into::<Error>::into)?
        .try_collect()
        .await
        .map_err(Into::<Error>::into)?;
    let show_map = get_watchlist_shows_db_map(
        &state.db,
        query.query.as_ref().map(StackString::as_str),
        query.source.map(Into::into),
        None,
        None,
    )
    .await
    .map_err(Into::<Error>::into)?;
    let body = tvshows_body(
        show_map,
        shows,
        query.query.as_ref().map(StackString::as_str),
        query.source.map(Into::into),
        query.offset,
        query.limit,
    )
    .map_err(Into::<Error>::into)?
    .into();
    task.await.ok();
    Ok(HtmlBase::new(body).into())
}

#[derive(RwebResponse)]
#[response(description = "Logged in User")]
struct UserResponse(JsonBase<LoggedUser, Error>);

#[get("/list/user")]
pub async fn user(#[filter = "LoggedUser::filter"] user: LoggedUser) -> WarpResult<UserResponse> {
    Ok(JsonBase::new(user).into())
}

#[derive(RwebResponse)]
#[response(description = "Transcode Status", content = "html")]
struct TranscodeStatusResponse(HtmlBase<StackString, Error>);

#[get("/list/transcode/status")]
pub async fn movie_queue_transcode_status(
    #[filter = "LoggedUser::filter"] user: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<TranscodeStatusResponse> {
    let url_task = user
        .store_url_task(
            state.trakt.get_client(),
            &state.config,
            "/list/transcode/status",
        )
        .await;
    let status = transcode_status(&state.config)
        .await
        .map_err(Into::<Error>::into)?;
    let body = transcode_get_html_body(status)
        .map_err(Into::<Error>::into)?
        .into();
    url_task.await.ok();
    Ok(HtmlBase::new(body).into())
}

#[derive(RwebResponse)]
#[response(description = "Transcode Status File List", content = "html")]
struct TranscodeStatusFileListResponse(HtmlBase<StackString, Error>);

#[get("/list/transcode/status/file_list")]
pub async fn movie_queue_transcode_status_file_list(
    #[filter = "LoggedUser::filter"] _: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<TranscodeStatusFileListResponse> {
    let mock_stdout = MockStdout::new();
    let stdout = StdoutChannel::with_mock_stdout(mock_stdout.clone(), mock_stdout.clone());

    let file_lists = FileLists::get_file_lists(&state.config, Some(&state.db), &stdout)
        .await
        .map_err(Into::<Error>::into)?;
    let status = transcode_status(&state.config)
        .await
        .map_err(Into::<Error>::into)?;
    let body = if file_lists.local_file_list.is_empty() {
        StackString::new()
    } else {
        let proc_map = status.get_proc_map();
        local_file_body(file_lists, proc_map, state.config.clone())
            .map_err(Into::<Error>::into)?
            .into()
    };
    Ok(HtmlBase::new(body).into())
}

#[derive(RwebResponse)]
#[response(description = "Transcode Status File List", content = "html")]
struct TranscodeStatusProcsResponse(HtmlBase<StackString, Error>);

#[get("/list/transcode/status/procs")]
pub async fn movie_queue_transcode_status_procs(
    #[filter = "LoggedUser::filter"] _: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<TranscodeStatusProcsResponse> {
    let status = transcode_status(&state.config)
        .await
        .map_err(Into::<Error>::into)?;
    let body = procs_html_body(status).map_err(Into::<Error>::into)?.into();
    Ok(HtmlBase::new(body).into())
}

#[derive(RwebResponse)]
#[response(description = "Transcode File", content = "html", status = "CREATED")]
struct TranscodeFileResponse(HtmlBase<StackString, Error>);

#[post("/list/transcode/file/{filename}")]
pub async fn movie_queue_transcode_file(
    filename: StackString,
    #[filter = "LoggedUser::filter"] user: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<TranscodeFileResponse> {
    let url = format_sstr!("/list/transcode/file/{filename}");
    let task = user
        .store_url_task(state.trakt.get_client(), &state.config, &url)
        .await;
    let mock_stdout = MockStdout::new();
    let stdout = StdoutChannel::with_mock_stdout(mock_stdout.clone(), mock_stdout.clone());

    let transcode_service = TranscodeService::new(
        &state.config,
        &state.config.transcode_queue,
        &state.db,
        &stdout,
    );
    let input_path = state
        .config
        .home_dir
        .join("Documents")
        .join("movies")
        .join(&filename);
    let req = TranscodeServiceRequest::create_transcode_request(&state.config, &input_path)
        .map_err(Into::<Error>::into)?;
    transcode_service
        .publish_transcode_job(&req, |_| async move { Ok(()) })
        .await
        .map_err(Into::<Error>::into)?;
    let body = req
        .publish_to_cli(&state.config)
        .await
        .map_err(Into::<Error>::into)?;
    task.await.ok();
    Ok(HtmlBase::new(body).into())
}

#[post("/list/transcode/remcom/file/{filename}")]
pub async fn movie_queue_remcom_file(
    filename: StackString,
    #[filter = "LoggedUser::filter"] user: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<TranscodeFileResponse> {
    let url = format_sstr!("/list/transcode/remcom/file/{filename}");
    let task = user
        .store_url_task(state.trakt.get_client(), &state.config, &url)
        .await;
    let mock_stdout = MockStdout::new();
    let stdout = StdoutChannel::with_mock_stdout(mock_stdout.clone(), mock_stdout.clone());

    let transcode_service = TranscodeService::new(
        &state.config,
        &state.config.remcom_queue,
        &state.db,
        &stdout,
    );
    let input_path = state
        .config
        .home_dir
        .join("Documents")
        .join("movies")
        .join(&filename);
    let directory: Option<PathBuf> = None;
    let req = TranscodeServiceRequest::create_remcom_request(
        &state.config,
        &input_path,
        directory,
        false,
    )
    .await
    .map_err(Into::<Error>::into)?;
    transcode_service
        .publish_transcode_job(&req, |_| async move { Ok(()) })
        .await
        .map_err(Into::<Error>::into)?;
    let body = req
        .publish_to_cli(&state.config)
        .await
        .map_err(Into::<Error>::into)?;
    task.await.ok();
    Ok(HtmlBase::new(body).into())
}

#[post("/list/transcode/remcom/directory/{directory}/{filename}")]
pub async fn movie_queue_remcom_directory_file(
    directory: StackString,
    filename: StackString,
    #[filter = "LoggedUser::filter"] user: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<TranscodeFileResponse> {
    let url = format_sstr!("/list/transcode/remcom/directory/{directory}/{filename}");
    let task = user
        .store_url_task(state.trakt.get_client(), &state.config, &url)
        .await;
    let mock_stdout = MockStdout::new();
    let stdout = StdoutChannel::with_mock_stdout(mock_stdout.clone(), mock_stdout.clone());

    let transcode_service = TranscodeService::new(
        &state.config,
        &state.config.remcom_queue,
        &state.db,
        &stdout,
    );
    let input_path = state
        .config
        .home_dir
        .join("Documents")
        .join("movies")
        .join(&filename);
    let req = TranscodeServiceRequest::create_remcom_request(
        &state.config,
        &input_path,
        Some(directory),
        false,
    )
    .await
    .map_err(Into::<Error>::into)?;
    transcode_service
        .publish_transcode_job(&req, |_| async move { Ok(()) })
        .await
        .map_err(Into::<Error>::into)?;
    let body = req
        .publish_to_cli(&state.config)
        .await
        .map_err(Into::<Error>::into)?;
    task.await.ok();
    Ok(HtmlBase::new(body).into())
}

#[derive(RwebResponse)]
#[response(description = "Cleanup Transcode File", content = "html")]
struct CleanupTranscodeFileResponse(HtmlBase<StackString, Error>);

#[delete("/list/transcode/cleanup/{path}")]
pub async fn movie_queue_transcode_cleanup(
    path: StackString,
    #[filter = "LoggedUser::filter"] user: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<CleanupTranscodeFileResponse> {
    let url = format_sstr!("/list/transcode/cleanup/{path}");
    let task = user
        .store_url_task(state.trakt.get_client(), &state.config, &url)
        .await;
    let movie_path = state
        .config
        .home_dir
        .join("Documents")
        .join("movies")
        .join(&path);
    let tmp_path = state.config.home_dir.join("tmp_avi").join(&path);
    let body = if movie_path.exists() {
        remove_file(&movie_path)
            .await
            .map_err(Into::<Error>::into)?;
        let srt_path = movie_path.with_extension("srt");
        if srt_path.exists() {
            remove_file(&srt_path).await.map_err(Into::<Error>::into)?;
        }
        let movie_path = movie_path.to_str().unwrap_or("");
        format_sstr!("Removed {movie_path}")
    } else if tmp_path.exists() {
        remove_file(&tmp_path).await.map_err(Into::<Error>::into)?;
        let tmp_path = tmp_path.to_string_lossy();
        format_sstr!("Removed {tmp_path}")
    } else {
        format_sstr!("File not found {path}")
    };
    task.await.ok();
    Ok(HtmlBase::new(body).into())
}

#[derive(RwebResponse)]
#[response(description = "Trakt Watchlist", content = "html")]
struct TraktWatchlistResponse(HtmlBase<StackString, Error>);

#[get("/trakt/watchlist")]
pub async fn trakt_watchlist(
    #[filter = "LoggedUser::filter"] user: LoggedUser,
    #[data] state: AppState,
    query: Query<TraktWatchlistRequest>,
) -> WarpResult<TraktWatchlistResponse> {
    let query = query.into_inner();
    let task = user
        .store_url_task(state.trakt.get_client(), &state.config, "/trakt/watchlist")
        .await;
    let shows = get_watchlist_shows_db_map(
        &state.db,
        query.query.as_ref().map(StackString::as_str),
        query.source.map(Into::into),
        query.offset,
        query.limit,
    )
    .await
    .map_err(Into::<Error>::into)?;
    let body = watchlist_body(
        shows,
        query.query.as_ref().map(StackString::as_str),
        query.offset,
        query.limit,
        query.source.map(Into::into),
    )
    .map_err(Into::<Error>::into)?
    .into();
    task.await.ok();
    Ok(HtmlBase::new(body).into())
}

async fn watchlist_action_worker(
    trakt: &TraktConnection,
    mc: &MovieCollection,
    action: TraktActions,
    show: &str,
) -> HttpResult<StackString> {
    trakt.init().await;
    let mut body = StackString::new();
    match action {
        TraktActions::Add => {
            if let Some(result) = watchlist_add(trakt, mc, show, None).await? {
                write!(body, "{result}")?;
            }
        }
        TraktActions::Remove => {
            if let Some(result) = watchlist_rm(trakt, mc, show).await? {
                write!(body, "{result}")?;
            }
        }
        _ => (),
    }
    Ok(body)
}

#[derive(RwebResponse)]
#[response(
    description = "Trakt Watchlist Action",
    content = "html",
    status = "CREATED"
)]
struct TraktWatchlistActionResponse(HtmlBase<StackString, Error>);

#[post("/trakt/watchlist/{action}/{show}")]
pub async fn trakt_watchlist_action(
    action: TraktActionsWrapper,
    show: StackString,
    #[filter = "LoggedUser::filter"] user: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<TraktWatchlistActionResponse> {
    let url = format_sstr!("/trakt/watchlist/{action}/{show}");
    let task = user
        .store_url_task(state.trakt.get_client(), &state.config, &url)
        .await;
    let mock_stdout = MockStdout::new();
    let stdout = StdoutChannel::with_mock_stdout(mock_stdout.clone(), mock_stdout.clone());
    let mc = MovieCollection::new(&state.config, &state.db, &stdout);
    let body = watchlist_action_worker(&state.trakt, &mc, action.into(), &show).await?;
    task.await.ok();
    Ok(HtmlBase::new(body).into())
}

#[derive(RwebResponse)]
#[response(description = "Trakt Watchlist Show List", content = "html")]
struct TraktWatchlistShowListResponse(HtmlBase<StackString, Error>);

#[get("/trakt/watched/list/{imdb_url}")]
pub async fn trakt_watched_seasons(
    imdb_url: StackString,
    #[filter = "LoggedUser::filter"] user: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<TraktWatchlistShowListResponse> {
    let url = format_sstr!("/trakt/watched/list/{imdb_url}");
    let task = user
        .store_url_task(state.trakt.get_client(), &state.config, &url)
        .await;
    let show_opt = ImdbRatings::get_show_by_link(&imdb_url, &state.db)
        .await
        .map(|s| s.map(|sh| (imdb_url, sh)))
        .map_err(Into::<Error>::into)?;

    let empty = || ("".into(), "".into(), "".into());
    let (imdb_url, show, link) =
        show_opt.map_or_else(empty, |(imdb_url, t)| (imdb_url, t.show, t.link));
    let req = ImdbSeasonsRequest { show };
    let entries = req.process(&state.db, &state.config).await?;
    let body = trakt_watched_seasons_body(link, imdb_url, entries)
        .map_err(Into::<Error>::into)?
        .into();
    task.await.ok();
    Ok(HtmlBase::new(body).into())
}

#[derive(RwebResponse)]
#[response(description = "Trakt Watchlist Show Season", content = "html")]
struct TraktWatchlistShowSeasonResponse(HtmlBase<StackString, Error>);

#[get("/trakt/watched/list/{imdb_url}/{season}")]
pub async fn trakt_watched_list(
    imdb_url: StackString,
    season: i32,
    #[filter = "LoggedUser::filter"] user: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<TraktWatchlistShowSeasonResponse> {
    let url = format_sstr!("/trakt/watched/list/{imdb_url}/{season}");
    let task = user
        .store_url_task(state.trakt.get_client(), &state.config, &url)
        .await;
    let mock_stdout = MockStdout::new();
    let stdout = StdoutChannel::with_mock_stdout(mock_stdout.clone(), mock_stdout.clone());

    let body = watch_list_http_body(&state.config, &state.db, &stdout, &imdb_url, season)
        .await
        .map_err(Into::<Error>::into)?
        .into();
    task.await.ok();
    Ok(HtmlBase::new(body).into())
}

#[derive(RwebResponse)]
#[response(
    description = "Trakt Watchlist Episode Action",
    content = "html",
    status = "CREATED"
)]
struct TraktWatchlistEpisodeActionResponse(HtmlBase<StackString, Error>);

#[post("/trakt/watched/{action}/{imdb_url}/{season}/{episode}")]
pub async fn trakt_watched_action(
    action: TraktActionsWrapper,
    imdb_url: StackString,
    season: i32,
    episode: i32,
    #[filter = "LoggedUser::filter"] user: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<TraktWatchlistEpisodeActionResponse> {
    let url = format_sstr!("/trakt/watched/{action}/{imdb_url}/{season}/{episode}");
    let task = user
        .store_url_task(state.trakt.get_client(), &state.config, &url)
        .await;
    let mock_stdout = MockStdout::new();
    let stdout = StdoutChannel::with_mock_stdout(mock_stdout.clone(), mock_stdout.clone());

    let body = watched_action_http_worker(
        &state.trakt,
        &state.db,
        action.into(),
        &imdb_url,
        season,
        episode,
        &state.config,
        &stdout,
    )
    .await?;
    task.await.ok();
    Ok(HtmlBase::new(body).into())
}

#[derive(RwebResponse)]
#[response(description = "Trakt Calendar", content = "html")]
struct TraktCalendarResponse(HtmlBase<StackString, Error>);

#[get("/trakt/cal")]
pub async fn trakt_cal(
    #[filter = "LoggedUser::filter"] user: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<TraktCalendarResponse> {
    let task = user
        .store_url_task(state.trakt.get_client(), &state.config, "/trakt/cal")
        .await;
    let body = trakt_cal_http_body(&state.db, &state.trakt)
        .await
        .map_err(Into::<Error>::into)?
        .into();
    task.await.ok();
    Ok(HtmlBase::new(body).into())
}

#[derive(RwebResponse)]
#[response(description = "Trakt Auth Url", content = "html")]
struct TraktAuthUrlResponse(HtmlBase<StackString, Error>);

#[get("/trakt/auth_url")]
pub async fn trakt_auth_url(
    #[filter = "LoggedUser::filter"] user: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<TraktAuthUrlResponse> {
    let task = user
        .store_url_task(state.trakt.get_client(), &state.config, "/trakt/auth_url")
        .await;
    state.trakt.init().await;
    let url = state
        .trakt
        .get_auth_url()
        .await
        .map_err(Into::<Error>::into)?;
    task.await.ok();
    Ok(HtmlBase::new(url.as_str().into()).into())
}

#[derive(Serialize, Deserialize, Schema)]
#[schema(component = "TraktCallbackRequest")]
pub struct TraktCallbackRequest {
    #[schema(description = "Authorization Code")]
    pub code: StackString,
    #[schema(description = "CSRF State")]
    pub state: StackString,
}

#[derive(RwebResponse)]
#[response(description = "Trakt Callback", content = "html")]
struct TraktCallbackResponse(HtmlBase<&'static str, Error>);

#[get("/trakt/callback")]
pub async fn trakt_callback(
    query: Query<TraktCallbackRequest>,
    #[filter = "LoggedUser::filter"] user: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<TraktCallbackResponse> {
    let task = user
        .store_url_task(state.trakt.get_client(), &state.config, "/trakt/callback")
        .await;
    state.trakt.init().await;
    let query = query.into_inner();
    state
        .trakt
        .exchange_code_for_auth_token(query.code.as_str(), query.state.as_str())
        .await
        .map_err(Into::<Error>::into)?;
    let body = r#"
        <title>Trakt auth code received!</title>
        This window can be closed.
        <script language="JavaScript" type="text/javascript">window.close()</script>"#;
    task.await.ok();
    Ok(HtmlBase::new(body).into())
}

#[derive(RwebResponse)]
#[response(description = "Trakt Refresh Auth", content = "html")]
struct TraktRefreshAuthResponse(HtmlBase<&'static str, Error>);

#[get("/trakt/refresh_auth")]
pub async fn refresh_auth(
    #[filter = "LoggedUser::filter"] user: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<TraktRefreshAuthResponse> {
    let task = user
        .store_url_task(
            state.trakt.get_client(),
            &state.config,
            "/trakt/refresh_auth",
        )
        .await;
    state.trakt.init().await;
    state
        .trakt
        .exchange_refresh_token()
        .await
        .map_err(Into::<Error>::into)?;
    task.await.ok();
    Ok(HtmlBase::new("Finished").into())
}

#[allow(clippy::too_many_arguments)]
async fn watched_action_http_worker(
    trakt: &TraktConnection,
    pool: &PgPool,
    action: TraktActions,
    imdb_url: &str,
    season: i32,
    episode: i32,
    config: &Config,
    stdout: &StdoutChannel<StackString>,
) -> HttpResult<StackString> {
    let mc = MovieCollection::new(config, pool, stdout);
    trakt.init().await;
    let body = match action {
        TraktActions::Add => {
            let result = if season != -1 && episode != -1 {
                trakt
                    .add_episode_to_watched(imdb_url, season, episode)
                    .await?
            } else {
                trakt.add_movie_to_watched(imdb_url).await?
            };
            if season != -1 && episode != -1 {
                WatchedEpisode {
                    imdb_url: imdb_url.into(),
                    season,
                    episode,
                    ..WatchedEpisode::default()
                }
                .insert_episode(&mc.pool)
                .await?;
            } else {
                WatchedMovie {
                    imdb_url: imdb_url.into(),
                    title: "".into(),
                }
                .insert_movie(&mc.pool)
                .await?;
            }

            format_sstr!("{result}")
        }
        TraktActions::Remove => {
            let result = if season != -1 && episode != -1 {
                trakt
                    .remove_episode_to_watched(imdb_url, season, episode)
                    .await?
            } else {
                trakt.remove_movie_to_watched(imdb_url).await?
            };

            if season != -1 && episode != -1 {
                if let Some(epi_) =
                    WatchedEpisode::get_watched_episode(&mc.pool, imdb_url, season, episode).await?
                {
                    epi_.delete_episode(&mc.pool).await?;
                }
            } else if let Some(movie) = WatchedMovie::get_watched_movie(&mc.pool, imdb_url).await? {
                movie.delete_movie(&mc.pool).await?;
            };

            format_sstr!("{result}")
        }
        _ => StackString::new(),
    };
    Ok(body)
}

#[derive(Debug, Serialize, Deserialize, Schema)]
#[schema(component = "PaginatedPlexEvent")]
struct PaginatedPlexEvent {
    pagination: Pagination,
    data: Vec<PlexEventWrapper>,
}

#[derive(RwebResponse)]
#[response(description = "Plex Events")]
struct PlexEventResponse(JsonBase<PaginatedPlexEvent, Error>);

#[get("/list/plex_event")]
pub async fn plex_events(
    query: Query<PlexEventRequest>,
    #[data] state: AppState,
    #[filter = "LoggedUser::filter"] user: LoggedUser,
) -> WarpResult<PlexEventResponse> {
    let task = user
        .store_url_task(
            state.trakt.get_client(),
            &state.config,
            "/list/movie_collection",
        )
        .await;
    let query = query.into_inner();
    let start_timestamp = query.start_timestamp.map(Into::into);
    let event_type = query.event_type.map(Into::into);

    let total = PlexEvent::get_total(&state.db, start_timestamp, event_type)
        .await
        .map_err(Into::<Error>::into)?;
    let offset = query.offset.unwrap_or(0);
    let limit = query.limit.unwrap_or(total);
    let pagination = Pagination {
        total,
        offset,
        limit,
    };

    let data = PlexEvent::get_events(
        &state.db,
        start_timestamp,
        event_type,
        Some(offset),
        Some(limit),
    )
    .await
    .map_err(Into::<Error>::into)?
    .map_ok(Into::into)
    .try_collect()
    .await
    .map_err(Into::<Error>::into)?;
    task.await.ok();
    Ok(JsonBase::new(PaginatedPlexEvent { pagination, data }).into())
}

#[derive(Serialize, Deserialize, Debug, Schema)]
#[schema(component = "PlexEventUpdateRequest")]
pub struct PlexEventUpdateRequest {
    events: Vec<PlexEventWrapper>,
}

#[derive(RwebResponse)]
#[response(
    description = "Update Plex Events",
    content = "html",
    status = "CREATED"
)]
struct PlexEventUpdateResponse(HtmlBase<&'static str, Error>);

#[post("/list/plex_event")]
pub async fn plex_events_update(
    payload: Json<PlexEventUpdateRequest>,
    #[data] state: AppState,
    #[filter = "LoggedUser::filter"] user: LoggedUser,
) -> WarpResult<PlexEventUpdateResponse> {
    let task = user
        .store_url_task(state.trakt.get_client(), &state.config, "/list/plex_event")
        .await;
    let payload = payload.into_inner();
    for event in payload.events {
        let event: PlexEvent = event.into();
        event
            .write_event(&state.db)
            .await
            .map_err(Into::<Error>::into)?;
    }
    task.await.ok();
    Ok(HtmlBase::new("Success").into())
}

#[derive(RwebResponse)]
#[response(description = "Plex Webhook", content = "html", status = "CREATED")]
struct PlexWebhookResponse(HtmlBase<&'static str, Error>);

#[post("/list/plex/webhook/{webhook_key}")]
pub async fn plex_webhook(
    #[filter = "rweb::multipart::form"] form: FormData,
    #[data] state: AppState,
    webhook_key: UuidWrapper,
) -> WarpResult<PlexWebhookResponse> {
    if state.config.plex_webhook_key == webhook_key {
        process_payload(form, &state.db, &state.config)
            .await
            .map_err(Into::<Error>::into)?;
    } else {
        error!("Incorrect webhook key");
    }
    Ok(HtmlBase::new("").into())
}

async fn process_payload(
    mut form: FormData,
    pool: &PgPool,
    config: &Config,
) -> Result<(), anyhow::Error> {
    let mut buf = Vec::new();
    if let Some(item) = form.next().await {
        let mut stream = item?.stream();
        while let Some(chunk) = stream.next().await {
            buf.extend_from_slice(chunk?.chunk());
        }
    }
    if let Ok(event) = PlexEvent::get_from_payload(&buf) {
        event.write_event(pool).await?;
        if let Some(metadata_key) = event.metadata_key.as_ref() {
            if PlexFilename::get_by_key(pool, metadata_key)
                .await?
                .is_none()
            {
                if let Ok(filename) = event.get_filename(config).await {
                    filename.insert(pool).await?;
                }
            }
        }
        Ok(())
    } else {
        let buf = std::str::from_utf8(&buf)?;
        error!("failed deserialize {buf}");
        Err(format_err!("failed deserialize {buf}"))
    }
}

#[derive(RwebResponse)]
#[response(description = "Plex Event List", content = "html")]
struct PlexEventList(HtmlBase<StackString, Error>);

#[get("/list/plex")]
pub async fn plex_list(
    #[filter = "LoggedUser::filter"] user: LoggedUser,
    #[data] state: AppState,
    query: Query<PlexEventRequest>,
) -> WarpResult<PlexEventList> {
    let task = user
        .store_url_task(state.trakt.get_client(), &state.config, "/list/plex")
        .await;
    let query = query.into_inner();
    let section_type = query.section_type.map(Into::into);
    let events = PlexEvent::get_plex_events(
        &state.db,
        query.start_timestamp.map(Into::into),
        query.event_type.map(Into::into),
        section_type,
        query.offset,
        query.limit,
    )
    .await
    .map_err(Into::<Error>::into)?;
    let body = plex_body(
        state.config.clone(),
        events,
        section_type,
        query.offset,
        query.limit,
    )
    .map_err(Into::<Error>::into)?
    .into();
    task.await.ok();
    Ok(HtmlBase::new(body).into())
}

#[derive(RwebResponse)]
#[response(description = "Plex Event Detail", content = "html")]
struct PlexEventDetail(HtmlBase<StackString, Error>);

#[get("/list/plex/{id}")]
pub async fn plex_detail(
    #[filter = "LoggedUser::filter"] user: LoggedUser,
    #[data] state: AppState,
    id: UuidWrapper,
    query: Query<PlexEventRequest>,
) -> WarpResult<PlexEventDetail> {
    let query = query.into_inner();
    let url = format_sstr!("/list/plex/{id}");
    let task = user
        .store_url_task(state.trakt.get_client(), &state.config, &url)
        .await;
    let body = if let Some(event) = PlexEvent::get_event_by_id(&state.db, id.into())
        .await
        .map_err(Into::<Error>::into)?
    {
        plex_detail_body(state.config.clone(), event, query.offset, query.limit)
            .map_err(Into::<Error>::into)?
            .into()
    } else {
        "".into()
    };
    task.await.ok();
    Ok(HtmlBase::new(body).into())
}

#[derive(Debug, Serialize, Deserialize, Schema)]
#[schema(component = "PaginatedPlexFilename")]
struct PaginatedPlexFilename {
    pagination: Pagination,
    data: Vec<PlexFilenameWrapper>,
}

#[derive(RwebResponse)]
#[response(description = "Plex Filenames")]
struct PlexFilenameResponse(JsonBase<PaginatedPlexFilename, Error>);

#[get("/list/plex_filename")]
pub async fn plex_filename(
    query: Query<PlexFilenameRequest>,
    #[data] state: AppState,
    #[filter = "LoggedUser::filter"] user: LoggedUser,
) -> WarpResult<PlexFilenameResponse> {
    let task = user
        .store_url_task(
            state.trakt.get_client(),
            &state.config,
            "/list/plex_filename",
        )
        .await;
    let query = query.into_inner();
    let start_timestamp = query.start_timestamp.map(Into::into);
    let total = PlexFilename::get_total(&state.db, start_timestamp)
        .await
        .map_err(Into::<Error>::into)?;
    let offset = query.offset.unwrap_or(0);
    let limit = query.limit.unwrap_or(total);
    let pagination = Pagination {
        total,
        offset,
        limit,
    };

    let data = PlexFilename::get_filenames(
        &state.db,
        query.start_timestamp.map(Into::into),
        query.offset,
        query.limit,
    )
    .await
    .map_err(Into::<Error>::into)?
    .map_ok(Into::into)
    .try_collect()
    .await
    .map_err(Into::<Error>::into)?;
    task.await.ok();
    Ok(JsonBase::new(PaginatedPlexFilename { pagination, data }).into())
}

#[derive(Serialize, Deserialize, Debug, Schema)]
#[schema(component = "PlexFilenameUpdateRequest")]
pub struct PlexFilenameUpdateRequest {
    filenames: Vec<PlexFilenameWrapper>,
}

#[derive(RwebResponse)]
#[response(
    description = "Update Plex Filenames",
    content = "html",
    status = "CREATED"
)]
struct PlexFilenameUpdateResponse(HtmlBase<&'static str, Error>);

#[post("/list/plex_filename")]
pub async fn plex_filename_update(
    payload: Json<PlexFilenameUpdateRequest>,
    #[data] state: AppState,
    #[filter = "LoggedUser::filter"] user: LoggedUser,
) -> WarpResult<PlexFilenameUpdateResponse> {
    let task = user
        .store_url_task(
            state.trakt.get_client(),
            &state.config,
            "/list/plex_filename",
        )
        .await;
    let payload = payload.into_inner();
    for filename in payload.filenames {
        let filename: PlexFilename = filename.into();
        filename
            .insert(&state.db)
            .await
            .map_err(Into::<Error>::into)?;
    }
    task.await.ok();
    Ok(HtmlBase::new("Success").into())
}

#[derive(Debug, Serialize, Deserialize, Schema)]
#[schema(component = "PaginatedPlexMetadata")]
struct PaginatedPlexMetadata {
    pagination: Pagination,
    data: Vec<PlexMetadataWrapper>,
}

#[derive(RwebResponse)]
#[response(description = "Plex Metadata")]
struct PlexMetadataResponse(JsonBase<PaginatedPlexMetadata, Error>);

#[get("/list/plex_metadata")]
pub async fn plex_metadata(
    query: Query<PlexFilenameRequest>,
    #[data] state: AppState,
    #[filter = "LoggedUser::filter"] user: LoggedUser,
) -> WarpResult<PlexMetadataResponse> {
    let task = user
        .store_url_task(
            state.trakt.get_client(),
            &state.config,
            "/list/plex_metadata",
        )
        .await;
    let query = query.into_inner();
    let start_timestamp = query.start_timestamp.map(Into::into);
    let total = PlexMetadata::get_total(&state.db, start_timestamp)
        .await
        .map_err(Into::<Error>::into)?;
    let offset = query.offset.unwrap_or(0);
    let limit = query.limit.unwrap_or(total);
    let pagination = Pagination {
        total,
        offset,
        limit,
    };

    let data = PlexMetadata::get_entries(&state.db, start_timestamp, Some(offset), Some(limit))
        .await
        .map_err(Into::<Error>::into)?
        .map_ok(Into::into)
        .try_collect()
        .await
        .map_err(Into::<Error>::into)?;
    task.await.ok();
    Ok(JsonBase::new(PaginatedPlexMetadata { pagination, data }).into())
}

#[derive(Serialize, Deserialize, Debug, Schema)]
#[schema(component = "PlexMetadataUpdateRequest")]
pub struct PlexMetadataUpdateRequest {
    entries: Vec<PlexMetadataWrapper>,
}

#[derive(RwebResponse)]
#[response(
    description = "Update Plex Metadata",
    content = "html",
    status = "CREATED"
)]
struct PlexMetadataUpdateResponse(HtmlBase<&'static str, Error>);

#[post("/list/plex_metadata")]
pub async fn plex_metadata_update(
    payload: Json<PlexMetadataUpdateRequest>,
    #[data] state: AppState,
    #[filter = "LoggedUser::filter"] user: LoggedUser,
) -> WarpResult<PlexMetadataUpdateResponse> {
    let task = user
        .store_url_task(
            state.trakt.get_client(),
            &state.config,
            "/list/plex_metadata",
        )
        .await;
    let payload = payload.into_inner();
    for metadata in payload.entries {
        let metadata: PlexMetadata = metadata.into();
        metadata
            .insert(&state.db)
            .await
            .map_err(Into::<Error>::into)?;
    }
    task.await.ok();
    Ok(HtmlBase::new("Success").into())
}

#[derive(Debug, Serialize, Deserialize, Schema)]
#[schema(component = "PaginatedMusicCollection")]
struct PaginatedMusicCollection {
    pagination: Pagination,
    data: Vec<MusicCollectionWrapper>,
}

#[derive(RwebResponse)]
#[response(description = "Music Collection")]
struct MusicCollectionResponse(JsonBase<PaginatedMusicCollection, Error>);

#[get("/list/music_collection")]
pub async fn music_collection(
    query: Query<PlexFilenameRequest>,
    #[data] state: AppState,
    #[filter = "LoggedUser::filter"] user: LoggedUser,
) -> WarpResult<MusicCollectionResponse> {
    let task = user
        .store_url_task(
            state.trakt.get_client(),
            &state.config,
            "/list/music_collection",
        )
        .await;
    let query = query.into_inner();
    let start_timestamp = query.start_timestamp.map(Into::into);
    let total = MusicCollection::get_count(&state.db, start_timestamp)
        .await
        .map_err(Into::<Error>::into)?;
    let offset = query.offset.unwrap_or(0);
    let limit = query.limit.unwrap_or(total);
    let pagination = Pagination {
        total,
        offset,
        limit,
    };

    let data = MusicCollection::get_entries(&state.db, start_timestamp, Some(offset), Some(limit))
        .await
        .map_err(Into::<Error>::into)?
        .map_ok(Into::into)
        .try_collect()
        .await
        .map_err(Into::<Error>::into)?;
    task.await.ok();
    Ok(JsonBase::new(PaginatedMusicCollection { pagination, data }).into())
}

#[derive(Serialize, Deserialize, Debug, Schema)]
pub struct MusicCollectionUpdateRequest {
    entries: Vec<MusicCollectionWrapper>,
}

#[derive(RwebResponse)]
#[response(
    description = "Update Music Collection",
    content = "html",
    status = "CREATED"
)]
struct MusicCollectionUpdateResponse(HtmlBase<&'static str, Error>);

#[post("/list/music_collection")]
pub async fn music_collection_update(
    payload: Json<MusicCollectionUpdateRequest>,
    #[data] state: AppState,
    #[filter = "LoggedUser::filter"] user: LoggedUser,
) -> WarpResult<MusicCollectionUpdateResponse> {
    let task = user
        .store_url_task(
            state.trakt.get_client(),
            &state.config,
            "/list/music_collection",
        )
        .await;
    let payload = payload.into_inner();
    for entry in payload.entries {
        if MusicCollection::get_by_id(&state.db, entry.id)
            .await
            .map_err(Into::<Error>::into)?
            .is_none()
        {
            let entry: MusicCollection = entry.into();
            entry.insert(&state.db).await.map_err(Into::<Error>::into)?;
        }
    }
    task.await.ok();
    Ok(HtmlBase::new("Success").into())
}

#[derive(RwebResponse)]
#[response(
    description = "Extract Subtitles",
    content = "html",
    status = "Created"
)]
struct ExtractSubtitlesResponse(HtmlBase<StackString, Error>);

#[post("/list/transcode/subtitle/{filename}/{index}/{suffix}")]
pub async fn movie_queue_extract_subtitle(
    filename: StackString,
    index: usize,
    suffix: StackString,
    #[filter = "LoggedUser::filter"] user: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<ExtractSubtitlesResponse> {
    let url = format_sstr!("/list/transcode/subtitle/{filename}/{index}/{suffix}");
    let task = user
        .store_url_task(state.trakt.get_client(), &state.config, &url)
        .await;

    let input_path = state
        .config
        .home_dir
        .join("Documents")
        .join("movies")
        .join(&filename);

    let input_file: StackString = input_path.to_string_lossy().into();
    let pyasstosrt_path = state.config.pyasstosrt_path.as_deref();
    let output = MkvTrack::extract_subtitles_from_mkv(
        &input_file,
        index,
        &suffix,
        pyasstosrt_path,
        &state.config.mkvextract_path,
    )
    .await
    .map_err(Into::<Error>::into)?;
    task.await.ok();

    Ok(HtmlBase::new(output).into())
}
