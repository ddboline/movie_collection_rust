#![allow(clippy::needless_pass_by_value)]

use anyhow::format_err;
use itertools::Itertools;
use maplit::hashmap;
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
use warp::{Rejection, Reply};

use movie_collection_lib::{
    config::Config,
    imdb_episodes::ImdbEpisodes,
    imdb_ratings::ImdbRatings,
    make_list::FileLists,
    make_queue::movie_queue_http,
    movie_collection::{ImdbSeason, MovieCollection, TvShowsResult},
    movie_queue::{MovieQueueDB, MovieQueueResult},
    pgpool::PgPool,
    trakt_connection::TraktConnection,
    trakt_utils::{
        get_watched_shows_db, get_watchlist_shows_db_map, TraktActions, WatchListShow,
        WatchedEpisode, WatchedMovie,
    },
    transcode_service::{transcode_status, TranscodeService, TranscodeServiceRequest},
    tv_show_source::TvShowSource,
    utils::HBR,
};

use super::{
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
    patterns: Vec<StackString>,
    queue: Vec<MovieQueueResult>,
    pool: &PgPool,
) -> HttpResult<StackString> {
    let mock_stdout = MockStdout::new();
    let stdout = StdoutChannel::with_mock_stdout(mock_stdout.clone(), mock_stdout.clone());

    let entries = movie_queue_http(&queue, pool, &config, &stdout).await?;
    let body = movie_queue_body(&patterns, &entries);
    Ok(body)
}

pub async fn movie_queue(_: LoggedUser, state: AppState) -> WarpResult<impl Reply> {
    let req = MovieQueueRequest {
        patterns: Vec::new(),
    };
    let (queue, _) = req.handle(&state.db, &state.config).await?;
    let body: String = queue_body_resp(&state.config, Vec::new(), queue, &state.db)
        .await?
        .into();
    Ok(warp::reply::html(body))
}

pub async fn movie_queue_show(
    path: StackString,
    _: LoggedUser,
    state: AppState,
) -> WarpResult<impl Reply> {
    let patterns = vec![path];

    let req = MovieQueueRequest { patterns };
    let (queue, patterns) = req.handle(&state.db, &state.config).await?;
    let body: String = queue_body_resp(&state.config, patterns, queue, &state.db)
        .await?
        .into();
    Ok(warp::reply::html(body))
}

pub async fn movie_queue_delete(
    path: StackString,
    _: LoggedUser,
    state: AppState,
) -> WarpResult<impl Reply> {
    let mock_stdout = MockStdout::new();
    let stdout = StdoutChannel::with_mock_stdout(mock_stdout.clone(), mock_stdout.clone());

    if path::Path::new(path.as_str()).exists() {
        MovieQueueDB::new(&state.config, &state.db, &stdout)
            .remove_from_queue_by_path(&path)
            .await
            .map_err(Into::<Error>::into)?;
    }
    let body: String = path.into();
    Ok(warp::reply::html(body))
}

async fn transcode_worker(
    config: &Config,
    directory: Option<&path::Path>,
    entries: &[MovieQueueResult],
    pool: &PgPool,
) -> HttpResult<StackString> {
    let mock_stdout = MockStdout::new();
    let stdout = StdoutChannel::with_mock_stdout(mock_stdout.clone(), mock_stdout.clone());

    let remcom_service = TranscodeService::new(&config, &config.remcom_queue, pool, &stdout);
    let mut output = Vec::new();
    for entry in entries {
        let payload = TranscodeServiceRequest::create_remcom_request(
            &config,
            &path::Path::new(entry.path.as_str()),
            directory,
            false,
        )
        .await?;
        remcom_service
            .publish_transcode_job(&payload, |_| async move { Ok(()) })
            .await?;
        output.push(format!("{:?}", payload));
        output.push(payload.publish_to_cli(&config).await?.into());
    }
    Ok(output.join("").into())
}

pub async fn movie_queue_transcode(
    path: StackString,
    _: LoggedUser,
    state: AppState,
) -> WarpResult<impl Reply> {
    let patterns = vec![path];

    let req = MovieQueueRequest { patterns };
    let (entries, _) = req.handle(&state.db, &state.config).await?;
    let body: String = transcode_worker(&state.config, None, &entries, &state.db)
        .await?
        .into();
    Ok(warp::reply::html(body))
}

