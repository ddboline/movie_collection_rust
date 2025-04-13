#![allow(clippy::needless_pass_by_value)]

use anyhow::format_err;
use axum::extract::{Json, Multipart, Path, Query, State};
use bytes::Buf;
use derive_more::{From, Into};
use futures::TryStreamExt;
use log::error;
use serde::{Deserialize, Serialize};
use stack_string::{format_sstr, StackString};
use std::{
    borrow::Borrow,
    collections::HashMap,
    fmt::Write,
    hash::{Hash, Hasher},
    path,
    path::PathBuf,
    sync::Arc,
};
use stdout_channel::{MockStdout, StdoutChannel};
use time::OffsetDateTime;
use tokio::fs::remove_file;
use tokio_stream::StreamExt;
use utoipa::{IntoParams, OpenApi, PartialSchema, ToSchema};
use utoipa_axum::{router::OpenApiRouter, routes};
use utoipa_helper::{
    derive_utoipa_params, derive_utoipa_schema, html_response::HtmlResponse as HtmlBase,
    json_response::JsonResponse as JsonBase, UtoipaResponse,
};
use uuid::Uuid;

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
    PlexEventRequest, PlexEventTypeWrapper, PlexEventWrapper, PlexFilenameRequest,
    PlexFilenameWrapper, PlexMetadataWrapper, PlexSectionTypeWrapper, TraktActionsWrapper,
    TraktWatchlistRequest, TvShowSourceWrapper,
};

type WarpResult<T> = Result<T, Error>;

#[derive(UtoipaResponse)]
#[response(description = "Scripts", content = "text/javascript")]
#[rustfmt::skip]
struct JsScriptsResponse(HtmlBase::<&'static str>);

#[utoipa::path(get, path = "/list/scripts.js", responses(JsScriptsResponse))]
async fn scripts_js() -> JsScriptsResponse {
    HtmlBase::new(include_str!("../../templates/scripts.js")).into()
}

#[derive(UtoipaResponse)]
#[response(description = "Movie Queue", content = "text/html")]
#[rustfmt::skip]
struct MovieQueueResponse(HtmlBase::<StackString>);

#[derive(Serialize, Deserialize, Debug, ToSchema, IntoParams)]
// FullQueueRequest
struct FullQueueRequest {
    // Search String
    #[schema(inline)]
    #[param(inline)]
    q: Option<StackString>,
    // Offset
    offset: Option<usize>,
    // Limit
    limit: Option<usize>,
    // Order By (asc/desc)
    order_by: Option<OrderByWrapper>,
}

#[utoipa::path(
    get,
    path = "/list/full_queue",
    params(FullQueueRequest),
    responses(MovieQueueResponse, Error)
)]
async fn movie_queue(
    user: LoggedUser,
    state: State<Arc<AppState>>,
    query: Query<FullQueueRequest>,
) -> WarpResult<MovieQueueResponse> {
    let Query(query) = query;
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

#[utoipa::path(get, path = "/list/queue/{path}", params(("path" = inline(StackString), description = "Path")), responses(MovieQueueResponse, Error))]
async fn movie_queue_show(
    state: State<Arc<AppState>>,
    path: Path<StackString>,
    user: LoggedUser,
) -> WarpResult<MovieQueueResponse> {
    let Path(path) = path;
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

#[derive(UtoipaResponse)]
#[response(description = "Delete Queue Entry", content = "text/html")]
#[rustfmt::skip]
struct DeleteMovieQueueResponse(HtmlBase::<StackString>);

#[utoipa::path(
    delete,
    path = "/list/delete/{path}",
    params(("path" = inline(StackString), description = "Path")),
    responses(DeleteMovieQueueResponse, Error)
)]
async fn movie_queue_delete(
    state: State<Arc<AppState>>,
    path: Path<StackString>,
    user: LoggedUser,
) -> WarpResult<DeleteMovieQueueResponse> {
    let Path(path) = path;
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
) -> WarpResult<StackString> {
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

#[derive(UtoipaResponse)]
#[response(
    description = "Transcode Queue Item",
    content = "text/html",
    status = "CREATED"
)]
#[rustfmt::skip]
struct TranscodeQueueResponse(HtmlBase::<StackString>);

