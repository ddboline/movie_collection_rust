#![allow(clippy::needless_pass_by_value)]

use actix_web::{
    web::{Data, Json, Path, Query},
    HttpResponse,
};
use anyhow::format_err;
use maplit::hashmap;
use serde::Serialize;
use stack_string::StackString;
use std::{
    borrow::Borrow,
    collections::{HashMap, HashSet},
    hash::{Hash, Hasher},
    path,
    path::PathBuf,
    time::Duration,
};
use stdout_channel::{MockStdout, StdoutChannel};
use tokio::{fs::remove_file, task::spawn_blocking, time::timeout};

use movie_collection_lib::{
    make_list::FileLists,
    make_queue::movie_queue_http,
    movie_collection::{MovieCollection, TvShowsResult},
    movie_queue::{MovieQueueDB, MovieQueueResult},
    pgpool::PgPool,
    trakt_utils::{get_watchlist_shows_db_map, WatchListShow},
    transcode_service::{transcode_status, TranscodeService, TranscodeServiceRequest},
    tv_show_source::TvShowSource,
    utils::HBR,
};

use super::{
    errors::ServiceError as Error,
    logged_user::LoggedUser,
    movie_queue_app::{AppState, CONFIG},
    movie_queue_requests::{
        FindNewEpisodeRequest, ImdbEpisodesSyncRequest, ImdbEpisodesUpdateRequest,
        ImdbRatingsSetSourceRequest, ImdbRatingsSyncRequest, ImdbRatingsUpdateRequest,
        ImdbShowRequest, LastModifiedRequest, MovieCollectionSyncRequest,
        MovieCollectionUpdateRequest, MoviePathRequest, MovieQueueRequest, MovieQueueSyncRequest,
        MovieQueueUpdateRequest, ParseImdbRequest,
    },
    HandleRequest,
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
    patterns: Vec<StackString>,
    queue: Vec<MovieQueueResult>,
    pool: &PgPool,
) -> Result<StackString, Error> {
    let mock_stdout = MockStdout::new();
    let stdout = StdoutChannel::with_mock_stdout(mock_stdout.clone(), mock_stdout.clone());

    let entries = movie_queue_http(&queue, pool, &CONFIG, &stdout).await?;
    let body = movie_queue_body(&patterns, &entries);
    Ok(body)
}

pub async fn movie_queue(_: LoggedUser, state: Data<AppState>) -> HttpResult {
    let req = MovieQueueRequest {
        patterns: Vec::new(),
    };
    let (queue, _) = state.db.handle(req).await?;
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
    let (queue, patterns) = state.db.handle(req).await?;
    let body = queue_body_resp(patterns, queue, &state.db).await?;
    form_http_response(body.into())
}

pub async fn movie_queue_delete(
    path: Path<StackString>,
    _: LoggedUser,
    state: Data<AppState>,
) -> HttpResult {
    let path = path.into_inner();

    let mock_stdout = MockStdout::new();
    let stdout = StdoutChannel::with_mock_stdout(mock_stdout.clone(), mock_stdout.clone());

    if path::Path::new(path.as_str()).exists() {
        MovieQueueDB::new(&CONFIG, &state.db, &stdout)
            .remove_from_queue_by_path(&path)
            .await?;
    }
    form_http_response(path.into())
}

async fn transcode_worker(
    directory: Option<&path::Path>,
    entries: &[MovieQueueResult],
    pool: &PgPool,
) -> Result<StackString, Error> {
    let mock_stdout = MockStdout::new();
    let stdout = StdoutChannel::with_mock_stdout(mock_stdout.clone(), mock_stdout.clone());

    let remcom_service = TranscodeService::new(&CONFIG, &CONFIG.remcom_queue, pool, &stdout);
    let mut output = Vec::new();
    for entry in entries {
        let payload = TranscodeServiceRequest::create_remcom_request(
            &CONFIG,
            &path::Path::new(entry.path.as_str()),
            directory,
            false,
        )
        .await?;
        remcom_service
            .publish_transcode_job(&payload, |_| async move { Ok(()) })
            .await?;
        output.push(format!("{:?}", payload));
        output.push(payload.publish_to_cli(&CONFIG).await?.into());
    }
    Ok(output.join("").into())
}

