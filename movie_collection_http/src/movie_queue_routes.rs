#![allow(clippy::needless_pass_by_value)]

use anyhow::format_err;
use bytes::Buf;
use itertools::Itertools;
use log::error;
use maplit::hashmap;
use rweb::{get, multipart::FormData, post, Json, Query, Rejection, Schema};
use rweb_helper::{
    html_response::HtmlResponse as HtmlBase, json_response::JsonResponse as JsonBase, RwebResponse,
};
use serde::{Deserialize, Serialize};
use stack_string::StackString;
use std::{
    borrow::Borrow,
    collections::{HashMap, HashSet},
    hash::{Hash, Hasher},
    path,
    path::PathBuf,
    sync::Arc,
    time::Duration,
};
use stdout_channel::{MockStdout, StdoutChannel};
use tokio::{fs::remove_file, time::timeout};
use tokio_stream::StreamExt;

use movie_collection_lib::{
    config::Config,
    imdb_episodes::ImdbEpisodes,
    imdb_ratings::ImdbRatings,
    make_list::FileLists,
    make_queue::movie_queue_http,
    movie_collection::{ImdbSeason, MovieCollection, TvShowsResult},
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
    datetime_wrapper::DateTimeWrapper,
    errors::ServiceError as Error,
    logged_user::LoggedUser,
    movie_queue_app::AppState,
    movie_queue_requests::{
        FindNewEpisodeRequest, ImdbEpisodesSyncRequest, ImdbEpisodesUpdateRequest,
        ImdbRatingsSetSourceRequest, ImdbRatingsSyncRequest, ImdbRatingsUpdateRequest,
        ImdbSeasonsRequest, ImdbShowRequest, LastModifiedRequest, MovieCollectionSyncRequest,
        MovieCollectionUpdateRequest, MoviePathRequest, MovieQueueRequest, MovieQueueSyncRequest,
        MovieQueueUpdateRequest, ParseImdbRequest, WatchlistActionRequest,
    },
    uuid_wrapper::UuidWrapper,
    ImdbEpisodesWrapper, ImdbRatingsWrapper, LastModifiedResponseWrapper,
    MovieCollectionRowWrapper, MovieQueueRowWrapper, PlexEventTypeWrapper, PlexEventWrapper,
    PlexFilenameWrapper, TraktActionsWrapper,
};

pub type WarpResult<T> = Result<T, Rejection>;
pub type HttpResult<T> = Result<T, Error>;

fn movie_queue_body(patterns: &[StackString], entries: &[StackString]) -> StackString {
    let previous = r#"<a href="javascript:updateMainArticle('/list/tvshows')">Go Back</a><br>"#;

    let watchlist_url = if patterns.is_empty() {
        "/trakt/watchlist".to_string()
    } else {
        format!("/trakt/watched/list/{}", patterns.join("_"))
    };

    let entries = format!(
        r#"{}<a href="javascript:updateMainArticle('{}')">Watch List</a><table border="0">{}</table>"#,
        previous,
        watchlist_url,
        entries.join("")
    );

    entries.into()
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
    let body = movie_queue_body(&patterns, &entries);
    Ok(body)
}

#[derive(RwebResponse)]
#[response(description = "Movie Queue", content = "html")]
struct MovieQueueResponse(HtmlBase<String, Error>);

#[get("/list/full_queue")]
pub async fn movie_queue(
    #[cookie = "jwt"] _: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<MovieQueueResponse> {
    let req = MovieQueueRequest {
        patterns: Vec::new(),
    };
    let (queue, _) = req.handle(&state.db, &state.config).await?;
    let body: String = queue_body_resp(&state.config, &[], &queue, &state.db)
        .await?
        .into();
    Ok(HtmlBase::new(body).into())
}

#[get("/list/queue/{path}")]
pub async fn movie_queue_show(
    path: StackString,
    #[cookie = "jwt"] _: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<MovieQueueResponse> {
    let patterns = vec![path];

    let req = MovieQueueRequest { patterns };
    let (queue, patterns) = req.handle(&state.db, &state.config).await?;
    let body: String = queue_body_resp(&state.config, &patterns, &queue, &state.db)
        .await?
        .into();
    Ok(HtmlBase::new(body).into())
}

#[derive(RwebResponse)]
#[response(description = "Delete Queue Entry", content = "html")]
struct DeleteMovieQueueResponse(HtmlBase<String, Error>);