#[utoipa::path(
    post,
    path = "/list/transcode/queue/{path}",
    params(("path" = inline(StackString), description = "Path")),
    responses(TranscodeQueueResponse, Error)
)]
async fn movie_queue_transcode(
    state: State<Arc<AppState>>,
    path: Path<StackString>,
    user: LoggedUser,
) -> WarpResult<TranscodeQueueResponse> {
    let Path(path) = path;
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

#[utoipa::path(
    post,
    path = "/list/transcode/queue/{directory}/{file}",
    params(
        ("directory" = inline(StackString), description = "Directory"),
        ("file" = inline(StackString), description = "File"),
    ),
    responses(TranscodeQueueResponse, Error)
)]
async fn movie_queue_transcode_directory(
    state: State<Arc<AppState>>,
    paths: Path<(StackString, StackString)>,
    user: LoggedUser,
) -> WarpResult<TranscodeQueueResponse> {
    let Path((directory, file)) = paths;

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

#[derive(UtoipaResponse)]
#[response(description = "Play Queue Item", content = "text/html")]
#[rustfmt::skip]
struct PlayQueueResponse(HtmlBase::<StackString>);

#[utoipa::path(get, path = "/list/play/{idx}", params(("idx" = Uuid, description = "Index")), responses(PlayQueueResponse, Error))]
async fn movie_queue_play(
    idx: Path<Uuid>,
    user: LoggedUser,
    state: State<Arc<AppState>>,
) -> WarpResult<PlayQueueResponse> {
    let Path(idx) = idx;
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
        .get_collection_path(idx)
        .await
        .map_err(Into::<Error>::into)?;

    let movie_path = path::Path::new(movie_path.as_str());
    let body = play_worker_body(&state.config, movie_path, last_url)
        .map_err(Into::<Error>::into)?
        .into();
    task.await.ok();
    Ok(HtmlBase::new(body).into())
}

#[derive(UtoipaResponse)]
#[response(description = "List Imdb Show", content = "text/html")]
#[rustfmt::skip]
struct ListImdbResponse(HtmlBase::<StackString>);

#[utoipa::path(get, path = "/list/imdb/{show}", params(("show" = inline(StackString), description = "Show"), ParseImdbRequest), responses(ListImdbResponse, Error))]
async fn imdb_show(
    state: State<Arc<AppState>>,
    show: Path<StackString>,
    query: Query<ParseImdbRequest>,
    user: LoggedUser,
) -> WarpResult<ListImdbResponse> {
    let Path(show) = show;
    let url = format_sstr!("/list/imdb/{show}");
    let task = user
        .store_url_task(state.trakt.get_client(), &state.config, &url)
        .await;
    let Query(query) = query;
    let req = ImdbShowRequest { show, query };
    let body = req.process(&state.db, &state.config).await?;
    task.await.ok();
    Ok(HtmlBase::new(body).into())
}

#[derive(Serialize, Deserialize, ToSchema, IntoParams)]
// FindNewEpisodeRequest
struct FindNewEpisodeRequest {
    // TV Show Source
    source: Option<TvShowSourceWrapper>,
    // TV Show
    #[schema(inline)]
    #[param(inline)]
    shows: Option<StackString>,
}

#[derive(UtoipaResponse)]
#[response(description = "List Calendar", content = "text/html")]
#[rustfmt::skip]
struct ListCalendarResponse(HtmlBase::<StackString>);

#[utoipa::path(
    get,
    path = "/list/cal",
    params(FindNewEpisodeRequest),
    responses(ListCalendarResponse, Error)
)]
async fn find_new_episodes(
    query: Query<FindNewEpisodeRequest>,
    user: LoggedUser,
    state: State<Arc<AppState>>,
) -> WarpResult<ListCalendarResponse> {
    let Query(query) = query;

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
struct ImdbEpisodesSyncRequest {
    start_timestamp: Option<DateTimeWrapper>,
    offset: Option<usize>,
    limit: Option<usize>,
}

derive_utoipa_schema!(ImdbEpisodesSyncRequest, _ImdbEpisodesSyncRequest);
derive_utoipa_params!(ImdbEpisodesSyncRequest, _ImdbEpisodesSyncRequest);

#[derive(ToSchema, IntoParams)]
#[allow(dead_code)]
struct _ImdbEpisodesSyncRequest {
    // Start Timestamp
    start_timestamp: Option<OffsetDateTime>,
    // Offset
    offset: Option<usize>,
    // Limit
    limit: Option<usize>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
// Pagination
struct Pagination {
    // Total Number of Entries
    total: usize,
    // Number of Entries to Skip
    offset: usize,
    // Number of Entries Returned
    limit: usize,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
// PaginatedImdbEpisodes
struct PaginatedImdbEpisodes {
    pagination: Pagination,
    data: Vec<ImdbEpisodesWrapper>,
}

#[derive(UtoipaResponse)]
#[response(description = "List Imdb Episodes")]
#[rustfmt::skip]
struct ListImdbEpisodesResponse(JsonBase::<PaginatedImdbEpisodes>);

#[utoipa::path(
    get,
    path = "/list/imdb_episodes",
    params(ImdbEpisodesSyncRequest),
    responses(ListImdbEpisodesResponse, Error)
)]
async fn imdb_episodes_route(
    query: Query<ImdbEpisodesSyncRequest>,
    user: LoggedUser,
    state: State<Arc<AppState>>,
) -> WarpResult<ListImdbEpisodesResponse> {
    let Query(query) = query;

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

#[derive(UtoipaResponse)]
#[response(
    description = "Imdb Episodes Update",
    content = "text/html",
    status = "CREATED"
)]
#[rustfmt::skip]
struct ImdbEpisodesUpdateResponse(HtmlBase::<&'static str>);

#[utoipa::path(
    post,
    path = "/list/imdb_episodes",
    request_body = ImdbEpisodesUpdateRequest,
    responses(ImdbEpisodesUpdateResponse, Error)
)]
async fn imdb_episodes_update(
    user: LoggedUser,
    state: State<Arc<AppState>>,
    episodes: Json<ImdbEpisodesUpdateRequest>,
) -> WarpResult<ImdbEpisodesUpdateResponse> {
    let Json(episodes) = episodes;
    let task = user
        .store_url_task(
            state.trakt.get_client(),
            &state.config,
            "/list/imdb_episodes",
        )
        .await;
    episodes.run_update(&state.db).await?;
    task.await.ok();
    Ok(HtmlBase::new("Success").into())
}

#[derive(Serialize, Deserialize)]
struct ImdbRatingsSyncRequest {
    start_timestamp: Option<DateTimeWrapper>,
    offset: Option<usize>,
    limit: Option<usize>,
}

derive_utoipa_schema!(ImdbRatingsSyncRequest, _ImdbEpisodesSyncRequest);
derive_utoipa_params!(ImdbRatingsSyncRequest, _ImdbEpisodesSyncRequest);

#[derive(Debug, Serialize, Deserialize, ToSchema, IntoParams)]
// PaginatedImdbRatings
struct PaginatedImdbRatings {
    pagination: Pagination,
    data: Vec<ImdbRatingsWrapper>,
}