pub async fn movie_queue_transcode(
    path: Path<StackString>,
    _: LoggedUser,
    state: Data<AppState>,
) -> HttpResult {
    let path = path.into_inner();
    let patterns = vec![path];

    let req = MovieQueueRequest { patterns };
    let (entries, _) = state.db.handle(req).await?;
    let body = transcode_worker(None, &entries, &state.db).await?;
    form_http_response(body.into())
}

pub async fn movie_queue_transcode_directory(
    path: Path<(StackString, StackString)>,
    _: LoggedUser,
    state: Data<AppState>,
) -> HttpResult {
    let (directory, file) = path.into_inner();
    let patterns = vec![file];

    let req = MovieQueueRequest { patterns };
    let (entries, _) = state.db.handle(req).await?;
    let body = transcode_worker(
        Some(&path::Path::new(directory.as_str())),
        &entries,
        &state.db,
    )
    .await?;
    form_http_response(body.into())
}

fn play_worker(full_path: &path::Path) -> HttpResult {
    let file_name = full_path
        .file_name()
        .ok_or_else(|| format_err!("Invalid path"))?
        .to_string_lossy();
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

    let partial_path = path::Path::new("/var/www/html/videos/partial").join(file_name.as_ref());
    if partial_path.exists() {
        std::fs::remove_file(&partial_path)?;
    }

    #[cfg(target_family = "unix")]
    std::os::unix::fs::symlink(&full_path, &partial_path)?;

    form_http_response(body)
}

pub async fn movie_queue_play(idx: Path<i32>, _: LoggedUser, state: Data<AppState>) -> HttpResult {
    let idx = idx.into_inner();

    let req = MoviePathRequest { idx };
    let movie_path = state.db.handle(req).await?;
    let movie_path = path::Path::new(movie_path.as_str());
    play_worker(&movie_path)
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
    let x = state.db.handle(req).await?;
    form_http_response(x.into())
}

fn new_episode_worker(entries: &[StackString]) -> HttpResult {
    let previous = r#"
        <a href="javascript:updateMainArticle('/list/tvshows')">Go Back</a><br>
        <input type="button" name="list_cal" value="TVCalendar" onclick="updateMainArticle('/list/cal');"/>
        <input type="button" name="list_cal" value="NetflixCalendar" onclick="updateMainArticle('/list/cal?source=netflix');"/>
        <input type="button" name="list_cal" value="AmazonCalendar" onclick="updateMainArticle('/list/cal?source=amazon');"/>
        <input type="button" name="list_cal" value="HuluCalendar" onclick="updateMainArticle('/list/cal?source=hulu');"/><br>
        <button name="remcomout" id="remcomoutput"> &nbsp; </button>
    "#;
    let entries = format!(
        r#"{}<table border="0">{}</table>"#,
        previous,
        entries.join("")
    );
    form_http_response(entries)
}

pub async fn find_new_episodes(
    query: Query<FindNewEpisodeRequest>,
    _: LoggedUser,
    state: Data<AppState>,
) -> HttpResult {
    let req = query.into_inner();
    let entries = state.db.handle(req).await?;
    new_episode_worker(&entries)
}

pub async fn imdb_episodes_route(
    query: Query<ImdbEpisodesSyncRequest>,
    _: LoggedUser,
    state: Data<AppState>,
) -> HttpResult {
    let req = query.into_inner();
    let x = state.db.handle(req).await?;
    to_json(x)
}

pub async fn imdb_episodes_update(
    data: Json<ImdbEpisodesUpdateRequest>,
    _: LoggedUser,
    state: Data<AppState>,
) -> HttpResult {
    let episodes = data.into_inner();

    let req = episodes;
    state.db.handle(req).await?;
    form_http_response("Success".to_string())
}

pub async fn imdb_ratings_route(
    query: Query<ImdbRatingsSyncRequest>,
    _: LoggedUser,
    state: Data<AppState>,
) -> HttpResult {
    let req = query.into_inner();
    let x = state.db.handle(req).await?;
    to_json(x)
}

