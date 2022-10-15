#![allow(clippy::needless_pass_by_value)]

use anyhow::format_err;
use bytes::Buf;
use itertools::Itertools;
use log::error;
use maplit::hashmap;
use rweb::{get, multipart::FormData, post, Json, Query, Rejection, Schema};
use rweb_helper::{
    derive_rweb_schema, html_response::HtmlResponse as HtmlBase,
    json_response::JsonResponse as JsonBase, DateTimeType, RwebResponse, UuidWrapper,
};
use serde::{Deserialize, Serialize};
use stack_string::{format_sstr, StackString};
use std::{
    borrow::Borrow,
    collections::{HashMap, HashSet},
    fmt::Write,
    hash::{Hash, Hasher},
    path,
    path::PathBuf,
};
use stdout_channel::{MockStdout, StdoutChannel};
use tokio::fs::remove_file;
use tokio_stream::StreamExt;

use movie_collection_lib::{
    config::Config,
    date_time_wrapper::DateTimeWrapper,
    imdb_episodes::{ImdbEpisodes, ImdbSeason},
    imdb_ratings::ImdbRatings,
    make_list::FileLists,
    make_queue::movie_queue_http,
    movie_collection::{
        find_new_episodes_http_worker, LastModifiedResponse, MovieCollection, TvShowsResult,
    },
    movie_queue::{MovieQueueDB, MovieQueueResult},
    pgpool::PgPool,
    plex_events::{PlexEvent, PlexFilename},
    trakt_connection::TraktConnection,
    trakt_utils::{
        get_watched_shows_db, get_watchlist_shows_db_map, TraktActions, WatchListShow,
        WatchedEpisode, WatchedMovie,
    },
    transcode_service::{transcode_status, TranscodeService, TranscodeServiceRequest},
    tv_show_source::TvShowSource,
    utils::HBR,
};

use crate::{
    errors::ServiceError as Error,
    logged_user::LoggedUser,
    movie_queue_app::AppState,
    movie_queue_requests::{
        ImdbEpisodesUpdateRequest, ImdbRatingsSetSourceRequest, ImdbRatingsUpdateRequest,
        ImdbSeasonsRequest, ImdbShowRequest, MovieCollectionSyncRequest,
        MovieCollectionUpdateRequest, MovieQueueRequest, MovieQueueSyncRequest,
        MovieQueueUpdateRequest, ParseImdbRequest, WatchlistActionRequest,
    },
    ImdbEpisodesWrapper, ImdbRatingsWrapper, LastModifiedResponseWrapper,
    MovieCollectionRowWrapper, MovieQueueRowWrapper, PlexEventRequest, PlexEventWrapper,
    PlexFilenameRequest, PlexFilenameWrapper, TraktActionsWrapper, TvShowSourceWrapper,
};

pub type WarpResult<T> = Result<T, Rejection>;
pub type HttpResult<T> = Result<T, Error>;

fn movie_queue_body(patterns: &[StackString], entries: &[StackString]) -> StackString {
    let previous = r#"<a href="javascript:updateMainArticle('/list/tvshows')">Go Back</a><br>"#;

    let watchlist_url = if patterns.is_empty() {
        format_sstr!("/trakt/watchlist")
    } else {
        let patterns = patterns.join("_");
        format_sstr!("/trakt/watched/list/{patterns}")
    };
    let entries = entries.join("");
    format_sstr!(
        r#"{previous}<a href="javascript:updateMainArticle('{watchlist_url}')">Watch List</a><table border="0">{entries}</table>"#,
    )
}

async fn queue_body_resp(
    config: &Config,
    patterns: &[StackString],
    queue: &[MovieQueueResult],
    pool: &PgPool,
) -> HttpResult<StackString> {
    let mock_stdout = MockStdout::new();
    let stdout = StdoutChannel::with_mock_stdout(mock_stdout.clone(), mock_stdout.clone());

    let entries = movie_queue_http(queue, pool, config, &stdout).await?;
    let body = movie_queue_body(patterns, &entries);
    Ok(body)
}

#[derive(RwebResponse)]
#[response(description = "Movie Queue", content = "html")]
struct MovieQueueResponse(HtmlBase<StackString, Error>);

#[get("/list/full_queue")]
pub async fn movie_queue(
    #[filter = "LoggedUser::filter"] user: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<MovieQueueResponse> {
    let task = user
        .store_url_task(state.trakt.get_client(), &state.config, "/list/full_queue")
        .await;
    let req = MovieQueueRequest {
        patterns: Vec::new(),
    };
    let (queue, _) = req.process(&state.db, &state.config).await?;
    let body = queue_body_resp(&state.config, &[], &queue, &state.db).await?;
    task.await.ok();
    Ok(HtmlBase::new(body).into())
}