#[derive(UtoipaResponse)]
#[response(description = "List Imdb Shows")]
#[rustfmt::skip]
struct ListImdbShowsResponse(JsonBase::<PaginatedImdbRatings>);

#[utoipa::path(
    get,
    path = "/list/imdb_ratings",
    params(ImdbRatingsSyncRequest),
    responses(ListImdbShowsResponse, Error)
)]
async fn imdb_ratings_route(
    query: Query<ImdbRatingsSyncRequest>,
    user: LoggedUser,
    state: State<Arc<AppState>>,
) -> WarpResult<ListImdbShowsResponse> {
    let Query(query) = query;

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

#[derive(UtoipaResponse)]
#[response(
    description = "Update Imdb Shows",
    content = "text/html",
    status = "CREATED"
)]
#[rustfmt::skip]
struct UpdateImdbShowsResponse(HtmlBase::<&'static str>);

#[utoipa::path(
    post,
    path = "/list/imdb_ratings",
    request_body = ImdbRatingsUpdateRequest,
    responses(UpdateImdbShowsResponse, Error)
)]
async fn imdb_ratings_update(
    user: LoggedUser,
    state: State<Arc<AppState>>,
    shows: Json<ImdbRatingsUpdateRequest>,
) -> WarpResult<UpdateImdbShowsResponse> {
    let Json(shows) = shows;
    let task = user
        .store_url_task(
            state.trakt.get_client(),
            &state.config,
            "/list/imdb_ratings",
        )
        .await;
    shows.run_update(&state.db).await?;
    task.await.ok();
    Ok(HtmlBase::new("Success").into())
}

#[derive(UtoipaResponse)]
#[response(description = "Imdb Show Set Source", content = "text/html")]
#[rustfmt::skip]
struct ImdbSetSourceResponse(HtmlBase::<&'static str>);

#[utoipa::path(
    get,
    path = "/list/imdb_ratings/set_source",
    params(ImdbRatingsSetSourceRequest),
    responses(ImdbSetSourceResponse, Error)
)]
async fn imdb_ratings_set_source(
    query: Query<ImdbRatingsSetSourceRequest>,
    user: LoggedUser,
    state: State<Arc<AppState>>,
) -> WarpResult<ImdbSetSourceResponse> {
    let Query(query) = query;
    let task = user
        .store_url_task(
            state.trakt.get_client(),
            &state.config,
            "/list/imdb_ratings/set_source",
        )
        .await;
    query.set_source(&state.db).await?;
    task.await.ok();
    Ok(HtmlBase::new("Success").into())
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
// PaginatedMovieQueueRow
struct PaginatedMovieQueueRow {
    pagination: Pagination,
    data: Vec<MovieQueueRowWrapper>,
}

#[derive(UtoipaResponse)]
#[response(description = "List Movie Queue Entries")]
#[rustfmt::skip]
struct ListMovieQueueResponse(JsonBase::<PaginatedMovieQueueRow>);

#[utoipa::path(
    get,
    path = "/list/movie_queue",
    params(MovieQueueSyncRequest),
    responses(ListMovieQueueResponse, Error)
)]
async fn movie_queue_route(
    query: Query<MovieQueueSyncRequest>,
    user: LoggedUser,
    state: State<Arc<AppState>>,
) -> WarpResult<ListMovieQueueResponse> {
    let Query(query) = query;

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

#[derive(UtoipaResponse)]
#[response(
    description = "Update Movie Queue Entries",
    content = "text/html",
    status = "CREATED"
)]
#[rustfmt::skip]
struct UpdateMovieQueueResponse(HtmlBase::<&'static str>);

#[utoipa::path(
    post,
    path = "/list/movie_queue",
    request_body = MovieQueueUpdateRequest,
    responses(UpdateMovieQueueResponse, Error)
)]
async fn movie_queue_update(
    user: LoggedUser,
    state: State<Arc<AppState>>,
    queue: Json<MovieQueueUpdateRequest>,
) -> WarpResult<UpdateMovieQueueResponse> {
    let Json(queue) = queue;
    let task = user
        .store_url_task(state.trakt.get_client(), &state.config, "/list/movie_queue")
        .await;
    queue.run_update(&state.db, &state.config).await?;
    task.await.ok();
    Ok(HtmlBase::new("Success").into())
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
// PaginatedMovieCollectionRow
struct PaginatedMovieCollectionRow {
    pagination: Pagination,
    data: Vec<MovieCollectionRowWrapper>,
}

#[derive(UtoipaResponse)]
#[response(description = "List Movie Collection Entries")]
#[rustfmt::skip]
struct ListMovieCollectionResponse(JsonBase::<PaginatedMovieCollectionRow>);

#[utoipa::path(
    get,
    path = "/list/movie_collection",
    params(MovieCollectionSyncRequest),
    responses(ListMovieCollectionResponse, Error)
)]
async fn movie_collection_route(
    query: Query<MovieCollectionSyncRequest>,
    user: LoggedUser,
    state: State<Arc<AppState>>,
) -> WarpResult<ListMovieCollectionResponse> {
    let task = user
        .store_url_task(state.trakt.get_client(), &state.config, "/list/movie_queue")
        .await;
    let Query(query) = query;
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

#[derive(UtoipaResponse)]
#[response(
    description = "Update Movie Collection Entries",
    content = "text/html",
    status = "CREATED"
)]
#[rustfmt::skip]
struct UpdateMovieCollectionResponse(HtmlBase::<&'static str>);

#[utoipa::path(
    post,
    path = "/list/movie_collection",
    request_body = MovieCollectionUpdateRequest,
    responses(UpdateMovieCollectionResponse, Error)
)]
async fn movie_collection_update(
    user: LoggedUser,
    state: State<Arc<AppState>>,
    collection: Json<MovieCollectionUpdateRequest>,
) -> WarpResult<UpdateMovieCollectionResponse> {
    let Json(collection) = collection;
    let task = user
        .store_url_task(
            state.trakt.get_client(),
            &state.config,
            "/list/movie_collection",
        )
        .await;
    collection.run_update(&state.db, &state.config).await?;
    task.await.ok();
    Ok(HtmlBase::new("Success").into())
}