pub async fn imdb_ratings_update(
    data: Json<ImdbRatingsUpdateRequest>,
    _: LoggedUser,
    state: Data<AppState>,
) -> HttpResult {
    let shows = data.into_inner();

    let req = shows;
    state.db.handle(req).await?;
    form_http_response("Success".to_string())
}

pub async fn imdb_ratings_set_source(
    query: Query<ImdbRatingsSetSourceRequest>,
    _: LoggedUser,
    state: Data<AppState>,
) -> HttpResult {
    let query = query.into_inner();
    state.db.handle(query).await?;
    form_http_response("Success".to_string())
}

pub async fn movie_queue_route(
    query: Query<MovieQueueSyncRequest>,
    _: LoggedUser,
    state: Data<AppState>,
) -> HttpResult {
    let req = query.into_inner();
    let x = state.db.handle(req).await?;
    to_json(x)
}

pub async fn movie_queue_update(
    data: Json<MovieQueueUpdateRequest>,
    _: LoggedUser,
    state: Data<AppState>,
) -> HttpResult {
    let queue = data.into_inner();

    let req = queue;
    state.db.handle(req).await?;
    form_http_response("Success".to_string())
}

pub async fn movie_collection_route(
    query: Query<MovieCollectionSyncRequest>,
    _: LoggedUser,
    state: Data<AppState>,
) -> HttpResult {
    let req = query.into_inner();
    let x = state.db.handle(req).await?;
    to_json(x)
}

pub async fn movie_collection_update(
    data: Json<MovieCollectionUpdateRequest>,
    _: LoggedUser,
    state: Data<AppState>,
) -> HttpResult {
    let collection = data.into_inner();

    let req = collection;
    state.db.handle(req).await?;
    form_http_response("Success".to_string())
}

pub async fn last_modified_route(_: LoggedUser, state: Data<AppState>) -> HttpResult {
    let req = LastModifiedRequest {};
    let x = state.db.handle(req).await?;
    to_json(x)
}