#[get("/list/delete/{path}")]
pub async fn movie_queue_delete(
    path: StackString,
    #[cookie = "jwt"] _: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<DeleteMovieQueueResponse> {
    let mock_stdout = MockStdout::new();
    let stdout = StdoutChannel::with_mock_stdout(mock_stdout.clone(), mock_stdout.clone());

    if path::Path::new(path.as_str()).exists() {
        MovieQueueDB::new(&state.config, &state.db, &stdout)
            .remove_from_queue_by_path(&path)
            .await
            .map_err(Into::<Error>::into)?;
    }
    let body: String = path.into();
    Ok(HtmlBase::new(body).into())
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
        output.push(format!("{:?}", payload));
        output.push(payload.publish_to_cli(config).await?.into());
    }
    Ok(output.join("").into())
}

#[derive(RwebResponse)]
#[response(description = "Transcode Queue Item", content = "html")]
struct TranscodeQueueResponse(HtmlBase<String, Error>);

#[get("/list/transcode/queue/{path}")]
pub async fn movie_queue_transcode(
    path: StackString,
    #[cookie = "jwt"] _: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<TranscodeQueueResponse> {
    let patterns = vec![path];

    let req = MovieQueueRequest { patterns };
    let (entries, _) = req.handle(&state.db, &state.config).await?;
    let body: String = transcode_worker(&state.config, None, &entries, &state.db)
        .await?
        .into();
    Ok(HtmlBase::new(body).into())
}

#[get("/list/transcode/queue/{directory}/{file}")]
pub async fn movie_queue_transcode_directory(
    directory: StackString,
    file: StackString,
    #[cookie = "jwt"] _: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<TranscodeQueueResponse> {
    let patterns = vec![file];

    let req = MovieQueueRequest { patterns };
    let (entries, _) = req.handle(&state.db, &state.config).await?;
    let body: String = transcode_worker(
        &state.config,
        Some(path::Path::new(directory.as_str())),
        &entries,
        &state.db,
    )
    .await?
    .into();
    Ok(HtmlBase::new(body).into())
}