#[derive(ToSchema, Serialize, Into, From)]
struct LastModifiedList(Vec<LastModifiedResponseWrapper>);

#[derive(UtoipaResponse)]
#[response(description = "Database Entries Last Modified Time")]
#[rustfmt::skip]
struct ListLastModifiedResponse(JsonBase::<LastModifiedList>);

#[utoipa::path(
    get,
    path = "/list/last_modified",
    responses(ListLastModifiedResponse, Error)
)]
async fn last_modified_route(
    state: State<Arc<AppState>>,
    user: LoggedUser,
) -> WarpResult<ListLastModifiedResponse> {
    let task = user
        .store_url_task(
            state.trakt.get_client(),
            &state.config,
            "/list/last_modified",
        )
        .await;
    let last_modified: Vec<_> = LastModifiedResponse::get_last_modified(&state.db)
        .await
        .map_err(Into::<Error>::into)?
        .into_iter()
        .map(Into::into)
        .collect();
    task.await.ok();
    Ok(JsonBase::new(last_modified.into()).into())
}

#[derive(UtoipaResponse)]
#[response(description = "Frontpage", content = "text/html")]
#[rustfmt::skip]
struct FrontpageResponse(HtmlBase::<StackString>);

#[utoipa::path(get, path = "/list/index.html", responses(FrontpageResponse, Error))]
#[allow(clippy::unused_async)]
async fn frontpage(_: LoggedUser) -> WarpResult<FrontpageResponse> {
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

#[derive(UtoipaResponse)]
#[response(description = "List TvShows", content = "text/html")]
#[rustfmt::skip]
struct ListTvShowsResponse(HtmlBase::<StackString>);

#[utoipa::path(
    get,
    path = "/list/tvshows",
    params(TraktWatchlistRequest),
    responses(ListTvShowsResponse, Error)
)]
async fn tvshows(
    user: LoggedUser,
    state: State<Arc<AppState>>,
    query: Query<TraktWatchlistRequest>,
) -> WarpResult<ListTvShowsResponse> {
    let Query(query) = query;
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

#[derive(UtoipaResponse)]
#[response(description = "Logged in User")]
#[rustfmt::skip]
struct UserResponse(JsonBase::<LoggedUser>);

#[utoipa::path(get, path = "/list/user", responses(UserResponse))]
async fn user(user: LoggedUser) -> UserResponse {
    JsonBase::new(user).into()
}

#[derive(UtoipaResponse)]
#[response(description = "Transcode Status", content = "text/html")]
#[rustfmt::skip]
struct TranscodeStatusResponse(HtmlBase::<StackString>);

#[utoipa::path(
    get,
    path = "/list/transcode/status",
    responses(TranscodeStatusResponse, Error)
)]
async fn movie_queue_transcode_status(
    user: LoggedUser,
    state: State<Arc<AppState>>,
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

#[derive(UtoipaResponse)]
#[response(description = "Transcode Status File List", content = "text/html")]
#[rustfmt::skip]
struct TranscodeStatusFileListResponse(HtmlBase::<StackString>);

#[utoipa::path(
    get,
    path = "/list/transcode/status/file_list",
    responses(TranscodeStatusFileListResponse, Error)
)]
async fn movie_queue_transcode_status_file_list(
    _: LoggedUser,
    state: State<Arc<AppState>>,
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

#[derive(UtoipaResponse)]
#[response(description = "Transcode Status File List", content = "text/html")]
#[rustfmt::skip]
struct TranscodeStatusProcsResponse(HtmlBase::<StackString>);

#[utoipa::path(
    get,
    path = "/list/transcode/status/procs",
    responses(TranscodeStatusProcsResponse, Error)
)]
async fn movie_queue_transcode_status_procs(
    _: LoggedUser,
    state: State<Arc<AppState>>,
) -> WarpResult<TranscodeStatusProcsResponse> {
    let status = transcode_status(&state.config)
        .await
        .map_err(Into::<Error>::into)?;
    let body = procs_html_body(status).map_err(Into::<Error>::into)?.into();
    Ok(HtmlBase::new(body).into())
}

#[derive(UtoipaResponse)]
#[response(description = "Transcode File", content = "text/html", status = "CREATED")]
#[rustfmt::skip]
struct TranscodeFileResponse(HtmlBase::<StackString>);