#[get("/list/queue/{path}")]
pub async fn movie_queue_show(
    path: StackString,
    #[filter = "LoggedUser::filter"] user: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<MovieQueueResponse> {
    let task = user
        .store_url_task(
            state.trakt.get_client(),
            &state.config,
            &format_sstr!("/list/queue/{path}"),
        )
        .await;
    let patterns = vec![path];

    let req = MovieQueueRequest { patterns };
    let (queue, patterns) = req.process(&state.db, &state.config).await?;
    let body = queue_body_resp(&state.config, &patterns, &queue, &state.db).await?;
    task.await.ok();
    Ok(HtmlBase::new(body).into())
}

#[derive(RwebResponse)]
#[response(description = "Delete Queue Entry", content = "html")]
struct DeleteMovieQueueResponse(HtmlBase<StackString, Error>);

#[get("/list/delete/{path}")]
pub async fn movie_queue_delete(
    path: StackString,
    #[filter = "LoggedUser::filter"] user: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<DeleteMovieQueueResponse> {
    let task = user
        .store_url_task(
            state.trakt.get_client(),
            &state.config,
            &format_sstr!("/list/delete/{path}"),
        )
        .await;
    let mock_stdout = MockStdout::new();
    let stdout = StdoutChannel::with_mock_stdout(mock_stdout.clone(), mock_stdout.clone());

    if path::Path::new(path.as_str()).exists() {
        MovieQueueDB::new(&state.config, &state.db, &stdout)
            .remove_from_queue_by_path(&path)
            .await
            .map_err(Into::<Error>::into)?;
    }
    task.await.ok();
    Ok(HtmlBase::new(path).into())
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
#[response(description = "Transcode Queue Item", content = "html")]
struct TranscodeQueueResponse(HtmlBase<StackString, Error>);

#[get("/list/transcode/queue/{path}")]
pub async fn movie_queue_transcode(
    path: StackString,
    #[filter = "LoggedUser::filter"] user: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<TranscodeQueueResponse> {
    let task = user
        .store_url_task(
            state.trakt.get_client(),
            &state.config,
            &format_sstr!("/list/transcode/queue/{path}"),
        )
        .await;
    let patterns = vec![path];

    let req = MovieQueueRequest { patterns };
    let (entries, _) = req.process(&state.db, &state.config).await?;
    let body = transcode_worker(&state.config, None, &entries, &state.db).await?;
    task.await.ok();
    Ok(HtmlBase::new(body).into())
}

#[get("/list/transcode/queue/{directory}/{file}")]
pub async fn movie_queue_transcode_directory(
    directory: StackString,
    file: StackString,
    #[filter = "LoggedUser::filter"] user: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<TranscodeQueueResponse> {
    let task = user
        .store_url_task(
            state.trakt.get_client(),
            &state.config,
            &format_sstr!("/list/transcode/queue/{directory}/{file}"),
        )
        .await;
    let patterns = vec![file];

    let req = MovieQueueRequest { patterns };
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

fn play_worker(
    config: &Config,
    full_path: &path::Path,
    last_url: Option<&str>,
) -> HttpResult<StackString> {
    let file_name = full_path
        .file_name()
        .ok_or_else(|| format_err!("Invalid path"))?
        .to_string_lossy();

    if let Some(partial_path) = &config.video_playback_path {
        let url = format_sstr!("/videos/partial/{file_name}");

        let body = format_sstr!(
            r#"
            {a}
            {file_name}<br>
            <video width="720" controls>
            <source src="{url}" type="video/mp4">
            Your browser does not support HTML5 video.
            </video>
        "#,
            a = if let Some(last_url) = last_url {
                format_sstr!(
                    r#"<input type="button" name="back" value="Back" onclick="updateMainArticle('{last_url}');"/><br>"#
                )
            } else {
                StackString::new()
            }
        );

        let partial_path = partial_path.join("videos").join("partial");
        let partial_path = partial_path.join(file_name.as_ref());
        if partial_path.exists() {
            std::fs::remove_file(&partial_path)?;
        }

        #[cfg(target_family = "unix")]
        std::os::unix::fs::symlink(&full_path, &partial_path).map_err(Into::<Error>::into)?;
        Ok(body)
    } else {
        Err(format_err!("video playback path does not exist").into())
    }
}

#[derive(RwebResponse)]
#[response(description = "Play Queue Item", content = "html")]
struct PlayQueueResponse(HtmlBase<StackString, Error>);

#[get("/list/play/{idx}")]
pub async fn movie_queue_play(
    idx: i32,
    #[filter = "LoggedUser::filter"] user: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<PlayQueueResponse> {
    let last_url = user
        .get_url(state.trakt.get_client(), &state.config)
        .await
        .map_err(Into::<Error>::into)?;
    let task = user
        .store_url_task(
            state.trakt.get_client(),
            &state.config,
            &format_sstr!("/list/play/{idx}"),
        )
        .await;

    let mock_stdout = MockStdout::new();
    let stdout = StdoutChannel::with_mock_stdout(mock_stdout.clone(), mock_stdout);

    let movie_path = MovieCollection::new(&state.config, &state.db, &stdout)
        .get_collection_path(idx)
        .await
        .map_err(Into::<Error>::into)?;

    let movie_path = path::Path::new(movie_path.as_str());
    let body = play_worker(
        &state.config,
        movie_path,
        last_url.as_ref().map(StackString::as_str),
    )?;
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
    let task = user
        .store_url_task(
            state.trakt.get_client(),
            &state.config,
            &format_sstr!("/list/imdb/{show}"),
        )
        .await;
    let query = query.into_inner();
    let req = ImdbShowRequest { show, query };
    let body = req.process(&state.db, &state.config).await?;
    task.await.ok();
    Ok(HtmlBase::new(body).into())
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
    let entries = entries.join("");
    format_sstr!(r#"{previous}<table border="0">{entries}</table>"#)
}

#[derive(Serialize, Deserialize, Schema)]
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

    let entries = find_new_episodes_http_worker(
        &state.config,
        &state.db,
        &stdout,
        query.shows,
        query.source.map(Into::into),
    )
    .await
    .map_err(Into::<Error>::into)?;

    let body = new_episode_worker(&entries);
    task.await.ok();
    Ok(HtmlBase::new(body).into())
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ImdbEpisodesSyncRequest {
    pub start_timestamp: DateTimeWrapper,
}

derive_rweb_schema!(ImdbEpisodesSyncRequest, _ImdbEpisodesSyncRequest);

#[derive(Schema)]
#[allow(dead_code)]
struct _ImdbEpisodesSyncRequest {
    #[schema(description = "Start Timestamp")]
    pub start_timestamp: DateTimeType,
}

#[derive(RwebResponse)]
#[response(description = "List Imdb Episodes")]
struct ListImdbEpisodesResponse(JsonBase<Vec<ImdbEpisodesWrapper>, Error>);

#[get("/list/imdb_episodes")]
pub async fn imdb_episodes_route(
    query: Query<ImdbEpisodesSyncRequest>,
    #[filter = "LoggedUser::filter"] user: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<ListImdbEpisodesResponse> {
    let query = query.into_inner();

    let task = user
        .store_url_task(
            state.trakt.get_client(),
            &state.config,
            "/list/imdb_episodes",
        )
        .await;

    let episodes =
        ImdbEpisodes::get_episodes_after_timestamp(query.start_timestamp.into(), &state.db)
            .await
            .map_err(Into::<Error>::into)?
            .into_iter()
            .map(Into::into)
            .collect();
    task.await.ok();
    Ok(JsonBase::new(episodes).into())
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
    pub start_timestamp: DateTimeWrapper,
}

derive_rweb_schema!(ImdbRatingsSyncRequest, _ImdbEpisodesSyncRequest);

#[derive(RwebResponse)]
#[response(description = "List Imdb Shows")]
struct ListImdbShowsResponse(JsonBase<Vec<ImdbRatingsWrapper>, Error>);

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

    let ratings = ImdbRatings::get_shows_after_timestamp(query.start_timestamp.into(), &state.db)
        .await
        .map_err(Into::<Error>::into)?
        .into_iter()
        .map(Into::into)
        .collect();
    task.await.ok();
    Ok(JsonBase::new(ratings).into())
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

#[derive(RwebResponse)]
#[response(description = "List Movie Queue Entries")]
struct ListMovieQueueResponse(JsonBase<Vec<MovieQueueRowWrapper>, Error>);

#[get("/list/movie_queue")]
pub async fn movie_queue_route(
    query: Query<MovieQueueSyncRequest>,
    #[filter = "LoggedUser::filter"] user: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<ListMovieQueueResponse> {
    let task = user
        .store_url_task(state.trakt.get_client(), &state.config, "/list/movie_queue")
        .await;
    let x = query
        .into_inner()
        .get_queue(&state.db, &state.config)
        .await?
        .into_iter()
        .map(Into::into)
        .collect();
    task.await.ok();
    Ok(JsonBase::new(x).into())
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

#[derive(RwebResponse)]
#[response(description = "List Movie Collection Entries")]
struct ListMovieCollectionResponse(JsonBase<Vec<MovieCollectionRowWrapper>, Error>);

#[get("/list/movie_collection")]
pub async fn movie_collection_route(
    query: Query<MovieCollectionSyncRequest>,
    #[filter = "LoggedUser::filter"] user: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<ListMovieCollectionResponse> {
    let task = user
        .store_url_task(state.trakt.get_client(), &state.config, "/list/movie_queue")
        .await;
    let x = query
        .into_inner()
        .get_collection(&state.db, &state.config)
        .await?
        .into_iter()
        .map(Into::into)
        .collect();
    task.await.ok();
    Ok(JsonBase::new(x).into())
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
    let body = HBR
        .render("index.html", &hashmap! {"BODY" => ""})
        .map_err(Into::<Error>::into)?;
    Ok(HtmlBase::new(body.into()).into())
}

type TvShowsMap = HashMap<StackString, (StackString, WatchListShow, Option<TvShowSource>)>;

#[derive(Debug, Default, Eq)]
struct ProcessShowItem {
    show: StackString,
    title: StackString,
    link: StackString,
    source: Option<TvShowSource>,
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

fn tvshows_worker(res1: TvShowsMap, tvshows: Vec<TvShowsResult>) -> StackString {
    let tvshows: HashSet<_> = tvshows
        .into_iter()
        .map(|s| {
            let item: ProcessShowItem = s.into();
            item
        })
        .collect();
    let watchlist: HashSet<_> = res1
        .into_iter()
        .map(|(link, (show, s, source))| {
            let item = ProcessShowItem {
                show,
                title: s.title,
                link: s.link,
                source,
            };
            debug_assert!(link.as_str() == item.link.as_str());
            item
        })
        .collect();

    let shows = process_shows(tvshows, watchlist);

    let previous = r#"
        <a href="javascript:updateMainArticle('/list/watchlist')">Go Back</a><br>
        <a href="javascript:updateMainArticle('/trakt/watchlist')">Watch List</a>
        <button name="remcomout" id="remcomoutput"> &nbsp; </button><br>
    "#;
    let shows = shows.join("");
    format_sstr!(r#"{previous}<table border="0">{shows}</table>"#)
}

#[derive(RwebResponse)]
#[response(description = "List TvShows", content = "html")]
struct ListTvShowsResponse(HtmlBase<StackString, Error>);

#[get("/list/tvshows")]
pub async fn tvshows(
    #[filter = "LoggedUser::filter"] user: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<ListTvShowsResponse> {
    let task = user
        .store_url_task(state.trakt.get_client(), &state.config, "/list/tvshows")
        .await;
    let mock_stdout = MockStdout::new();
    let stdout = StdoutChannel::with_mock_stdout(mock_stdout.clone(), mock_stdout.clone());

    let mc = MovieCollection::new(&state.config, &state.db, &stdout);
    let shows = mc.print_tv_shows().await.map_err(Into::<Error>::into)?;
    let show_map = get_watchlist_shows_db_map(&state.db)
        .await
        .map_err(Into::<Error>::into)?;
    let body = tvshows_worker(show_map, shows);
    task.await.ok();
    Ok(HtmlBase::new(body).into())
}

fn process_shows(
    tvshows: HashSet<ProcessShowItem>,
    watchlist: HashSet<ProcessShowItem>,
) -> Vec<StackString> {
    let watchlist_shows = watchlist
        .iter()
        .filter(|item| tvshows.get(item.link.as_str()).is_none());

    let mut shows: Vec<_> = tvshows.iter().chain(watchlist_shows).collect();
    shows.sort_by(|x, y| x.show.cmp(&y.show));

    let button_add = r#"<td><button type="submit" id="ID" onclick="watchlist_add('SHOW');">add to watchlist</button></td>"#;
    let button_rm = r#"<td><button type="submit" id="ID" onclick="watchlist_rm('SHOW');">remove from watchlist</button></td>"#;

    shows
        .into_iter()
        .map(|item| {
            let link = item.link.as_str();
            let title = &item.title;
            let show = &item.show;
            let has_watchlist = watchlist.contains(link);
            let mut watchlist_str = StackString::new();
            if has_watchlist {
                write!(watchlist_str, r#"<a href="javascript:updateMainArticle('/trakt/watched/list/{link}')">watchlist</a>"#).unwrap();
            }
            format_sstr!(
                r#"<tr><td>{ql}</td>
                <td><a href="https://www.imdb.com/title/{link}" target="_blank">imdb</a></td><td>{src}</td><td>{watchlist_str}</td><td>{sh}</td></tr>"#,
                ql=if tvshows.contains(link) {
                    format_sstr!(r#"<a href="javascript:updateMainArticle('/list/queue/{show}')">{title}</a>"#)
                } else {
                    format_sstr!(
                        r#"<a href="javascript:updateMainArticle('/trakt/watched/list/{link}')">{title}</a>"#
                    )
                },
                src = match item.source {
                    Some(TvShowSource::Netflix) => r#"<a href="https://netflix.com" target="_blank">netflix</a>"#,
                    Some(TvShowSource::Hulu) => r#"<a href="https://hulu.com" target="_blank">hulu</a>"#,
                    Some(TvShowSource::Amazon) => r#"<a href="https://amazon.com" target="_blank">amazon</a>"#,
                    _ => "",
                },
                sh = if has_watchlist {
                    button_rm.replace("SHOW", link)
                } else {
                    button_add.replace("SHOW", link)
                },
            )
        })
        .collect()
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
    let body = status.get_html().join("").into();
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
        status
            .get_local_file_html(&file_lists, &state.config)
            .join("")
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
    let body = status.get_procs_html().join("").into();
    Ok(HtmlBase::new(body).into())
}

#[derive(RwebResponse)]
#[response(description = "Transcode File", content = "html")]
struct TranscodeFileResponse(HtmlBase<StackString, Error>);

#[get("/list/transcode/file/{filename}")]
pub async fn movie_queue_transcode_file(
    filename: StackString,
    #[filter = "LoggedUser::filter"] user: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<TranscodeFileResponse> {
    let task = user
        .store_url_task(
            state.trakt.get_client(),
            &state.config,
            &format_sstr!("/list/transcode/file/{filename}"),
        )
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

#[get("/list/transcode/remcom/file/{filename}")]
pub async fn movie_queue_remcom_file(
    filename: StackString,
    #[filter = "LoggedUser::filter"] user: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<TranscodeFileResponse> {
    let task = user
        .store_url_task(
            state.trakt.get_client(),
            &state.config,
            &format_sstr!("/list/transcode/remcom/file/{filename}"),
        )
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

#[get("/list/transcode/remcom/directory/{directory}/{filename}")]
pub async fn movie_queue_remcom_directory_file(
    directory: StackString,
    filename: StackString,
    #[filter = "LoggedUser::filter"] user: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<TranscodeFileResponse> {
    let task = user
        .store_url_task(
            state.trakt.get_client(),
            &state.config,
            &format_sstr!("/list/transcode/remcom/directory/{directory}/{filename}"),
        )
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

#[get("/list/transcode/cleanup/{path}")]
pub async fn movie_queue_transcode_cleanup(
    path: StackString,
    #[filter = "LoggedUser::filter"] user: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<CleanupTranscodeFileResponse> {
    let task = user
        .store_url_task(
            state.trakt.get_client(),
            &state.config,
            &format_sstr!("/list/transcode/cleanup/{path}"),
        )
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
        let movie_path = movie_path.to_string_lossy();
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

fn watchlist_worker(
    shows: HashMap<StackString, (StackString, WatchListShow, Option<TvShowSource>)>,
) -> StackString {
    let mut shows: Vec<_> = shows
        .into_iter()
        .map(|(_, (_, s, source))| (s.title, s.link, source))
        .collect();

    shows.sort();

    let shows = shows
        .into_iter()
        .map(|(title, link, source)| {
            format_sstr!(
                r#"<tr><td>{li}</td><td>
                   <a href="https://www.imdb.com/title/{link}" target="_blank">imdb</a> {sl} </tr>"#,
                li=format_sstr!(
                    r#"<a href="javascript:updateMainArticle('/trakt/watched/list/{link}')">{title}</a>"#
                ),
                sl=format_sstr!(
                    r#"<td><form action="javascript:setSource('{link}', '{link}_source_id')">
                       <select id="{link}_source_id" onchange="setSource('{link}', '{link}_source_id');">
                       {options}
                       </select>
                       </form></td>
                    "#,
                    options = match source {
                        Some(TvShowSource::All) | None => {
                            r#"
                                <option value="all"></option>
                                <option value="amazon">Amazon</option>
                                <option value="hulu">Hulu</option>
                                <option value="netflix">Netflix</option>
                            "#
                        }
                        Some(TvShowSource::Amazon) => {
                            r#"
                                <option value="amazon">Amazon</option>
                                <option value="all"></option>
                                <option value="hulu">Hulu</option>
                                <option value="netflix">Netflix</option>
                            "#
                        }
                        Some(TvShowSource::Hulu) => {
                            r#"
                                <option value="hulu">Hulu</option>
                                <option value="all"></option>
                                <option value="amazon">Amazon</option>
                                <option value="netflix">Netflix</option>
                            "#
                        }
                        Some(TvShowSource::Netflix) => {
                            r#"
                                <option value="netflix">Netflix</option>
                                <option value="all"></option>
                                <option value="amazon">Amazon</option>
                                <option value="hulu">Hulu</option>
                            "#
                        }
                    },
                )
            )
        })
        .join("");

    let previous = r#"<a href="javascript:updateMainArticle('/list/tvshows')">Go Back</a><br>"#;
    format_sstr!(r#"{previous}<table border="0">{shows}</table>"#)
}

#[derive(RwebResponse)]
#[response(description = "Trakt Watchlist", content = "html")]
struct TraktWatchlistResponse(HtmlBase<StackString, Error>);

#[get("/trakt/watchlist")]
pub async fn trakt_watchlist(
    #[filter = "LoggedUser::filter"] user: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<TraktWatchlistResponse> {
    let task = user
        .store_url_task(state.trakt.get_client(), &state.config, "/trakt/watchlist")
        .await;
    let shows = get_watchlist_shows_db_map(&state.db)
        .await
        .map_err(Into::<Error>::into)?;
    let body = watchlist_worker(shows);
    task.await.ok();
    Ok(HtmlBase::new(body).into())
}

async fn watchlist_action_worker(
    trakt: &TraktConnection,
    action: TraktActions,
    imdb_url: &str,
) -> HttpResult<StackString> {
    trakt.init().await;
    let mut body = StackString::new();
    match action {
        TraktActions::Add => write!(body, "{}", trakt.add_watchlist_show(imdb_url).await?)?,
        TraktActions::Remove => write!(body, "{}", trakt.remove_watchlist_show(imdb_url).await?)?,
        _ => (),
    }
    Ok(body)
}

#[derive(RwebResponse)]
#[response(description = "Trakt Watchlist Action", content = "html")]
struct TraktWatchlistActionResponse(HtmlBase<StackString, Error>);

#[get("/trakt/watchlist/{action}/{imdb_url}")]
pub async fn trakt_watchlist_action(
    action: TraktActionsWrapper,
    imdb_url: StackString,
    #[filter = "LoggedUser::filter"] user: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<TraktWatchlistActionResponse> {
    let task = user
        .store_url_task(
            state.trakt.get_client(),
            &state.config,
            &format_sstr!("/trakt/watchlist/{action}/{imdb_url}"),
        )
        .await;
    let req = WatchlistActionRequest {
        action: action.into(),
        imdb_url,
    };
    let imdb_url = req.process(&state.db, &state.trakt).await?;
    let body = watchlist_action_worker(&state.trakt, action.into(), &imdb_url).await?;
    task.await.ok();
    Ok(HtmlBase::new(body).into())
}

fn trakt_watched_seasons_worker(link: &str, imdb_url: &str, entries: &[ImdbSeason]) -> StackString {
    let entries = entries
        .iter()
        .map(|s| {
            let show = &s.show;
            let season = s.season;
            let title = &s.title;
            let id = format_sstr!("watched_seasons_id_{show}_{link}_{season}");
            let button_add = format_sstr!(
                r#"
            <td>
            <button type="submit" id="{id}"
                onclick="imdb_update('{show}', '{link}', {season}, '/trakt/watched/list/{link}');"
                >update database</button></td>"#,
            );
            let nepisodes = s.nepisodes;
            format_sstr!(
                "<tr><td>{a}<td>{season}<td>{nepisodes}<td>{button_add}</tr>",
                a = format_sstr!(
                    r#"<a href="javascript:updateMainArticle('/trakt/watched/list/{imdb_url}/{season}')">{title}</t>"#,
                ),
            )
        })
        .join("");

    format_sstr!(
        r#"<a href="javascript:updateMainArticle('/trakt/watchlist')">Go Back</a><br><table border="0">{entries}</table>"#
    )
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
    let task = user
        .store_url_task(
            state.trakt.get_client(),
            &state.config,
            &format_sstr!("/trakt/watched/list/{imdb_url}"),
        )
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
    let body = trakt_watched_seasons_worker(&link, &imdb_url, &entries);
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
    let task = user
        .store_url_task(
            state.trakt.get_client(),
            &state.config,
            &format_sstr!("/trakt/watched/list/{imdb_url}/{season}"),
        )
        .await;
    let mock_stdout = MockStdout::new();
    let stdout = StdoutChannel::with_mock_stdout(mock_stdout.clone(), mock_stdout.clone());

    let body = watch_list_http_worker(&state.config, &state.db, &stdout, &imdb_url, season).await?;
    task.await.ok();
    Ok(HtmlBase::new(body).into())
}

#[derive(RwebResponse)]
#[response(description = "Trakt Watchlist Episode Action", content = "html")]
struct TraktWatchlistEpisodeActionResponse(HtmlBase<StackString, Error>);

#[get("/trakt/watched/{action}/{imdb_url}/{season}/{episode}")]
pub async fn trakt_watched_action(
    action: TraktActionsWrapper,
    imdb_url: StackString,
    season: i32,
    episode: i32,
    #[filter = "LoggedUser::filter"] user: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<TraktWatchlistEpisodeActionResponse> {
    let task = user
        .store_url_task(
            state.trakt.get_client(),
            &state.config,
            &format_sstr!("/trakt/watched/{action}/{imdb_url}/{season}/{episode}"),
        )
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

fn trakt_cal_worker(entries: &[StackString]) -> StackString {
    let previous = r#"<a href="javascript:updateMainArticle('/list/tvshows')">Go Back</a><br>"#;
    let entries = entries.join("");
    format_sstr!(r#"{previous}<table border="0">{entries}</table>"#)
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
    let entries = trakt_cal_http_worker(&state.trakt, &state.db).await?;
    let body = trakt_cal_worker(&entries);
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

async fn trakt_cal_http_worker(
    trakt: &TraktConnection,
    pool: &PgPool,
) -> Result<Vec<StackString>, Error> {
    let button_add = r#"<td><button type="submit" id="ID"
        onclick="imdb_update('SHOW', 'LINK', SEASON, '/trakt/cal');">update database</button></td>"#;
    trakt.init().await;
    let cal_list = trakt.get_calendar().await?;

    let mut lines = Vec::new();
    for cal in cal_list {
        let show = match ImdbRatings::get_show_by_link(&cal.link, pool).await? {
            Some(s) => s.show,
            None => "".into(),
        };
        let exists = if show.is_empty() {
            None
        } else {
            let idx_opt = ImdbEpisodes {
                show: show.clone(),
                season: cal.season,
                episode: cal.episode,
                ..ImdbEpisodes::default()
            }
            .get_index(pool)
            .await?;

            match idx_opt {
                Some(idx) => ImdbEpisodes::from_index(idx, pool).await?,
                None => None,
            }
        };
        let season_str = StackString::from_display(cal.season);
        let episode_str = StackString::from_display(cal.episode);
        let cal_link = &cal.link;
        let show = &cal.show;
        let mut button = StackString::new();
        if exists.is_none() {
            write!(
                button,
                "{}",
                button_add
                    .replace("SHOW", show)
                    .replace("LINK", cal_link)
                    .replace("SEASON", &season_str)
            )?;
        }
        let airdate = cal.airdate;
        let line = format_sstr!(
            "<tr><td>{a}</td><td>{b}</td><td>{c}</td><td>{airdate}</td>{button}</tr>",
            a = format_sstr!(
                r#"<a href="javascript:updateMainArticle('/trakt/watched/list/{cal_link}/{season_str}')">{show}</a>"#
            ),
            b = format_sstr!(
                r#"<a href="https://www.imdb.com/title/{cal_link}" target="_blank">imdb</a>"#
            ),
            c = if let Some(link) = cal.ep_link {
                format_sstr!(
                    r#"<a href="https://www.imdb.com/title/{link}" target="_blank">{season_str} {episode_str}</a>"#
                )
            } else if let Some(link) = exists.as_ref() {
                format_sstr!(
                    r#"<a href="https://www.imdb.com/title/{link}" target="_blank">{season_str} {episode_str}</a>"#
                )
            } else {
                format_sstr!("{season_str} {episode_str}")
            },
        );
        lines.push(line);
    }
    Ok(lines)
}

async fn watch_list_http_worker(
    config: &Config,
    pool: &PgPool,
    stdout: &StdoutChannel<StackString>,
    imdb_url: &str,
    season: i32,
) -> HttpResult<StackString> {
    let button_add = r#"<button type="submit" id="ID"
        onclick="watched_add('SHOW', SEASON, EPISODE);">add to watched</button>"#;
    let button_rm = r#"<button type="submit" id="ID" 
        onclick="watched_rm('SHOW', SEASON, EPISODE);">remove from watched</button>"#;

    let mc = MovieCollection::new(config, pool, stdout);
    let mq = MovieQueueDB::new(config, pool, stdout);

    let show = ImdbRatings::get_show_by_link(imdb_url, pool)
        .await?
        .ok_or_else(|| format_err!("Show Doesn't exist"))?;
    let show_str = &show.show;
    let link_str = &show.link;

    let watched_episodes_db: HashSet<i32> = get_watched_shows_db(pool, show_str, Some(season))
        .await?
        .into_iter()
        .map(|s| s.episode)
        .collect();

    let queue: HashMap<(StackString, i32, i32), _> = mq
        .print_movie_queue(&[show_str.as_str()])
        .await?
        .into_iter()
        .filter_map(|s| match &s.show {
            Some(show) => match s.season {
                Some(season) => match s.episode {
                    Some(episode) => Some(((show.clone(), season, episode), s)),
                    None => None,
                },
                None => None,
            },
            None => None,
        })
        .collect();

    let entries: Vec<_> = mc.print_imdb_episodes(show_str, Some(season)).await?;

    let mut collection_idx_map = HashMap::new();
    let mut collection_metadata_map = HashMap::new();
    for r in &entries {
        if let Some(row) = queue.get(&(show_str.clone(), season, r.episode)) {
            if let Some(index) = mc.get_collection_index(&row.path).await? {
                collection_idx_map.insert(r.episode, index);
                if let Some(metadata_key) = mc.get_plex_metadata_key(index).await? {
                    collection_metadata_map.insert(index, metadata_key);
                }
            }
        }
    }

    let entries = entries
        .iter()
        .map(|s| {
            let eptitle = &s.eptitle;
            let entry = if let Some(collection_idx) = collection_idx_map.get(&s.episode) {
                let host = config.plex_host.as_ref();
                let server = config.plex_server.as_ref();
                if let (Some(metadata_key), Some(host), Some(server)) = (collection_metadata_map.get(collection_idx), host, server) {
                    format_sstr!(
                        r#"<a href="http://{host}:32400/web/index.html#!/server/{server}/details?key={metadata_key}" target="_blank">{eptitle}</a>"#,
                    )
                } else {
                    format_sstr!(
                        r#"<a href="javascript:updateMainArticle('{a}');">{eptitle}</a>"#,
                        a=format_sstr!("/list/play/{collection_idx}"),
                    )
                }
            } else {
                format_sstr!("{eptitle}")
            };
            let season_str = StackString::from_display(season);
            let episode_str = StackString::from_display(s.episode);
            let airdate = s.airdate;
            let show_rating = show.rating.as_ref().unwrap_or(&-1.0);
            let ep_rating = &s.rating;
            let ep_url = &s.epurl;
            format_sstr!(
                "<tr><td>{show_str}</td><td><td>{entry}</td><td>{a}</td><td>{b}</td><td>{airdate}</td><td>{c}</td></tr>",
                a=format_sstr!(
                    r#"<a href="https://www.imdb.com/title/{ep_url}" target="_blank">s{season} ep{episode_str}</a>"#,
                ),
                b=format_sstr!(
                    "rating: {ep_rating:0.1} / {show_rating:0.1}",
                ),
                c=if watched_episodes_db.contains(&s.episode) {
                    button_rm
                        .replace("SHOW", link_str)
                        .replace("SEASON", &season_str)
                        .replace("EPISODE", &episode_str)
                } else {
                    button_add
                        .replace("SHOW", link_str)
                        .replace("SEASON", &season_str)
                        .replace("EPISODE", &episode_str)
                }
            )
        })
        .join("\n");

    let previous = format_sstr!(
        r#"<a href="javascript:updateMainArticle('/trakt/watched/list/{imdb_url}')">Go Back</a><br>"#
    );
    let buttons = format_sstr!(
        r#"
        <button name="remcomout" id="remcomoutput"> &nbsp; </button>
        <button type="submit" id="ID"
            onclick="imdb_update('{show_str}', '{link_str}', {season},
            '/trakt/watched/list/{link_str}/{season}');"
            >update database</button><br>"#
    );

    let entries = format_sstr!(r#"{previous}{buttons}<table border="0">{entries}</table>"#);
    Ok(entries)
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

#[derive(RwebResponse)]
#[response(description = "Plex Events")]
struct PlexEventResponse(JsonBase<Vec<PlexEventWrapper>, Error>);

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
    let events = PlexEvent::get_events(
        &state.db,
        query.start_timestamp.map(Into::into),
        query.event_type.map(Into::into),
        query.offset,
        query.limit,
    )
    .await
    .map_err(Into::<Error>::into)?
    .into_iter()
    .map(Into::into)
    .collect();
    task.await.ok();
    Ok(JsonBase::new(events).into())
}

#[derive(Serialize, Deserialize, Debug, Schema)]
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
    let body = PlexEvent::get_event_http(
        &state.db,
        &state.config,
        query.start_timestamp.map(Into::into),
        query.event_type.map(Into::into),
        query.offset,
        query.limit,
    )
    .await
    .map_err(Into::<Error>::into)?
    .join("\n");
    let body = format_sstr!(
        r#"
            <table border="1" class="dataframe">
            <thead><tr>
            <th>Time</th>
            <th>Event Type</th>
            <th>Item Type</th>
            <th>Section</th>
            <th>Title</th></tr></thead>
            <tbody>{body}</tbody>
            </table>
        "#
    );
    task.await.ok();
    Ok(HtmlBase::new(body).into())
}

#[derive(RwebResponse)]
#[response(description = "Plex Filenames")]
struct PlexFilenameResponse(JsonBase<Vec<PlexFilenameWrapper>, Error>);

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
    let filenames = PlexFilename::get_filenames(
        &state.db,
        query.start_timestamp.map(Into::into),
        query.offset,
        query.limit,
    )
    .await
    .map_err(Into::<Error>::into)?
    .into_iter()
    .map(Into::into)
    .collect();
    task.await.ok();
    Ok(JsonBase::new(filenames).into())
}

#[derive(Serialize, Deserialize, Debug, Schema)]
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
        if PlexFilename::get_by_key(&state.db, &filename.metadata_key)
            .await
            .map_err(Into::<Error>::into)?
            .is_none()
        {
            let filename: PlexFilename = filename.into();
            filename
                .insert(&state.db)
                .await
                .map_err(Into::<Error>::into)?;
        }
    }
    task.await.ok();
    Ok(HtmlBase::new("Success").into())
}