pub async fn frontpage(_: LoggedUser, _: Data<AppState>) -> HttpResult {
    form_http_response(HBR.render("index.html", &hashmap! {"BODY" => ""})?)
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
        self.link.hash(state)
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

fn tvshows_worker(res1: TvShowsMap, tvshows: Vec<TvShowsResult>) -> Result<StackString, Error> {
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

    let shows = process_shows(tvshows, watchlist)?;

    let previous = r#"
        <a href="javascript:updateMainArticle('/list/watchlist')">Go Back</a><br>
        <a href="javascript:updateMainArticle('/trakt/watchlist')">Watch List</a>
        <button name="remcomout" id="remcomoutput"> &nbsp; </button><br>
    "#;

    let entries = format!(
        r#"{}<table border="0">{}</table>"#,
        previous,
        shows.join("")
    )
    .into();

    Ok(entries)
}

pub async fn tvshows(_: LoggedUser, state: Data<AppState>) -> HttpResult {
    let mock_stdout = MockStdout::new();
    let stdout = StdoutChannel::with_mock_stdout(mock_stdout.clone(), mock_stdout.clone());

    let mc = MovieCollection::new(&CONFIG, &state.db, &stdout);
    let shows = mc.print_tv_shows().await?;
    let show_map = get_watchlist_shows_db_map(&state.db).await?;
    let entries = tvshows_worker(show_map, shows)?;
    form_http_response(entries.into())
}

fn process_shows(
    tvshows: HashSet<ProcessShowItem>,
    watchlist: HashSet<ProcessShowItem>,
) -> Result<Vec<StackString>, Error> {
    let watchlist_shows: Vec<_> = watchlist
        .iter()
        .filter(|item| tvshows.get(item.link.as_str()).is_none())
        .collect();

    let mut shows: Vec<_> = tvshows.iter().chain(watchlist_shows.into_iter()).collect();
    shows.sort_by(|x, y| x.show.cmp(&y.show));

    let button_add = r#"<td><button type="submit" id="ID" onclick="watchlist_add('SHOW');">add to watchlist</button></td>"#;
    let button_rm = r#"<td><button type="submit" id="ID" onclick="watchlist_rm('SHOW');">remove from watchlist</button></td>"#;

    let shows: Vec<_> = shows
        .into_iter()
        .map(|item| {
            let has_watchlist = watchlist.contains(item.link.as_str());
            format!(
                r#"<tr><td>{}</td>
                <td><a href="https://www.imdb.com/title/{}" target="_blank">imdb</a></td><td>{}</td><td>{}</td><td>{}</td></tr>"#,
                if tvshows.contains(item.link.as_str()) {
                    format!(r#"<a href="javascript:updateMainArticle('/list/{}')">{}</a>"#, item.show, item.title)
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
        .collect();
    Ok(shows)
}

pub async fn user(user: LoggedUser) -> HttpResult {
    to_json(user)
}

pub async fn movie_queue_transcode_status(_: LoggedUser, _: Data<AppState>) -> HttpResult {
    let task = timeout(
        Duration::from_secs(10),
        spawn_blocking(move || FileLists::get_file_lists(&CONFIG)),
    );
    let status = transcode_status(&CONFIG).await?;
    let file_lists = task
        .await
        .map_or_else(|_| Ok(FileLists::default()), |task| task.unwrap())?;
    form_http_response(status.get_html(&file_lists, &CONFIG).join(""))
}

pub async fn movie_queue_transcode_file(
    path: Path<StackString>,
    _: LoggedUser,
    state: Data<AppState>,
) -> HttpResult {
    let mock_stdout = MockStdout::new();
    let stdout = StdoutChannel::with_mock_stdout(mock_stdout.clone(), mock_stdout.clone());

    let filename = path.into_inner();
    let transcode_service =
        TranscodeService::new(&CONFIG, &CONFIG.transcode_queue, &state.db, &stdout);
    let input_path = CONFIG
        .home_dir
        .join("Documents")
        .join("movies")
        .join(&filename);
    let req = TranscodeServiceRequest::create_transcode_request(&CONFIG, &input_path)?;
    transcode_service
        .publish_transcode_job(&req, |_| async move { Ok(()) })
        .await?;
    let body = req.publish_to_cli(&CONFIG).await?.into();
    form_http_response(body)
}

pub async fn movie_queue_remcom_file(
    path: Path<StackString>,
    _: LoggedUser,
    state: Data<AppState>,
) -> HttpResult {
    let mock_stdout = MockStdout::new();
    let stdout = StdoutChannel::with_mock_stdout(mock_stdout.clone(), mock_stdout.clone());

    let filename = path.into_inner();
    let transcode_service =
        TranscodeService::new(&CONFIG, &CONFIG.remcom_queue, &state.db, &stdout);
    let input_path = CONFIG
        .home_dir
        .join("Documents")
        .join("movies")
        .join(&filename);
    let directory: Option<PathBuf> = None;
    let req =
        TranscodeServiceRequest::create_remcom_request(&CONFIG, &input_path, directory, false)
            .await?;
    transcode_service
        .publish_transcode_job(&req, |_| async move { Ok(()) })
        .await?;
    let body = req.publish_to_cli(&CONFIG).await?.into();
    form_http_response(body)
}

pub async fn movie_queue_remcom_directory_file(
    path: Path<(StackString, StackString)>,
    _: LoggedUser,
    state: Data<AppState>,
) -> HttpResult {
    let mock_stdout = MockStdout::new();
    let stdout = StdoutChannel::with_mock_stdout(mock_stdout.clone(), mock_stdout.clone());

    let (directory, filename) = path.into_inner();
    let transcode_service =
        TranscodeService::new(&CONFIG, &CONFIG.remcom_queue, &state.db, &stdout);
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
    transcode_service
        .publish_transcode_job(&req, |_| async move { Ok(()) })
        .await?;
    let body = req.publish_to_cli(&CONFIG).await?.into();
    form_http_response(body)
}

pub async fn movie_queue_transcode_cleanup(
    path: Path<StackString>,
    _: LoggedUser,
    _: Data<AppState>,
) -> HttpResult {
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