#[utoipa::path(
    post,
    path = "/list/transcode/file/{filename}",
    params(("filename" = inline(StackString), description = "Filename")),
    responses(TranscodeFileResponse, Error)
)]
async fn movie_queue_transcode_file(
    state: State<Arc<AppState>>,
    filename: Path<StackString>,
    user: LoggedUser,
) -> WarpResult<TranscodeFileResponse> {
    let Path(filename) = filename;
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

#[utoipa::path(
    post,
    path = "/list/transcode/remcom/file/{filename}",
    params(("filename" = inline(StackString), description = "Filename")),
    responses(TranscodeFileResponse, Error)
)]
async fn movie_queue_remcom_file(
    state: State<Arc<AppState>>,
    filename: Path<StackString>,
    user: LoggedUser,
) -> WarpResult<TranscodeFileResponse> {
    let Path(filename) = filename;
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

#[utoipa::path(
    post,
    path = "/list/transcode/remcom/directory/{directory}/{filename}",
    params(
        ("directory" = inline(StackString), description = "Directory"),
        ("filename" = inline(StackString), description = "Filename"),
    ),
    responses(TranscodeFileResponse, Error)
)]
async fn movie_queue_remcom_directory_file(
    state: State<Arc<AppState>>,
    paths: Path<(StackString, StackString)>,
    user: LoggedUser,
) -> WarpResult<TranscodeFileResponse> {
    let Path((directory, filename)) = paths;
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

#[derive(UtoipaResponse)]
#[response(description = "Cleanup Transcode File", content = "text/html")]
#[rustfmt::skip]
struct CleanupTranscodeFileResponse(HtmlBase::<StackString>);

#[utoipa::path(
    delete,
    path = "/list/transcode/cleanup/{path}",
    params(("path" = inline(StackString), description = "Path")),
    responses(CleanupTranscodeFileResponse, Error)
)]
async fn movie_queue_transcode_cleanup(
    state: State<Arc<AppState>>,
    path: Path<StackString>,
    user: LoggedUser,
) -> WarpResult<CleanupTranscodeFileResponse> {
    let Path(path) = path;
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

#[derive(UtoipaResponse)]
#[response(description = "Trakt Watchlist", content = "text/html")]
#[rustfmt::skip]
struct TraktWatchlistResponse(HtmlBase::<StackString>);

#[utoipa::path(
    get,
    path = "/trakt/watchlist",
    params(TraktWatchlistRequest),
    responses(TraktWatchlistResponse, Error)
)]
async fn trakt_watchlist(
    user: LoggedUser,
    state: State<Arc<AppState>>,
    query: Query<TraktWatchlistRequest>,
) -> WarpResult<TraktWatchlistResponse> {
    let Query(query) = query;
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
) -> WarpResult<StackString> {
    trakt.init().await?;
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

#[derive(UtoipaResponse)]
#[response(
    description = "Trakt Watchlist Action",
    content = "text/html",
    status = "CREATED"
)]
#[rustfmt::skip]
struct TraktWatchlistActionResponse(HtmlBase::<StackString>);

#[utoipa::path(
    post,
    path = "/trakt/watchlist/{action}/{show}",
    params(
        ("action" = TraktActionsWrapper, description = "Action"),
        ("show" = inline(StackString), description = "Show"),
    ),
    responses(TraktWatchlistActionResponse, Error)
)]
async fn trakt_watchlist_action(
    state: State<Arc<AppState>>,
    paths: Path<(TraktActionsWrapper, StackString)>,
    user: LoggedUser,
) -> WarpResult<TraktWatchlistActionResponse> {
    let Path((action, show)) = paths;
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

#[derive(UtoipaResponse)]
#[response(description = "Trakt Watchlist Show List", content = "text/html")]
#[rustfmt::skip]
struct TraktWatchlistShowListResponse(HtmlBase::<StackString>);

#[utoipa::path(
    get,
    path = "/trakt/watched/list/{imdb_url}",
    params(("imdb_url" = inline(StackString), description = "IMDB Url")),
    responses(TraktWatchlistShowListResponse, Error)
)]
async fn trakt_watched_seasons(
    state: State<Arc<AppState>>,
    imdb_url: Path<StackString>,
    user: LoggedUser,
) -> WarpResult<TraktWatchlistShowListResponse> {
    let Path(imdb_url) = imdb_url;
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

#[derive(UtoipaResponse)]
#[response(description = "Trakt Watchlist Show Season", content = "text/html")]
#[rustfmt::skip]
struct TraktWatchlistShowSeasonResponse(HtmlBase::<StackString>);

#[utoipa::path(
    get,
    path = "/trakt/watched/list/{imdb_url}/{season}",
    params(
        ("imdb_url" = inline(StackString), description = "IMDB Url"),
        ("season" = i32, description = "Season"),
    ),
    responses(TraktWatchlistShowSeasonResponse, Error)
)]
async fn trakt_watched_list(
    state: State<Arc<AppState>>,
    paths: Path<(StackString, i32)>,
    user: LoggedUser,
) -> WarpResult<TraktWatchlistShowSeasonResponse> {
    let Path((imdb_url, season)) = paths;
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

#[derive(UtoipaResponse)]
#[response(
    description = "Trakt Watchlist Episode Action",
    content = "text/html",
    status = "CREATED"
)]
#[rustfmt::skip]
struct TraktWatchlistEpisodeActionResponse(HtmlBase::<StackString>);