fn play_worker(config: &Config, full_path: &path::Path) -> HttpResult<String> {
    let file_name = full_path
        .file_name()
        .ok_or_else(|| format_err!("Invalid path"))?
        .to_string_lossy();

    if let Some(partial_path) = &config.video_playback_path {
        let url = format!("/videos/partial/{}", file_name);

        let body = format!(
            r#"
            {}<br>
            <video width="720" controls>
            <source src="{}" type="video/mp4">
            Your browser does not support HTML5 video.
            </video>
        "#,
            file_name, url
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
struct PlayQueueResponse(HtmlBase<String, Error>);

#[get("/list/play/{idx}")]
pub async fn movie_queue_play(
    idx: i32,
    #[cookie = "jwt"] _: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<PlayQueueResponse> {
    let req = MoviePathRequest { idx };
    let movie_path = req.handle(&state.db, &state.config).await?;
    let movie_path = path::Path::new(movie_path.as_str());
    let body = play_worker(&state.config, movie_path)?;
    Ok(HtmlBase::new(body).into())
}

#[derive(RwebResponse)]
#[response(description = "List Imdb Show", content = "html")]
struct ListImdbResponse(HtmlBase<String, Error>);

#[get("/list/imdb/{show}")]
pub async fn imdb_show(
    show: StackString,
    query: Query<ParseImdbRequest>,
    #[cookie = "jwt"] _: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<ListImdbResponse> {
    let query = query.into_inner();
    let req = ImdbShowRequest { show, query };
    let body: String = req.handle(&state.db, &state.config).await?.into();
    Ok(HtmlBase::new(body).into())
}

fn new_episode_worker(entries: &[StackString]) -> String {
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
}

#[derive(RwebResponse)]
#[response(description = "List Calendar", content = "html")]
struct ListCalendarResponse(HtmlBase<String, Error>);

#[get("/list/cal")]
pub async fn find_new_episodes(
    query: Query<FindNewEpisodeRequest>,
    #[cookie = "jwt"] _: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<ListCalendarResponse> {
    let entries = query.into_inner().handle(&state.db, &state.config).await?;
    let body = new_episode_worker(&entries);
    Ok(HtmlBase::new(body).into())
}

#[derive(RwebResponse)]
#[response(description = "List Imdb Episodes")]
struct ListImdbEpisodesResponse(JsonBase<Vec<ImdbEpisodesWrapper>, Error>);

#[get("/list/imdb_episodes")]
pub async fn imdb_episodes_route(
    query: Query<ImdbEpisodesSyncRequest>,
    #[cookie = "jwt"] _: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<ListImdbEpisodesResponse> {
    let x = query
        .into_inner()
        .handle(&state.db)
        .await?
        .into_iter()
        .map(Into::into)
        .collect();
    Ok(JsonBase::new(x).into())
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
    #[cookie = "jwt"] _: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<ImdbEpisodesUpdateResponse> {
    episodes.into_inner().handle(&state.db).await?;
    Ok(HtmlBase::new("Success").into())
}

#[derive(RwebResponse)]
#[response(description = "List Imdb Shows")]
struct ListImdbShowsResponse(JsonBase<Vec<ImdbRatingsWrapper>, Error>);

#[get("/list/imdb_ratings")]
pub async fn imdb_ratings_route(
    query: Query<ImdbRatingsSyncRequest>,
    #[cookie = "jwt"] _: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<ListImdbShowsResponse> {
    let x = query
        .into_inner()
        .handle(&state.db)
        .await?
        .into_iter()
        .map(Into::into)
        .collect();
    Ok(JsonBase::new(x).into())
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
    #[cookie = "jwt"] _: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<UpdateImdbShowsResponse> {
    shows.into_inner().handle(&state.db).await?;
    Ok(HtmlBase::new("Success").into())
}

#[derive(RwebResponse)]
#[response(description = "Imdb Show Set Source", content = "html")]
struct ImdbSetSourceResponse(HtmlBase<&'static str, Error>);

#[get("/list/imdb_ratings/set_source")]
pub async fn imdb_ratings_set_source(
    query: Query<ImdbRatingsSetSourceRequest>,
    #[cookie = "jwt"] _: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<ImdbSetSourceResponse> {
    query.into_inner().handle(&state.db).await?;
    Ok(HtmlBase::new("Success").into())
}

#[derive(RwebResponse)]
#[response(description = "List Movie Queue Entries")]
struct ListMovieQueueResponse(JsonBase<Vec<MovieQueueRowWrapper>, Error>);

#[get("/list/movie_queue")]
pub async fn movie_queue_route(
    query: Query<MovieQueueSyncRequest>,
    #[cookie = "jwt"] _: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<ListMovieQueueResponse> {
    let x = query
        .into_inner()
        .handle(&state.db, &state.config)
        .await?
        .into_iter()
        .map(Into::into)
        .collect();
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
    #[cookie = "jwt"] _: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<UpdateMovieQueueResponse> {
    queue.into_inner().handle(&state.db, &state.config).await?;
    Ok(HtmlBase::new("Success").into())
}

#[derive(RwebResponse)]
#[response(description = "List Movie Collection Entries")]
struct ListMovieCollectionResponse(JsonBase<Vec<MovieCollectionRowWrapper>, Error>);

#[get("/list/movie_collection")]
pub async fn movie_collection_route(
    query: Query<MovieCollectionSyncRequest>,
    #[cookie = "jwt"] _: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<ListMovieCollectionResponse> {
    let x = query
        .into_inner()
        .handle(&state.db, &state.config)
        .await?
        .into_iter()
        .map(Into::into)
        .collect();
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
    #[cookie = "jwt"] _: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<UpdateMovieCollectionResponse> {
    collection
        .into_inner()
        .handle(&state.db, &state.config)
        .await?;
    Ok(HtmlBase::new("Success").into())
}

#[derive(RwebResponse)]
#[response(description = "Database Entries Last Modified Time")]
struct ListLastModifiedResponse(JsonBase<Vec<LastModifiedResponseWrapper>, Error>);

#[get("/list/last_modified")]
pub async fn last_modified_route(
    #[cookie = "jwt"] _: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<ListLastModifiedResponse> {
    let req = LastModifiedRequest {};
    let x = req
        .handle(&state.db)
        .await?
        .into_iter()
        .map(Into::into)
        .collect();
    Ok(JsonBase::new(x).into())
}

#[derive(RwebResponse)]
#[response(description = "Frontpage", content = "html")]
struct FrontpageResponse(HtmlBase<String, Error>);

#[allow(clippy::unused_async)]
#[get("/list/index.html")]
pub async fn frontpage(#[cookie = "jwt"] _: LoggedUser) -> WarpResult<FrontpageResponse> {
    let body = HBR
        .render("index.html", &hashmap! {"BODY" => ""})
        .map_err(Into::<Error>::into)?;
    Ok(HtmlBase::new(body).into())
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

    format!(
        r#"{}<table border="0">{}</table>"#,
        previous,
        shows.join("")
    )
    .into()
}

#[derive(RwebResponse)]
#[response(description = "List TvShows", content = "html")]
struct ListTvShowsResponse(HtmlBase<String, Error>);

#[get("/list/tvshows")]
pub async fn tvshows(
    #[cookie = "jwt"] _: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<ListTvShowsResponse> {
    let mock_stdout = MockStdout::new();
    let stdout = StdoutChannel::with_mock_stdout(mock_stdout.clone(), mock_stdout.clone());

    let mc = MovieCollection::new(&state.config, &state.db, &stdout);
    let shows = mc.print_tv_shows().await.map_err(Into::<Error>::into)?;
    let show_map = get_watchlist_shows_db_map(&state.db)
        .await
        .map_err(Into::<Error>::into)?;
    let body: String = tvshows_worker(show_map, shows).into();
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
            let has_watchlist = watchlist.contains(item.link.as_str());
            format!(
                r#"<tr><td>{}</td>
                <td><a href="https://www.imdb.com/title/{}" target="_blank">imdb</a></td><td>{}</td><td>{}</td><td>{}</td></tr>"#,
                if tvshows.contains(item.link.as_str()) {
                    format!(r#"<a href="javascript:updateMainArticle('/list/queue/{}')">{}</a>"#, item.show, item.title)
                } else {
                    format!(
                        r#"<a href="javascript:updateMainArticle('/trakt/watched/list/{}')">{}</a>"#,
                        item.link, item.title
                    )
                },
                item.link,
                match item.source {
                    Some(TvShowSource::Netflix) => r#"<a href="https://netflix.com" target="_blank">netflix</a>"#,
                    Some(TvShowSource::Hulu) => r#"<a href="https://hulu.com" target="_blank">hulu</a>"#,
                    Some(TvShowSource::Amazon) => r#"<a href="https://amazon.com" target="_blank">amazon</a>"#,
                    _ => "",
                },
                if has_watchlist {
                    format!(r#"<a href="javascript:updateMainArticle('/trakt/watched/list/{}')">watchlist</a>"#, item.link)
                } else {
                    "".to_string()
                },
                if has_watchlist {
                    button_rm.replace("SHOW", &item.link)
                } else {
                    button_add.replace("SHOW", &item.link)
                },
            ).into()
        })
        .collect()
}

#[derive(RwebResponse)]
#[response(description = "Logged in User")]
struct UserResponse(JsonBase<LoggedUser, Error>);

#[allow(clippy::unused_async)]
#[get("/list/user")]
pub async fn user(#[cookie = "jwt"] user: LoggedUser) -> WarpResult<UserResponse> {
    Ok(JsonBase::new(user).into())
}

#[derive(RwebResponse)]
#[response(description = "Transcode Status", content = "html")]
struct TranscodeStatusResponse(HtmlBase<String, Error>);

#[get("/list/transcode/status")]
pub async fn movie_queue_transcode_status(
    #[cookie = "jwt"] _: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<TranscodeStatusResponse> {
    let config = state.config.clone();
    let task = timeout(Duration::from_secs(10), FileLists::get_file_lists(&config));
    let status = transcode_status(&state.config)
        .await
        .map_err(Into::<Error>::into)?;
    let file_lists = task
        .await
        .map_or_else(|_| FileLists::default(), Result::unwrap);
    let body = status.get_html(&file_lists, &state.config).join("");
    Ok(HtmlBase::new(body).into())
}

#[derive(RwebResponse)]
#[response(description = "Transcode File", content = "html")]
struct TranscodeFileResponse(HtmlBase<String, Error>);

#[get("/list/transcode/file/{filename}")]
pub async fn movie_queue_transcode_file(
    filename: StackString,
    #[cookie = "jwt"] _: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<TranscodeFileResponse> {
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
    let body: String = req
        .publish_to_cli(&state.config)
        .await
        .map_err(Into::<Error>::into)?
        .into();
    Ok(HtmlBase::new(body).into())
}

#[get("/list/transcode/remcom/file/{filename}")]
pub async fn movie_queue_remcom_file(
    filename: StackString,
    #[cookie = "jwt"] _: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<TranscodeFileResponse> {
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
    let body: String = req
        .publish_to_cli(&state.config)
        .await
        .map_err(Into::<Error>::into)?
        .into();
    Ok(HtmlBase::new(body).into())
}

#[get("/list/transcode/remcom/directory/{directory}/{filename}")]
pub async fn movie_queue_remcom_directory_file(
    directory: StackString,
    filename: StackString,
    #[cookie = "jwt"] _: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<TranscodeFileResponse> {
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
    let body: String = req
        .publish_to_cli(&state.config)
        .await
        .map_err(Into::<Error>::into)?
        .into();
    Ok(HtmlBase::new(body).into())
}

#[derive(RwebResponse)]
#[response(description = "Cleanup Transcode File", content = "html")]
struct CleanupTranscodeFileResponse(HtmlBase<String, Error>);

#[get("/list/transcode/cleanup/{path}")]
pub async fn movie_queue_transcode_cleanup(
    path: StackString,
    #[cookie = "jwt"] _: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<CleanupTranscodeFileResponse> {
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
        format!("Removed {}", movie_path.to_string_lossy())
    } else if tmp_path.exists() {
        remove_file(&tmp_path).await.map_err(Into::<Error>::into)?;
        format!("Removed {}", tmp_path.to_string_lossy())
    } else {
        format!("File not found {}", path)
    };
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
            format!(
                r#"<tr><td>{}</td><td>
                   <a href="https://www.imdb.com/title/{}" target="_blank">imdb</a> {} </tr>"#,
                format!(
                    r#"<a href="javascript:updateMainArticle('/trakt/watched/list/{}')">{}</a>"#,
                    link, title
                ),
                link,
                format!(
                    r#"<td><form action="javascript:setSource('{link}', '{link}_source_id')">
                       <select id="{link}_source_id" onchange="setSource('{link}', '{link}_source_id');">
                       {options}
                       </select>
                       </form></td>
                    "#,
                    link = link,
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
    format!(r#"{}<table border="0">{}</table>"#, previous, shows).into()
}

#[derive(RwebResponse)]
#[response(description = "Trakt Watchlist", content = "html")]
struct TraktWatchlistResponse(HtmlBase<String, Error>);

#[get("/trakt/watchlist")]
pub async fn trakt_watchlist(
    #[cookie = "jwt"] _: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<TraktWatchlistResponse> {
    let shows = get_watchlist_shows_db_map(&state.db)
        .await
        .map_err(Into::<Error>::into)?;
    let body: String = watchlist_worker(shows).into();
    Ok(HtmlBase::new(body).into())
}

async fn watchlist_action_worker(
    trakt: &TraktConnection,
    action: TraktActions,
    imdb_url: &str,
) -> HttpResult<StackString> {
    trakt.init().await;
    let body = match action {
        TraktActions::Add => trakt.add_watchlist_show(imdb_url).await?.to_string(),
        TraktActions::Remove => trakt.remove_watchlist_show(imdb_url).await?.to_string(),
        _ => "".to_string(),
    };
    Ok(body.into())
}

#[derive(RwebResponse)]
#[response(description = "Trakt Watchlist Action", content = "html")]
struct TraktWatchlistActionResponse(HtmlBase<String, Error>);

#[get("/trakt/watchlist/{action}/{imdb_url}")]
pub async fn trakt_watchlist_action(
    action: TraktActionsWrapper,
    imdb_url: StackString,
    #[cookie = "jwt"] _: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<TraktWatchlistActionResponse> {
    let req = WatchlistActionRequest {
        action: action.into(),
        imdb_url,
    };
    let imdb_url = req.handle(&state.db, &state.trakt).await?;
    let body: String = watchlist_action_worker(&state.trakt, action.into(), &imdb_url)
        .await?
        .into();
    Ok(HtmlBase::new(body).into())
}

fn trakt_watched_seasons_worker(link: &str, imdb_url: &str, entries: &[ImdbSeason]) -> StackString {
    let entries = entries
        .iter()
        .map(|s| {
            let id = format!("watched_seasons_id_{}_{}_{}", &s.show, &link, &s.season);
            let button_add = format!(
                r#"
            <td>
            <button type="submit" id="{id}"
                onclick="imdb_update('{show}', '{link}', {season}, '/trakt/watched/list/{link}');"
                >update database</button></td>"#,
                id = id,
                show = s.show,
                link = link,
                season = s.season,
            );

            format!(
                "<tr><td>{}<td>{}<td>{}<td>{}</tr>",
                format!(
                    r#"<a href="javascript:updateMainArticle('/trakt/watched/list/{}/{}')">{}</t>"#,
                    imdb_url, s.season, s.title
                ),
                s.season,
                s.nepisodes,
                button_add
            )
        })
        .join("");

    let previous = r#"<a href="javascript:updateMainArticle('/trakt/watchlist')">Go Back</a><br>"#;
    format!(r#"{}<table border="0">{}</table>"#, previous, entries).into()
}

#[derive(RwebResponse)]
#[response(description = "Trakt Watchlist Show List", content = "html")]
struct TraktWatchlistShowListResponse(HtmlBase<String, Error>);

#[get("/trakt/watched/list/{imdb_url}")]
pub async fn trakt_watched_seasons(
    imdb_url: StackString,
    #[cookie = "jwt"] _: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<TraktWatchlistShowListResponse> {
    let show_opt = ImdbRatings::get_show_by_link(&imdb_url, &state.db)
        .await
        .map(|s| s.map(|sh| (imdb_url, sh)))
        .map_err(Into::<Error>::into)?;

    let empty = || ("".into(), "".into(), "".into());
    let (imdb_url, show, link) =
        show_opt.map_or_else(empty, |(imdb_url, t)| (imdb_url, t.show, t.link));
    let req = ImdbSeasonsRequest { show };
    let entries = req.handle(&state.db, &state.config).await?;
    let body: String = trakt_watched_seasons_worker(&link, &imdb_url, &entries).into();
    Ok(HtmlBase::new(body).into())
}

#[derive(RwebResponse)]
#[response(description = "Trakt Watchlist Show Season", content = "html")]
struct TraktWatchlistShowSeasonResponse(HtmlBase<String, Error>);

#[get("/trakt/watched/list/{imdb_url}/{season}")]
pub async fn trakt_watched_list(
    imdb_url: StackString,
    season: i32,
    #[cookie = "jwt"] _: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<TraktWatchlistShowSeasonResponse> {
    let mock_stdout = MockStdout::new();
    let stdout = StdoutChannel::with_mock_stdout(mock_stdout.clone(), mock_stdout.clone());

    let body: String = watch_list_http_worker(&state.config, &state.db, &stdout, &imdb_url, season)
        .await?
        .into();
    Ok(HtmlBase::new(body).into())
}

#[derive(RwebResponse)]
#[response(description = "Trakt Watchlist Episode Action", content = "html")]
struct TraktWatchlistEpisodeActionResponse(HtmlBase<String, Error>);

#[get("/trakt/watched/{action}/{imdb_url}/{season}/{episode}")]
pub async fn trakt_watched_action(
    action: TraktActionsWrapper,
    imdb_url: StackString,
    season: i32,
    episode: i32,
    #[cookie = "jwt"] _: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<TraktWatchlistEpisodeActionResponse> {
    let mock_stdout = MockStdout::new();
    let stdout = StdoutChannel::with_mock_stdout(mock_stdout.clone(), mock_stdout.clone());

    let body: String = watched_action_http_worker(
        &state.trakt,
        &state.db,
        action.into(),
        &imdb_url,
        season,
        episode,
        &state.config,
        &stdout,
    )
    .await?
    .into();
    Ok(HtmlBase::new(body).into())
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

#[derive(RwebResponse)]
#[response(description = "Trakt Calendar", content = "html")]
struct TraktCalendarResponse(HtmlBase<String, Error>);

#[get("/trakt/cal")]
pub async fn trakt_cal(
    #[cookie = "jwt"] _: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<TraktCalendarResponse> {
    let entries = trakt_cal_http_worker(&state.trakt, &state.db).await?;
    let body: String = trakt_cal_worker(&entries).into();
    Ok(HtmlBase::new(body).into())
}

#[derive(RwebResponse)]
#[response(description = "Trakt Auth Url", content = "html")]
struct TraktAuthUrlResponse(HtmlBase<String, Error>);

#[get("/trakt/auth_url")]
pub async fn trakt_auth_url(
    #[cookie = "jwt"] _: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<TraktAuthUrlResponse> {
    state.trakt.init().await;
    let url: String = state
        .trakt
        .get_auth_url()
        .await
        .map(Into::into)
        .map_err(Into::<Error>::into)?;
    Ok(HtmlBase::new(url).into())
}

#[derive(Serialize, Deserialize, Schema)]
pub struct TraktCallbackRequest {
    pub code: StackString,
    pub state: StackString,
}

#[derive(RwebResponse)]
#[response(description = "Trakt Callback", content = "html")]
struct TraktCallbackResponse(HtmlBase<&'static str, Error>);

#[get("/trakt/callback")]
pub async fn trakt_callback(
    query: Query<TraktCallbackRequest>,
    #[cookie = "jwt"] _: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<TraktCallbackResponse> {
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
    Ok(HtmlBase::new(body).into())
}

#[derive(RwebResponse)]
#[response(description = "Trakt Refresh Auth", content = "html")]
struct TraktRefreshAuthResponse(HtmlBase<&'static str, Error>);

#[get("/trakt/refresh_auth")]
pub async fn refresh_auth(
    #[cookie = "jwt"] _: LoggedUser,
    #[data] state: AppState,
) -> WarpResult<TraktRefreshAuthResponse> {
    state.trakt.init().await;
    state
        .trakt
        .exchange_refresh_token()
        .await
        .map_err(Into::<Error>::into)?;
    Ok(HtmlBase::new("Finished").into())
}

async fn trakt_cal_http_worker(
    trakt: &TraktConnection,
    pool: &PgPool,
) -> Result<Vec<StackString>, Error> {
    let button_add = Arc::new(format!(
        "{}{}",
        r#"<td><button type="submit" id="ID" "#,
        r#"onclick="imdb_update('SHOW', 'LINK', SEASON, '/trakt/cal');"
            >update database</button></td>"#
    ));
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
        let line = format!(
            "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td>{}</tr>",
            format!(
                r#"<a href="javascript:updateMainArticle('/trakt/watched/list/{}/{}')">{}</a>"#,
                cal.link, cal.season, cal.show,
            ),
            format!(
                r#"<a href="https://www.imdb.com/title/{}" target="_blank">imdb</a>"#,
                cal.link
            ),
            if let Some(link) = cal.ep_link {
                format!(
                    r#"<a href="https://www.imdb.com/title/{}" target="_blank">{} {}</a>"#,
                    link, cal.season, cal.episode,
                )
            } else if let Some(link) = exists.as_ref() {
                format!(
                    r#"<a href="https://www.imdb.com/title/{}" target="_blank">{} {}</a>"#,
                    link, cal.season, cal.episode,
                )
            } else {
                format!("{} {}", cal.season, cal.episode,)
            },
            cal.airdate,
            if exists.is_some() {
                "".to_string()
            } else {
                button_add
                    .replace("SHOW", &show)
                    .replace("LINK", &cal.link)
                    .replace("SEASON", &cal.season.to_string())
            },
        )
        .into();
        lines.push(line);
    }
    Ok(lines)
}

pub async fn watch_list_http_worker(
    config: &Config,
    pool: &PgPool,
    stdout: &StdoutChannel<StackString>,
    imdb_url: &str,
    season: i32,
) -> HttpResult<StackString> {
    let button_add = format!(
        "{}{}",
        r#"<button type="submit" id="ID" "#,
        r#"onclick="watched_add('SHOW', SEASON, EPISODE);">add to watched</button>"#
    );
    let button_rm = format!(
        "{}{}",
        r#"<button type="submit" id="ID" "#,
        r#"onclick="watched_rm('SHOW', SEASON, EPISODE);">remove from watched</button>"#
    );

    let mc = MovieCollection::new(config, pool, stdout);
    let mq = MovieQueueDB::new(config, pool, stdout);

    let show = ImdbRatings::get_show_by_link(imdb_url, pool)
        .await?
        .ok_or_else(|| format_err!("Show Doesn't exist"))?;

    let watched_episodes_db: HashSet<i32> = get_watched_shows_db(pool, &show.show, Some(season))
        .await?
        .into_iter()
        .map(|s| s.episode)
        .collect();

    let queue: HashMap<(StackString, i32, i32), _> = mq
        .print_movie_queue(&[show.show.as_str()])
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

    let entries: Vec<_> = mc.print_imdb_episodes(&show.show, Some(season)).await?;

    let mut collection_idx_map = HashMap::new();
    for r in &entries {
        if let Some(row) = queue.get(&(show.show.clone(), season, r.episode)) {
            if let Some(index) = mc.get_collection_index(&row.path).await? {
                collection_idx_map.insert(r.episode, index);
            }
        }
    }

    let entries = entries
        .iter()
        .map(|s| {
            let entry = if let Some(collection_idx) = collection_idx_map.get(&s.episode) {
                format!(
                    r#"<a href="javascript:updateMainArticle('{}');">{}</a>"#,
                    &format!("{}/{}", "/list/play", collection_idx),
                    s.eptitle
                )
            } else {
                s.eptitle.to_string()
            };

            format!(
                "<tr><td>{}</td><td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
                show.show,
                entry,
                format!(
                    r#"<a href="https://www.imdb.com/title/{}" target="_blank">s{} ep{}</a>"#,
                    s.epurl, season, s.episode,
                ),
                format!(
                    "rating: {:0.1} / {:0.1}",
                    s.rating,
                    show.rating.as_ref().unwrap_or(&-1.0)
                ),
                s.airdate,
                if watched_episodes_db.contains(&s.episode) {
                    button_rm
                        .replace("SHOW", &show.link)
                        .replace("SEASON", &season.to_string())
                        .replace("EPISODE", &s.episode.to_string())
                } else {
                    button_add
                        .replace("SHOW", &show.link)
                        .replace("SEASON", &season.to_string())
                        .replace("EPISODE", &s.episode.to_string())
                }
            )
        })
        .join("\n");

    let previous = format!(
        r#"<a href="javascript:updateMainArticle('/trakt/watched/list/{}')">Go Back</a><br>"#,
        imdb_url
    );
    let buttons = format!(
        r#"
        <button name="remcomout" id="remcomoutput"> &nbsp; </button>
        <button type="submit" id="ID"
            onclick="imdb_update('{show}', '{link}', {season},
            '/trakt/watched/list/{link}/{season}');"
            >update database</button><br>
    "#,
        show = show.show,
        link = show.link,
        season = season
    );

    let entries = format!(
        r#"{}{}<table border="0">{}</table>"#,
        previous, buttons, entries
    )
    .into();
    Ok(entries)
}

#[allow(clippy::too_many_arguments)]
pub async fn watched_action_http_worker(
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
    let imdb_url = Arc::new(imdb_url.to_owned());
    trakt.init().await;
    let body = match action {
        TraktActions::Add => {
            let result = if season != -1 && episode != -1 {
                trakt
                    .add_episode_to_watched(&imdb_url, season, episode)
                    .await?
            } else {
                trakt.add_movie_to_watched(&imdb_url).await?
            };
            if season != -1 && episode != -1 {
                WatchedEpisode {
                    imdb_url: imdb_url.to_string().into(),
                    season,
                    episode,
                    ..WatchedEpisode::default()
                }
                .insert_episode(&mc.pool)
                .await?;
            } else {
                WatchedMovie {
                    imdb_url: imdb_url.to_string().into(),
                    title: "".into(),
                }
                .insert_movie(&mc.pool)
                .await?;
            }

            format!("{}", result)
        }
        TraktActions::Remove => {
            let imdb_url_ = Arc::clone(&imdb_url);
            let result = if season != -1 && episode != -1 {
                trakt
                    .remove_episode_to_watched(&imdb_url_, season, episode)
                    .await?
            } else {
                trakt.remove_movie_to_watched(&imdb_url_).await?
            };

            if season != -1 && episode != -1 {
                if let Some(epi_) =
                    WatchedEpisode::get_watched_episode(&mc.pool, &imdb_url, season, episode)
                        .await?
                {
                    epi_.delete_episode(&mc.pool).await?;
                }
            } else if let Some(movie) = WatchedMovie::get_watched_movie(&mc.pool, &imdb_url).await?
            {
                movie.delete_movie(&mc.pool).await?;
            };

            format!("{}", result)
        }
        _ => "".to_string(),
    }
    .into();
    Ok(body)
}

#[derive(RwebResponse)]
#[response(description = "Plex Events")]
struct PlexEventResponse(JsonBase<Vec<PlexEventWrapper>, Error>);

#[derive(Serialize, Deserialize, Debug, Schema)]
pub struct PlexEventRequest {
    pub start_timestamp: Option<DateTimeWrapper>,
    pub event_type: Option<PlexEventTypeWrapper>,
    pub offset: Option<u64>,
    pub limit: Option<u64>,
}

#[get("/list/plex_event")]
pub async fn plex_events(
    query: Query<PlexEventRequest>,
    #[data] state: AppState,
    #[cookie = "jwt"] _: LoggedUser,
) -> WarpResult<PlexEventResponse> {
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
    #[cookie = "jwt"] _: LoggedUser,
) -> WarpResult<PlexEventUpdateResponse> {
    let payload = payload.into_inner();
    for event in payload.events {
        let event: PlexEvent = event.into();
        event
            .write_event(&state.db)
            .await
            .map_err(Into::<Error>::into)?;
    }

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
    if state.config.plex_webhook_key == webhook_key.into() {
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
        error!("failed deserialize {}", buf);
        Err(format_err!("failed deserialize {}", buf))
    }
}

#[derive(RwebResponse)]
#[response(description = "Plex Event List", content = "html")]
struct PlexEventList(HtmlBase<String, Error>);

#[get("/list/plex")]
pub async fn plex_list(
    #[cookie = "jwt"] _: LoggedUser,
    #[data] state: AppState,
    query: Query<PlexEventRequest>,
) -> WarpResult<PlexEventList> {
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
    let body = format!(
        r#"
            <table border="1" class="dataframe">
            <thead><tr>
            <th>Time</th>
            <th>Event Type</th>
            <th>Item Type</th>
            <th>Section</th>
            <th>Title</th></tr></thead>
            <tbody>{}</tbody>
            </table>
        "#,
        body,
    );

    Ok(HtmlBase::new(body).into())
}

#[derive(RwebResponse)]
#[response(description = "Plex Filenames")]
struct PlexFilenameResponse(JsonBase<Vec<PlexFilenameWrapper>, Error>);

#[derive(Serialize, Deserialize, Debug, Schema)]
pub struct PlexFilenameRequest {
    pub start_timestamp: Option<DateTimeWrapper>,
    pub offset: Option<u64>,
    pub limit: Option<u64>,
}

#[get("/list/plex_filename")]
pub async fn plex_filename(
    query: Query<PlexFilenameRequest>,
    #[data] state: AppState,
    #[cookie = "jwt"] _: LoggedUser,
) -> WarpResult<PlexFilenameResponse> {
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
    #[cookie = "jwt"] _: LoggedUser,
) -> WarpResult<PlexFilenameUpdateResponse> {
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
    Ok(HtmlBase::new("Success").into())
}