pub async fn movie_queue_transcode_directory(
    directory: StackString,
    file: StackString,
    _: LoggedUser,
    state: AppState,
) -> WarpResult<impl Reply> {
    let patterns = vec![file];

    let req = MovieQueueRequest { patterns };
    let (entries, _) = req.handle(&state.db, &state.config).await?;
    let body: String = transcode_worker(
        &state.config,
        Some(&path::Path::new(directory.as_str())),
        &entries,
        &state.db,
    )
    .await?
    .into();
    Ok(warp::reply::html(body))
}

fn play_worker(full_path: &path::Path) -> HttpResult<String> {
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
    std::os::unix::fs::symlink(&full_path, &partial_path).map_err(Into::<Error>::into)?;

    Ok(body)
}

pub async fn movie_queue_play(idx: i32, _: LoggedUser, state: AppState) -> WarpResult<impl Reply> {
    let req = MoviePathRequest { idx };
    let movie_path = req.handle(&state.db, &state.config).await?;
    let movie_path = path::Path::new(movie_path.as_str());
    let body = play_worker(&movie_path)?;
    Ok(warp::reply::html(body))
}

pub async fn imdb_show(
    show: StackString,
    query: ParseImdbRequest,
    _: LoggedUser,
    state: AppState,
) -> WarpResult<impl Reply> {
    let req = ImdbShowRequest { show, query };
    let body: String = req.handle(&state.db, &state.config).await?.into();
    Ok(warp::reply::html(body))
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

pub async fn find_new_episodes(
    query: FindNewEpisodeRequest,
    _: LoggedUser,
    state: AppState,
) -> WarpResult<impl Reply> {
    let entries = query.handle(&state.db, &state.config).await?;
    let body = new_episode_worker(&entries);
    Ok(warp::reply::html(body))
}

pub async fn imdb_episodes_route(
    query: ImdbEpisodesSyncRequest,
    _: LoggedUser,
    state: AppState,
) -> WarpResult<impl Reply> {
    let x = query.handle(&state.db).await?;
    Ok(warp::reply::json(&x))
}

pub async fn imdb_episodes_update(
    episodes: ImdbEpisodesUpdateRequest,
    _: LoggedUser,
    state: AppState,
) -> WarpResult<impl Reply> {
    episodes.handle(&state.db).await?;
    Ok(warp::reply::html("Success"))
}

pub async fn imdb_ratings_route(
    query: ImdbRatingsSyncRequest,
    _: LoggedUser,
    state: AppState,
) -> WarpResult<impl Reply> {
    let x = query.handle(&state.db).await?;
    Ok(warp::reply::json(&x))
}

pub async fn imdb_ratings_update(
    shows: ImdbRatingsUpdateRequest,
    _: LoggedUser,
    state: AppState,
) -> WarpResult<impl Reply> {
    shows.handle(&state.db).await?;
    Ok(warp::reply::html("Success"))
}

pub async fn imdb_ratings_set_source(
    query: ImdbRatingsSetSourceRequest,
    _: LoggedUser,
    state: AppState,
) -> WarpResult<impl Reply> {
    query.handle(&state.db).await?;
    Ok(warp::reply::html("Success"))
}

pub async fn movie_queue_route(
    query: MovieQueueSyncRequest,
    _: LoggedUser,
    state: AppState,
) -> WarpResult<impl Reply> {
    let x = query.handle(&state.db, &state.config).await?;
    Ok(warp::reply::json(&x))
}

pub async fn movie_queue_update(
    queue: MovieQueueUpdateRequest,
    _: LoggedUser,
    state: AppState,
) -> WarpResult<impl Reply> {
    queue.handle(&state.db, &state.config).await?;
    Ok(warp::reply::html("Success"))
}

pub async fn movie_collection_route(
    query: MovieCollectionSyncRequest,
    _: LoggedUser,
    state: AppState,
) -> WarpResult<impl Reply> {
    let x = query.handle(&state.db, &state.config).await?;
    Ok(warp::reply::json(&x))
}

pub async fn movie_collection_update(
    collection: MovieCollectionUpdateRequest,
    _: LoggedUser,
    state: AppState,
) -> WarpResult<impl Reply> {
    collection.handle(&state.db, &state.config).await?;
    Ok(warp::reply::html("Success"))
}

pub async fn last_modified_route(_: LoggedUser, state: AppState) -> WarpResult<impl Reply> {
    let req = LastModifiedRequest {};
    let x = req.handle(&state.db).await?;
    Ok(warp::reply::json(&x))
}

pub async fn frontpage(_: LoggedUser, _: AppState) -> WarpResult<impl Reply> {
    let body = HBR
        .render("index.html", &hashmap! {"BODY" => ""})
        .map_err(Into::<Error>::into)?;
    Ok(warp::reply::html(body))
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

pub async fn tvshows(_: LoggedUser, state: AppState) -> WarpResult<impl Reply> {
    let mock_stdout = MockStdout::new();
    let stdout = StdoutChannel::with_mock_stdout(mock_stdout.clone(), mock_stdout.clone());

    let mc = MovieCollection::new(&state.config, &state.db, &stdout);
    let shows = mc.print_tv_shows().await.map_err(Into::<Error>::into)?;
    let show_map = get_watchlist_shows_db_map(&state.db)
        .await
        .map_err(Into::<Error>::into)?;
    let body: String = tvshows_worker(show_map, shows)?.into();
    Ok(warp::reply::html(body))
}

fn process_shows(
    tvshows: HashSet<ProcessShowItem>,
    watchlist: HashSet<ProcessShowItem>,
) -> Vec<StackString> {
    let watchlist_shows: Vec<_> = watchlist
        .iter()
        .filter(|item| tvshows.get(item.link.as_str()).is_none())
        .collect();

    let mut shows: Vec<_> = tvshows.iter().chain(watchlist_shows.into_iter()).collect();
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
        .collect()
}

pub async fn user(user: LoggedUser) -> WarpResult<impl Reply> {
    Ok(warp::reply::json(&user))
}

pub async fn movie_queue_transcode_status(
    _: LoggedUser,
    state: AppState,
) -> WarpResult<impl Reply> {
    let config = state.config.clone();
    let task = timeout(Duration::from_secs(10), FileLists::get_file_lists(&config));
    let status = transcode_status(&state.config)
        .await
        .map_err(Into::<Error>::into)?;
    let file_lists = task
        .await
        .map_or_else(|_| FileLists::default(), Result::unwrap);
    let body = status.get_html(&file_lists, &state.config).join("");
    Ok(warp::reply::html(body))
}

pub async fn movie_queue_transcode_file(
    filename: StackString,
    _: LoggedUser,
    state: AppState,
) -> WarpResult<impl Reply> {
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
    Ok(warp::reply::html(body))
}

pub async fn movie_queue_remcom_file(
    filename: StackString,
    _: LoggedUser,
    state: AppState,
) -> WarpResult<impl Reply> {
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
    Ok(warp::reply::html(body))
}

pub async fn movie_queue_remcom_directory_file(
    directory: StackString,
    filename: StackString,
    _: LoggedUser,
    state: AppState,
) -> WarpResult<impl Reply> {
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
    Ok(warp::reply::html(body))
}

pub async fn movie_queue_transcode_cleanup(
    path: StackString,
    _: LoggedUser,
    state: AppState,
) -> WarpResult<impl Reply> {
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
    Ok(warp::reply::html(body))
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

pub async fn trakt_watchlist(_: LoggedUser, state: AppState) -> WarpResult<impl Reply> {
    let shows = get_watchlist_shows_db_map(&state.db)
        .await
        .map_err(Into::<Error>::into)?;
    let body: String = watchlist_worker(shows).into();
    Ok(warp::reply::html(body))
}

async fn watchlist_action_worker(
    trakt: &TraktConnection,
    action: TraktActions,
    imdb_url: &str,
) -> HttpResult<StackString> {
    trakt.init().await;
    let body = match action {
        TraktActions::Add => trakt.add_watchlist_show(&imdb_url).await?.to_string(),
        TraktActions::Remove => trakt.remove_watchlist_show(&imdb_url).await?.to_string(),
        _ => "".to_string(),
    };
    Ok(body.into())
}

pub async fn trakt_watchlist_action(
    action: TraktActions,
    imdb_url: StackString,
    _: LoggedUser,
    state: AppState,
) -> WarpResult<impl Reply> {
    let req = WatchlistActionRequest { action, imdb_url };
    let imdb_url = req.handle(&state.db, &state.trakt).await?;
    let body: String = watchlist_action_worker(&state.trakt, action, &imdb_url)
        .await?
        .into();
    Ok(warp::reply::html(body))
}

fn trakt_watched_seasons_worker(link: &str, imdb_url: &str, entries: &[ImdbSeason]) -> StackString {
    let button_add = r#"
        <td>
        <button type="submit" id="ID"
            onclick="imdb_update('SHOW', 'LINK', SEASON, '/trakt/watched/list/LINK');"
            >update database</button></td>"#;

    let entries = entries
        .iter()
        .map(|s| {
            format!(
                "<tr><td>{}<td>{}<td>{}<td>{}</tr>",
                format!(
                    r#"<a href="javascript:updateMainArticle('/trakt/watched/list/{}/{}')">{}</t>"#,
                    imdb_url, s.season, s.title
                ),
                s.season,
                s.nepisodes,
                button_add
                    .replace("SHOW", &s.show)
                    .replace("LINK", &link)
                    .replace("SEASON", &s.season.to_string())
            )
        })
        .join("");

    let previous = r#"<a href="javascript:updateMainArticle('/trakt/watchlist')">Go Back</a><br>"#;
    format!(r#"{}<table border="0">{}</table>"#, previous, entries).into()
}

pub async fn trakt_watched_seasons(
    imdb_url: StackString,
    _: LoggedUser,
    state: AppState,
) -> WarpResult<impl Reply> {
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
    Ok(warp::reply::html(body))
}

pub async fn trakt_watched_list(
    imdb_url: StackString,
    season: i32,
    _: LoggedUser,
    state: AppState,
) -> WarpResult<impl Reply> {
    let mock_stdout = MockStdout::new();
    let stdout = StdoutChannel::with_mock_stdout(mock_stdout.clone(), mock_stdout.clone());

    let body: String = watch_list_http_worker(&state.config, &state.db, &stdout, &imdb_url, season)
        .await?
        .into();
    Ok(warp::reply::html(body))
}

pub async fn trakt_watched_action(
    action: TraktActions,
    imdb_url: StackString,
    season: i32,
    episode: i32,
    _: LoggedUser,
    state: AppState,
) -> WarpResult<impl Reply> {
    let mock_stdout = MockStdout::new();
    let stdout = StdoutChannel::with_mock_stdout(mock_stdout.clone(), mock_stdout.clone());

    let body: String = watched_action_http_worker(
        &state.trakt,
        &state.db,
        action,
        &imdb_url,
        season,
        episode,
        &state.config,
        &stdout,
    )
    .await?
    .into();
    Ok(warp::reply::html(body))
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

pub async fn trakt_cal(_: LoggedUser, state: AppState) -> WarpResult<impl Reply> {
    let entries = trakt_cal_http_worker(&state.trakt, &state.db).await?;
    let body: String = trakt_cal_worker(&entries).into();
    Ok(warp::reply::html(body))
}

pub async fn trakt_auth_url(_: LoggedUser, state: AppState) -> WarpResult<impl Reply> {
    state.trakt.init().await;
    let url = state
        .trakt
        .get_auth_url()
        .await
        .map_err(Into::<Error>::into)?;
    Ok(warp::reply::html(url.into_string()))
}

#[derive(Serialize, Deserialize)]
pub struct TraktCallbackRequest {
    pub code: StackString,
    pub state: StackString,
}

pub async fn trakt_callback(
    query: TraktCallbackRequest,
    _: LoggedUser,
    state: AppState,
) -> WarpResult<impl Reply> {
    state.trakt.init().await;
    state
        .trakt
        .exchange_code_for_auth_token(query.code.as_str(), query.state.as_str())
        .await
        .map_err(Into::<Error>::into)?;
    let body = r#"
        <title>Trakt auth code received!</title>
        This window can be closed.
        <script language="JavaScript" type="text/javascript">window.close()</script>"#;
    Ok(warp::reply::html(body))
}

pub async fn refresh_auth(_: LoggedUser, state: AppState) -> WarpResult<impl Reply> {
    state.trakt.init().await;
    state
        .trakt
        .exchange_refresh_token()
        .await
        .map_err(Into::<Error>::into)?;
    Ok(warp::reply::html("finished"))
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
        let show = match ImdbRatings::get_show_by_link(&cal.link, &pool).await? {
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
            .get_index(&pool)
            .await?;

            match idx_opt {
                Some(idx) => ImdbEpisodes::from_index(idx, &pool).await?,
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
    stdout: &StdoutChannel,
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

    let show = ImdbRatings::get_show_by_link(imdb_url, &pool)
        .await?
        .ok_or_else(|| format_err!("Show Doesn't exist"))?;

    let watched_episodes_db: HashSet<i32> = get_watched_shows_db(&pool, &show.show, Some(season))
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
    stdout: &StdoutChannel,
) -> HttpResult<StackString> {
    let mc = MovieCollection::new(config, pool, stdout);
    let imdb_url = Arc::new(imdb_url.to_owned());
    trakt.init().await;
    let body = match action {
        TraktActions::Add => {
            let result = if season != -1 && episode != -1 {
                let imdb_url_ = Arc::clone(&imdb_url);
                trakt
                    .add_episode_to_watched(&imdb_url_, season, episode)
                    .await?
            } else {
                let imdb_url_ = Arc::clone(&imdb_url);
                trakt.add_movie_to_watched(&imdb_url_).await?
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