#[utoipa::path(
    post,
    path = "/trakt/watched/{action}/{imdb_url}/{season}/{episode}",
    params(
        ("action" = TraktActionsWrapper, description = "Action"),
        ("imdb_url" = inline(StackString), description = "IMDB Url"),
        ("season" = i32, description = "Season"),
        ("episode" = i32, description = "Episode"),
    ),
    responses(TraktWatchlistEpisodeActionResponse, Error)
)]
async fn trakt_watched_action(
    state: State<Arc<AppState>>,
    paths: Path<(TraktActionsWrapper, StackString, i32, i32)>,
    user: LoggedUser,
) -> WarpResult<TraktWatchlistEpisodeActionResponse> {
    let Path((action, imdb_url, season, episode)) = paths;
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

#[derive(UtoipaResponse)]
#[response(description = "Trakt Calendar", content = "text/html")]
#[rustfmt::skip]
struct TraktCalendarResponse(HtmlBase::<StackString>);

#[utoipa::path(get, path = "/trakt/cal", responses(TraktCalendarResponse, Error))]
async fn trakt_cal(
    user: LoggedUser,
    state: State<Arc<AppState>>,
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

#[derive(UtoipaResponse)]
#[response(description = "Trakt Auth Url", content = "text/html")]
#[rustfmt::skip]
struct TraktAuthUrlResponse(HtmlBase::<StackString>);

#[utoipa::path(get, path = "/trakt/auth_url", responses(TraktAuthUrlResponse, Error))]
async fn trakt_auth_url(
    user: LoggedUser,
    state: State<Arc<AppState>>,
) -> WarpResult<TraktAuthUrlResponse> {
    let task = user
        .store_url_task(state.trakt.get_client(), &state.config, "/trakt/auth_url")
        .await;
    let url = state
        .trakt
        .get_auth_url()
        .await
        .map_err(Into::<Error>::into)?;
    task.await.ok();
    Ok(HtmlBase::new(url.as_str().into()).into())
}

#[derive(Serialize, Deserialize, ToSchema, IntoParams)]
// TraktCallbackRequest
struct TraktCallbackRequest {
    // Authorization Code
    #[schema(inline)]
    #[param(inline)]
    code: StackString,
    // CSRF State
    #[schema(inline)]
    #[param(inline)]
    state: StackString,
}

#[derive(UtoipaResponse)]
#[response(description = "Trakt Callback", content = "text/html")]
#[rustfmt::skip]
struct TraktCallbackResponse(HtmlBase::<&'static str>);

#[utoipa::path(
    get,
    path = "/trakt/callback",
    params(TraktCallbackRequest),
    responses(TraktCallbackResponse, Error)
)]
async fn trakt_callback(
    query: Query<TraktCallbackRequest>,
    user: LoggedUser,
    state: State<Arc<AppState>>,
) -> WarpResult<TraktCallbackResponse> {
    let task = user
        .store_url_task(state.trakt.get_client(), &state.config, "/trakt/callback")
        .await;
    let Query(query) = query;
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

#[derive(UtoipaResponse)]
#[response(description = "Trakt Refresh Auth", content = "text/html")]
#[rustfmt::skip]
struct TraktRefreshAuthResponse(HtmlBase::<&'static str>);

#[utoipa::path(
    get,
    path = "/trakt/refresh_auth",
    responses(TraktRefreshAuthResponse, Error)
)]
async fn refresh_auth(
    user: LoggedUser,
    state: State<Arc<AppState>>,
) -> WarpResult<TraktRefreshAuthResponse> {
    let task = user
        .store_url_task(
            state.trakt.get_client(),
            &state.config,
            "/trakt/refresh_auth",
        )
        .await;
    let access_token = state.trakt.read_auth_token().await?;
    if access_token.has_expired() {
        state
            .trakt
            .exchange_refresh_token(&access_token)
            .await
            .map_err(Into::<Error>::into)?;
    }
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
) -> WarpResult<StackString> {
    let mc = MovieCollection::new(config, pool, stdout);
    trakt.init().await?;
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

#[derive(Debug, Serialize, Deserialize, ToSchema)]
// PaginatedPlexEvent
struct PaginatedPlexEvent {
    pagination: Pagination,
    data: Vec<PlexEventWrapper>,
}

#[derive(UtoipaResponse)]
#[response(description = "Plex Events")]
#[rustfmt::skip]
struct PlexEventResponse(JsonBase::<PaginatedPlexEvent>);

#[utoipa::path(
    get,
    path = "/list/plex_event",
    params(PlexEventRequest),
    responses(PlexEventResponse, Error)
)]
async fn plex_events(
    query: Query<PlexEventRequest>,
    state: State<Arc<AppState>>,
    user: LoggedUser,
) -> WarpResult<PlexEventResponse> {
    let task = user
        .store_url_task(
            state.trakt.get_client(),
            &state.config,
            "/list/movie_collection",
        )
        .await;
    let Query(query) = query;
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

#[derive(Serialize, Deserialize, Debug, ToSchema)]
// PlexEventUpdateRequest
struct PlexEventUpdateRequest {
    events: Vec<PlexEventWrapper>,
}

#[derive(UtoipaResponse)]
#[response(
    description = "Update Plex Events",
    content = "text/html",
    status = "CREATED"
)]
#[rustfmt::skip]
struct PlexEventUpdateResponse(HtmlBase::<&'static str>);

#[utoipa::path(
    post,
    path = "/list/plex_event",
    request_body = PlexEventUpdateRequest,
    responses(PlexEventUpdateResponse, Error)
)]
async fn plex_events_update(
    state: State<Arc<AppState>>,
    user: LoggedUser,
    payload: Json<PlexEventUpdateRequest>,
) -> WarpResult<PlexEventUpdateResponse> {
    let task = user
        .store_url_task(state.trakt.get_client(), &state.config, "/list/plex_event")
        .await;
    let Json(payload) = payload;
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

#[derive(UtoipaResponse)]
#[response(description = "Plex Webhook", content = "text/html", status = "CREATED")]
#[rustfmt::skip]
struct PlexWebhookResponse(HtmlBase::<&'static str>);

#[utoipa::path(
    post,
    path = "/list/plex/webhook/{webhook_key}",
    params(("webhook_key" = Uuid, description = "Webhook Key")),
    request_body = PlexEventWrapper,
    responses(PlexWebhookResponse, Error)
)]
async fn plex_webhook(
    state: State<Arc<AppState>>,
    webhook_key: Path<Uuid>,
    form: Multipart,
) -> WarpResult<PlexWebhookResponse> {
    let Path(webhook_key) = webhook_key;
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
    mut form: Multipart,
    pool: &PgPool,
    config: &Config,
) -> Result<(), anyhow::Error> {
    let mut buf = Vec::new();
    if let Some(mut stream) = form.next_field().await? {
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

#[derive(UtoipaResponse)]
#[response(description = "Plex Event List", content = "text/html")]
#[rustfmt::skip]
struct PlexEventList(HtmlBase::<StackString>);

#[utoipa::path(
    get,
    path = "/list/plex",
    params(PlexEventRequest),
    responses(PlexEventList, Error)
)]
async fn plex_list(
    user: LoggedUser,
    state: State<Arc<AppState>>,
    query: Query<PlexEventRequest>,
) -> WarpResult<PlexEventList> {
    let task = user
        .store_url_task(state.trakt.get_client(), &state.config, "/list/plex")
        .await;
    let Query(query) = query;
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

#[derive(UtoipaResponse)]
#[response(description = "Plex Event Detail", content = "text/html")]
#[rustfmt::skip]
struct PlexEventDetail(HtmlBase::<StackString>);

#[utoipa::path(get, path = "/list/plex/{id}", params(("id" = Uuid, description = "ID"), PlexEventRequest), responses(PlexEventDetail, Error))]
async fn plex_detail(
    state: State<Arc<AppState>>,
    query: Query<PlexEventRequest>,
    user: LoggedUser,
    id: Path<Uuid>,
) -> WarpResult<PlexEventDetail> {
    let Path(id) = id;
    let Query(query) = query;
    let url = format_sstr!("/list/plex/{id}");
    let task = user
        .store_url_task(state.trakt.get_client(), &state.config, &url)
        .await;
    let body = if let Some(event) = PlexEvent::get_event_by_id(&state.db, id)
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

#[derive(Debug, Serialize, Deserialize, ToSchema)]
// PaginatedPlexFilename
struct PaginatedPlexFilename {
    pagination: Pagination,
    data: Vec<PlexFilenameWrapper>,
}

#[derive(UtoipaResponse)]
#[response(description = "Plex Filenames")]
#[rustfmt::skip]
struct PlexFilenameResponse(JsonBase::<PaginatedPlexFilename>);

#[utoipa::path(
    get,
    path = "/list/plex_filename",
    params(PlexFilenameRequest),
    responses(PlexFilenameResponse, Error)
)]
async fn plex_filename(
    query: Query<PlexFilenameRequest>,
    state: State<Arc<AppState>>,
    user: LoggedUser,
) -> WarpResult<PlexFilenameResponse> {
    let task = user
        .store_url_task(
            state.trakt.get_client(),
            &state.config,
            "/list/plex_filename",
        )
        .await;
    let Query(query) = query;
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

#[derive(Serialize, Deserialize, Debug, ToSchema)]
// PlexFilenameUpdateRequest
struct PlexFilenameUpdateRequest {
    filenames: Vec<PlexFilenameWrapper>,
}

#[derive(UtoipaResponse)]
#[response(
    description = "Update Plex Filenames",
    content = "text/html",
    status = "CREATED"
)]
#[rustfmt::skip]
struct PlexFilenameUpdateResponse(HtmlBase::<&'static str>);

#[utoipa::path(
    post,
    path = "/list/plex_filename",
    request_body = PlexFilenameUpdateRequest,
    responses(PlexFilenameUpdateResponse, Error)
)]
async fn plex_filename_update(
    state: State<Arc<AppState>>,
    user: LoggedUser,
    payload: Json<PlexFilenameUpdateRequest>,
) -> WarpResult<PlexFilenameUpdateResponse> {
    let task = user
        .store_url_task(
            state.trakt.get_client(),
            &state.config,
            "/list/plex_filename",
        )
        .await;
    let Json(payload) = payload;
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

#[derive(Debug, Serialize, Deserialize, ToSchema)]
// PaginatedPlexMetadata
struct PaginatedPlexMetadata {
    pagination: Pagination,
    data: Vec<PlexMetadataWrapper>,
}

#[derive(UtoipaResponse)]
#[response(description = "Plex Metadata")]
#[rustfmt::skip]
struct PlexMetadataResponse(JsonBase::<PaginatedPlexMetadata>);

#[utoipa::path(
    get,
    path = "/list/plex_metadata",
    params(PlexFilenameRequest),
    responses(PlexMetadataResponse, Error)
)]
async fn plex_metadata(
    query: Query<PlexFilenameRequest>,
    state: State<Arc<AppState>>,
    user: LoggedUser,
) -> WarpResult<PlexMetadataResponse> {
    let task = user
        .store_url_task(
            state.trakt.get_client(),
            &state.config,
            "/list/plex_metadata",
        )
        .await;
    let Query(query) = query;
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

#[derive(Serialize, Deserialize, Debug, ToSchema)]
// PlexMetadataUpdateRequest
struct PlexMetadataUpdateRequest {
    entries: Vec<PlexMetadataWrapper>,
}

#[derive(UtoipaResponse)]
#[response(
    description = "Update Plex Metadata",
    content = "text/html",
    status = "CREATED"
)]
#[rustfmt::skip]
struct PlexMetadataUpdateResponse(HtmlBase::<&'static str>);

#[utoipa::path(
    post,
    path = "/list/plex_metadata",
    request_body = PlexMetadataUpdateRequest,
    responses(PlexMetadataUpdateResponse, Error)
)]
async fn plex_metadata_update(
    state: State<Arc<AppState>>,
    user: LoggedUser,
    payload: Json<PlexMetadataUpdateRequest>,
) -> WarpResult<PlexMetadataUpdateResponse> {
    let task = user
        .store_url_task(
            state.trakt.get_client(),
            &state.config,
            "/list/plex_metadata",
        )
        .await;
    let Json(payload) = payload;
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

#[derive(Debug, Serialize, Deserialize, ToSchema)]
// PaginatedMusicCollection
struct PaginatedMusicCollection {
    pagination: Pagination,
    data: Vec<MusicCollectionWrapper>,
}

#[derive(UtoipaResponse)]
#[response(description = "Music Collection")]
#[rustfmt::skip]
struct MusicCollectionResponse(JsonBase::<PaginatedMusicCollection>);

#[utoipa::path(
    get,
    path = "/list/music_collection",
    params(PlexFilenameRequest),
    responses(MusicCollectionResponse, Error)
)]
async fn music_collection(
    query: Query<PlexFilenameRequest>,
    state: State<Arc<AppState>>,
    user: LoggedUser,
) -> WarpResult<MusicCollectionResponse> {
    let task = user
        .store_url_task(
            state.trakt.get_client(),
            &state.config,
            "/list/music_collection",
        )
        .await;
    let Query(query) = query;
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

#[derive(Serialize, Deserialize, Debug, ToSchema)]
struct MusicCollectionUpdateRequest {
    entries: Vec<MusicCollectionWrapper>,
}

#[derive(UtoipaResponse)]
#[response(
    description = "Update Music Collection",
    content = "text/html",
    status = "CREATED"
)]
#[rustfmt::skip]
struct MusicCollectionUpdateResponse(HtmlBase::<&'static str>);

#[utoipa::path(
    post,
    path = "/list/music_collection",
    request_body = MusicCollectionUpdateRequest,
    responses(MusicCollectionUpdateResponse, Error)
)]
async fn music_collection_update(
    state: State<Arc<AppState>>,
    user: LoggedUser,
    payload: Json<MusicCollectionUpdateRequest>,
) -> WarpResult<MusicCollectionUpdateResponse> {
    let task = user
        .store_url_task(
            state.trakt.get_client(),
            &state.config,
            "/list/music_collection",
        )
        .await;
    let Json(payload) = payload;
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

#[derive(UtoipaResponse)]
#[response(
    description = "Extract Subtitles",
    content = "text/html",
    status = "Created"
)]
#[rustfmt::skip]
struct ExtractSubtitlesResponse(HtmlBase::<StackString>);

#[utoipa::path(
    post,
    path = "/list/transcode/subtitle/{filename}/{index}/{suffix}",
    params(
        ("filename" = inline(StackString), description = "Filename"),
        ("index" = usize, description = "Index"),
        ("suffix" = inline(StackString), description = "Suffix"),
    ),
    responses(ExtractSubtitlesResponse, Error)
)]
async fn movie_queue_extract_subtitle(
    state: State<Arc<AppState>>,
    paths: Path<(StackString, usize, StackString)>,
    user: LoggedUser,
) -> WarpResult<ExtractSubtitlesResponse> {
    let Path((filename, index, suffix)) = paths;
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

pub fn get_full_path(app: &AppState) -> OpenApiRouter {
    let app = Arc::new(app.clone());

    OpenApiRouter::new()
        .routes(routes!(frontpage))
        .routes(routes!(scripts_js))
        .routes(routes!(find_new_episodes))
        .routes(routes!(tvshows))
        .routes(routes!(movie_queue_delete))
        .routes(routes!(movie_queue_transcode_status))
        .routes(routes!(movie_queue_transcode_status_file_list))
        .routes(routes!(movie_queue_transcode_status_procs))
        .routes(routes!(movie_queue_transcode_file))
        .routes(routes!(movie_queue_remcom_file))
        .routes(routes!(movie_queue_remcom_directory_file))
        .routes(routes!(movie_queue_transcode))
        .routes(routes!(movie_queue_transcode_directory))
        .routes(routes!(movie_queue_transcode_cleanup))
        .routes(routes!(movie_queue_play))
        .routes(routes!(imdb_episodes_route))
        .routes(routes!(imdb_episodes_update))
        .routes(routes!(imdb_ratings_set_source))
        .routes(routes!(imdb_ratings_route))
        .routes(routes!(imdb_ratings_update))
        .routes(routes!(movie_queue_route))
        .routes(routes!(movie_queue_update))
        .routes(routes!(movie_collection_route))
        .routes(routes!(movie_collection_update))
        .routes(routes!(imdb_show))
        .routes(routes!(last_modified_route))
        .routes(routes!(user))
        .routes(routes!(movie_queue))
        .routes(routes!(movie_queue_show))
        .routes(routes!(plex_webhook))
        .routes(routes!(plex_events))
        .routes(routes!(plex_events_update))
        .routes(routes!(plex_list))
        .routes(routes!(plex_detail))
        .routes(routes!(plex_filename))
        .routes(routes!(plex_filename_update))
        .routes(routes!(plex_metadata))
        .routes(routes!(plex_metadata_update))
        .routes(routes!(music_collection))
        .routes(routes!(music_collection_update))
        .routes(routes!(movie_queue_extract_subtitle))
        .routes(routes!(trakt_auth_url))
        .routes(routes!(trakt_callback))
        .routes(routes!(refresh_auth))
        .routes(routes!(trakt_cal))
        .routes(routes!(trakt_watchlist))
        .routes(routes!(trakt_watchlist_action))
        .routes(routes!(trakt_watched_seasons))
        .routes(routes!(trakt_watched_list))
        .routes(routes!(trakt_watched_action))
        .with_state(app)
}

#[derive(OpenApi)]
#[openapi(
    info(
        title = "Movie Queue WebApp",
        description = "Web Frontend for Movie Queue",
    ),
    components(schemas(
        LoggedUser,
        Pagination,
        LastModifiedResponseWrapper,
        TraktActionsWrapper,
        PlexEventTypeWrapper,
        PlexSectionTypeWrapper,
        OrderByWrapper,
    ))
)]
pub struct ApiDoc;
