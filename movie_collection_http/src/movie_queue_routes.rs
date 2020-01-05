#![allow(clippy::needless_pass_by_value)]

use actix_web::web::{block, Data, Json, Path, Query};
use actix_web::HttpResponse;
use failure::{err_msg, Error};
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::path;
use subprocess::Exec;

use super::logged_user::LoggedUser;
use super::movie_queue_app::AppState;
use super::movie_queue_requests::{
    FindNewEpisodeRequest, ImdbEpisodesSyncRequest, ImdbEpisodesUpdateRequest, ImdbRatingsRequest,
    ImdbRatingsSyncRequest, ImdbRatingsUpdateRequest, ImdbSeasonsRequest, ImdbShowRequest,
    LastModifiedRequest, MovieCollectionSyncRequest, MovieCollectionUpdateRequest,
    MoviePathRequest, MovieQueueRequest, MovieQueueSyncRequest, MovieQueueUpdateRequest,
    ParseImdbRequest, QueueDeleteRequest, TraktCalRequest, TvShowsRequest, WatchedActionRequest,
    WatchedListRequest, WatchlistActionRequest, WatchlistShowsRequest,
};
use super::HandleRequest;
use movie_collection_lib::common::make_queue::movie_queue_http;
use movie_collection_lib::common::movie_collection::ImdbSeason;
use movie_collection_lib::common::movie_collection::TvShowsResult;
use movie_collection_lib::common::movie_queue::MovieQueueResult;
use movie_collection_lib::common::trakt_instance;
use movie_collection_lib::common::trakt_utils::TraktActions;
use movie_collection_lib::common::trakt_utils::WatchListShow;
use movie_collection_lib::common::tv_show_source::TvShowSource;
use movie_collection_lib::common::utils::remcom_single_file;

fn form_http_response(body: String) -> Result<HttpResponse, Error> {
    Ok(HttpResponse::Ok()
        .content_type("text/html; charset=utf-8")
        .body(body))
}

fn to_json<T>(js: &T) -> Result<HttpResponse, Error>
where
    T: Serialize,
{
    Ok(HttpResponse::Ok().json2(js))
}

fn movie_queue_body(patterns: &[String], entries: &[String]) -> String {
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
        entries.join("\n")
    );

    entries
}

async fn queue_body_resp(patterns: Vec<String>, queue: Vec<MovieQueueResult>) -> Result<HttpResponse, Error> {
    let entries = block(move || movie_queue_http(&queue)).await.map_err(err_msg)?;
    let body = movie_queue_body(&patterns, &entries);
    let resp = HttpResponse::Ok()
        .content_type("text/html; charset=utf-8")
        .body(body);
    Ok(resp)
}

pub async fn movie_queue(_: LoggedUser, state: Data<AppState>) -> Result<HttpResponse, Error> {
    let req = MovieQueueRequest {
        patterns: Vec::new(),
    };
    let (queue, _) = block(move || state.db.handle(req)).await.map_err(err_msg)?;
    queue_body_resp(Vec::new(), queue).await
}

pub async fn movie_queue_show(
    path: Path<String>,
    _: LoggedUser,
    state: Data<AppState>,
) -> Result<HttpResponse, Error> {
    let path = path.into_inner();
    let patterns = vec![path];

    let req = MovieQueueRequest { patterns };
    let (queue, patterns) = block(move || state.db.handle(req)).await.map_err(err_msg)?;
    queue_body_resp(patterns, queue).await
}

pub async fn movie_queue_delete(
    path: Path<String>,
    _: LoggedUser,
    state: Data<AppState>,
) -> Result<HttpResponse, Error> {
    let path = path.into_inner();

    let req = QueueDeleteRequest { path };
    let body = block(move || state.db.handle(req)).await.map_err(err_msg)?;
    form_http_response(body)
}

fn transcode_worker(
    directory: Option<&str>,
    entries: &[MovieQueueResult],
) -> Result<HttpResponse, Error> {
    let entries: Result<Vec<_>, Error> = entries
        .iter()
        .map(|entry| {
            remcom_single_file(&entry.path, directory, false)?;
            Ok(format!("{}", entry))
        })
        .collect();
    let resp = HttpResponse::Ok()
        .content_type("text/html; charset=utf-8")
        .body(entries?.join("\n"));
    Ok(resp)
}

pub async fn movie_queue_transcode(
    path: Path<String>,
    _: LoggedUser,
    state: Data<AppState>,
) -> Result<HttpResponse, Error> {
    let path = path.into_inner();
    let patterns = vec![path];

    let req = MovieQueueRequest { patterns };
    let (entries, _) = block(move || state.db.handle(req)).await.map_err(err_msg)?;
    transcode_worker(None, &entries)
}

pub async fn movie_queue_transcode_directory(
    path: Path<(String, String)>,
    _: LoggedUser,
    state: Data<AppState>,
) -> Result<HttpResponse, Error> {
    let (directory, file) = path.into_inner();
    let patterns = vec![file];

    let req = MovieQueueRequest { patterns };
    let (entries, _) = block(move || state.db.handle(req)).await.map_err(err_msg)?;
    transcode_worker(Some(&directory), &entries)
}

fn play_worker(full_path: String) -> Result<HttpResponse, Error> {
    let path = path::Path::new(&full_path);

    let file_name = path
        .file_name()
        .ok_or_else(|| err_msg("Invalid path"))?
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

    let command = format!("rm -f /var/www/html/videos/partial/{}", file_name);
    Exec::shell(&command).join().map_err(err_msg)?;
    let command = format!(
        "ln -s {} /var/www/html/videos/partial/{}",
        full_path, file_name
    );
    Exec::shell(&command).join().map_err(err_msg)?;

    let resp = HttpResponse::Ok()
        .content_type("text/html; charset=utf-8")
        .body(body);
    Ok(resp)
}

pub async fn movie_queue_play(
    idx: Path<i32>,
    _: LoggedUser,
    state: Data<AppState>,
) -> Result<HttpResponse, Error> {
    let idx = idx.into_inner();

    let req = MoviePathRequest { idx };
    let x = block(move || state.db.handle(req)).await.map_err(err_msg)?;
    play_worker(x)
}

pub async fn imdb_show(
    path: Path<String>,
    query: Query<ParseImdbRequest>,
    _: LoggedUser,
    state: Data<AppState>,
) -> Result<HttpResponse, Error> {
    let show = path.into_inner();
    let query = query.into_inner();

    let req = ImdbShowRequest { show, query };
    let x = block(move || state.db.handle(req)).await.map_err(err_msg)?;
    form_http_response(x)
}

fn new_episode_worker(entries: &[String]) -> Result<HttpResponse, Error> {
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
        entries.join("\n")
    );
    let resp = HttpResponse::Ok()
        .content_type("text/html; charset=utf-8")
        .body(entries);
    Ok(resp)
}

pub async fn find_new_episodes(
    query: Query<FindNewEpisodeRequest>,
    _: LoggedUser,
    state: Data<AppState>,
) -> Result<HttpResponse, Error> {
    let req = query.into_inner();
    let entries = block(move || state.db.handle(req)).await.map_err(err_msg)?;
    new_episode_worker(&entries)
}

pub async fn imdb_episodes_route(
    query: Query<ImdbEpisodesSyncRequest>,
    _: LoggedUser,
    state: Data<AppState>,
) -> Result<HttpResponse, Error> {
    let req = query.into_inner();
    let x = block(move || state.db.handle(req)).await.map_err(err_msg)?;
    to_json(&x)
}

pub async fn imdb_episodes_update(
    data: Json<ImdbEpisodesUpdateRequest>,
    _: LoggedUser,
    state: Data<AppState>,
) -> Result<HttpResponse, Error> {
    let episodes = data.into_inner();

    let req = episodes;
    block(move || state.db.handle(req)).await.map_err(err_msg)?;
    form_http_response("Success".to_string())
}

pub async fn imdb_ratings_route(
    query: Query<ImdbRatingsSyncRequest>,
    _: LoggedUser,
    state: Data<AppState>,
) -> Result<HttpResponse, Error> {
    let req = query.into_inner();
    let x = block(move || state.db.handle(req)).await.map_err(err_msg)?;
    to_json(&x)
}

pub async fn imdb_ratings_update(
    data: Json<ImdbRatingsUpdateRequest>,
    _: LoggedUser,
    state: Data<AppState>,
) -> Result<HttpResponse, Error> {
    let shows = data.into_inner();

    let req = shows;
    block(move || state.db.handle(req)).await.map_err(err_msg)?;
    form_http_response("Success".to_string())
}

pub async fn movie_queue_route(
    query: Query<MovieQueueSyncRequest>,
    _: LoggedUser,
    state: Data<AppState>,
) -> Result<HttpResponse, Error> {
    let req = query.into_inner();
    let x = block(move || state.db.handle(req)).await.map_err(err_msg)?;
    to_json(&x)
}

pub async fn movie_queue_update(
    data: Json<MovieQueueUpdateRequest>,
    _: LoggedUser,
    state: Data<AppState>,
) -> Result<HttpResponse, Error> {
    let queue = data.into_inner();

    let req = queue;
    block(move || state.db.handle(req)).await.map_err(err_msg)?;
    form_http_response("Success".to_string())
}

pub async fn movie_collection_route(
    query: Query<MovieCollectionSyncRequest>,
    _: LoggedUser,
    state: Data<AppState>,
) -> Result<HttpResponse, Error> {
    let req = query.into_inner();
    let x = block(move || state.db.handle(req)).await.map_err(err_msg)?;
    to_json(&x)
}

pub async fn movie_collection_update(
    data: Json<MovieCollectionUpdateRequest>,
    _: LoggedUser,
    state: Data<AppState>,
) -> Result<HttpResponse, Error> {
    let collection = data.into_inner();

    let req = collection;
    block(move || state.db.handle(req)).await.map_err(err_msg)?;
    form_http_response("Success".to_string())
}

pub async fn last_modified_route(
    _: LoggedUser,
    state: Data<AppState>,
) -> Result<HttpResponse, Error> {
    let req = LastModifiedRequest {};
    let x = block(move || state.db.handle(req)).await.map_err(err_msg)?;
    to_json(&x)
}

pub async fn frontpage(_: LoggedUser, _: Data<AppState>) -> Result<HttpResponse, Error> {
    form_http_response(include_str!("../../templates/index.html").replace("BODY", ""))
}

type TvShowsMap = HashMap<String, (String, WatchListShow, Option<TvShowSource>)>;

#[derive(Debug, Default)]
struct ProcessShowItem {
    show: String,
    title: String,
    link: String,
    source: Option<TvShowSource>,
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

fn tvshows_worker(res1: TvShowsMap, tvshows: Vec<TvShowsResult>) -> Result<String, Error> {
    let tvshows: HashMap<String, _> = tvshows
        .into_iter()
        .map(|s| {
            let item: ProcessShowItem = s.into();
            (item.link.clone(), item)
        })
        .collect();
    let watchlist: HashMap<String, _> = res1
        .into_iter()
        .map(|(link, (show, s, source))| {
            let item = ProcessShowItem {
                show,
                title: s.title,
                link: s.link,
                source,
            };
            (link, item)
        })
        .collect();

    let shows = process_shows(tvshows, watchlist)?;

    let previous = r#"
        <a href="javascript:updateMainArticle('/list/watchlist')">Go Back</a><br>
        <a href="javascript:updateMainArticle('/list/trakt/watchlist')">Watch List</a>
        <button name="remcomout" id="remcomoutput"> &nbsp; </button><br>
    "#;

    let entries = format!(
        r#"{}<table border="0">{}</table>"#,
        previous,
        shows.join("\n")
    );

    Ok(entries)
}

pub async fn tvshows(_: LoggedUser, state: Data<AppState>) -> Result<HttpResponse, Error> {
    let s = state.clone();
    let shows = block(move || s.db.handle(TvShowsRequest {}))
        .await
        .map_err(err_msg)?;
    let res1 = block(move || state.db.handle(WatchlistShowsRequest {}))
        .await
        .map_err(err_msg)?;
    let entries = block(move || tvshows_worker(res1, shows))
        .await
        .map_err(err_msg)?;

    let resp = HttpResponse::Ok()
        .content_type("text/html; charset=utf-8")
        .body(entries);
    Ok(resp)
}

fn process_shows(
    tvshows: HashMap<String, ProcessShowItem>,
    watchlist: HashMap<String, ProcessShowItem>,
) -> Result<Vec<String>, Error> {
    let watchlist_keys: HashSet<_> = watchlist.keys().cloned().collect();
    let watchlist_shows: Vec<_> = watchlist
        .into_iter()
        .filter_map(|(_, item)| match tvshows.get(&item.link) {
            None => Some(item),
            Some(_) => None,
        })
        .collect();

    let tvshow_keys: HashSet<_> = tvshows.keys().cloned().collect();
    let mut shows: Vec<_> = tvshows
        .into_iter()
        .map(|(_, v)| v)
        .chain(watchlist_shows.into_iter())
        .collect();
    shows.sort_by_key(|item| item.show.clone());

    let button_add = r#"<td><button type="submit" id="ID" onclick="watchlist_add('SHOW');">add to watchlist</button></td>"#;
    let button_rm = r#"<td><button type="submit" id="ID" onclick="watchlist_rm('SHOW');">remove from watchlist</button></td>"#;

    let shows: Vec<_> = shows
        .into_iter()
        .map(|item| {
            let has_watchlist = watchlist_keys.contains(&item.link);
            format!(
                r#"<tr><td>{}</td>
                <td><a href="https://www.imdb.com/title/{}">imdb</a></td><td>{}</td><td>{}</td><td>{}</td></tr>"#,
                if tvshow_keys.contains(&item.link) {
                    format!(r#"<a href="javascript:updateMainArticle('/list/{}')">{}</a>"#, item.show, item.title)
                } else {
                    format!(
                        r#"<a href="javascript:updateMainArticle('/list/trakt/watched/list/{}')">{}</a>"#,
                        item.link, item.title
                    )
                },
                item.link,
                match item.source {
                    Some(TvShowSource::Netflix) => r#"<a href="https://netflix.com">netflix</a>"#,
                    Some(TvShowSource::Hulu) => r#"<a href="https://hulu.com">hulu</a>"#,
                    Some(TvShowSource::Amazon) => r#"<a href="https://amazon.com">amazon</a>"#,
                    _ => "",
                },
                if has_watchlist {
                    format!(r#"<a href="javascript:updateMainArticle('/list/trakt/watched/list/{}')">watchlist</a>"#, item.link)
                } else {
                    "".to_string()
                },
                if has_watchlist {
                    button_rm.replace("SHOW", &item.link)
                } else {
                    button_add.replace("SHOW", &item.link)
                },
            )
        })
        .collect();
    Ok(shows)
}

fn watchlist_worker(
    shows: HashMap<String, (String, WatchListShow, Option<TvShowSource>)>,
) -> Result<HttpResponse, Error> {
    let mut shows: Vec<_> = shows
        .into_iter()
        .map(|(_, (_, s, source))| (s.title, s.link, source))
        .collect();

    shows.sort();

    let shows: Vec<_> = shows
        .into_iter()
        .map(|(title, link, source)| {
            format!(
                r#"<tr><td>{}</td>
            <td><a href="https://www.imdb.com/title/{}">imdb</a> {} </tr>"#,
                format!(
                    r#"<a href="javascript:updateMainArticle('/list/trakt/watched/list/{}')">{}</a>"#,
                    link, title
                ),
                link,
                match source {
                    Some(TvShowSource::Netflix) => {
                        r#"<td><a href="https://netflix.com">netflix</a>"#
                    }
                    Some(TvShowSource::Hulu) => r#"<td><a href="https://hulu.com">netflix</a>"#,
                    Some(TvShowSource::Amazon) => r#"<td><a href="https://amazon.com">netflix</a>"#,
                    _ => "",
                },
            )
        })
        .collect();

    let previous = r#"<a href="javascript:updateMainArticle('/list/tvshows')">Go Back</a><br>"#;
    let entries = format!(
        r#"{}<table border="0">{}</table>"#,
        previous,
        shows.join("\n")
    );

    let resp = HttpResponse::Ok()
        .content_type("text/html; charset=utf-8")
        .body(entries);
    Ok(resp)
}

pub async fn trakt_watchlist(_: LoggedUser, state: Data<AppState>) -> Result<HttpResponse, Error> {
    let req = WatchlistShowsRequest {};
    let x = block(move || state.db.handle(req)).await.map_err(err_msg)?;
    watchlist_worker(x)
}

fn watchlist_action_worker(action: TraktActions, imdb_url: &str) -> Result<HttpResponse, Error> {
    let body = match action {
        TraktActions::Add => trakt_instance::add_watchlist_show(&imdb_url)?.to_string(),
        TraktActions::Remove => trakt_instance::remove_watchlist_show(&imdb_url)?.to_string(),
        _ => "".to_string(),
    };
    let resp = HttpResponse::Ok()
        .content_type("text/html; charset=utf-8")
        .body(body);
    Ok(resp)
}

pub async fn trakt_watchlist_action(
    path: Path<(String, String)>,
    _: LoggedUser,
    state: Data<AppState>,
) -> Result<HttpResponse, Error> {
    let (action, imdb_url) = path.into_inner();
    let action = TraktActions::from_command(&action);

    let req = WatchlistActionRequest { action, imdb_url };
    let imdb_url = block(move || state.db.handle(req)).await.map_err(err_msg)?;
    watchlist_action_worker(action, &imdb_url)
}

fn trakt_watched_seasons_worker(
    link: &str,
    imdb_url: &str,
    entries: &[ImdbSeason],
) -> Result<String, Error> {
    let button_add = r#"
        <td>
        <button type="submit" id="ID"
            onclick="imdb_update('SHOW', 'LINK', SEASON, '/list/trakt/watched/list/LINK');"
            >update database</button></td>"#;

    let entries: Vec<_> = entries
        .iter()
        .map(|s| {
            format!(
                "<tr><td>{}<td>{}<td>{}<td>{}</tr>",
                format!(
                    r#"<a href="javascript:updateMainArticle('/list/trakt/watched/list/{}/{}')">{}</t>"#,
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
        .collect();

    let previous =
        r#"<a href="javascript:updateMainArticle('/list/trakt/watchlist')">Go Back</a><br>"#;
    let entries = format!(
        r#"{}<table border="0">{}</table>"#,
        previous,
        entries.join("\n")
    );
    Ok(entries)
}

pub async fn trakt_watched_seasons(
    path: Path<String>,
    _: LoggedUser,
    state: Data<AppState>,
) -> Result<HttpResponse, Error> {
    let imdb_url = path.into_inner();
    let s = state.clone();
    let show_opt = block(move || s.db.handle(ImdbRatingsRequest { imdb_url }))
        .await
        .map_err(err_msg)?;
    let empty = || ("".to_string(), "".to_string(), "".to_string());
    let (imdb_url, show, link) =
        show_opt.map_or_else(empty, |(imdb_url, t)| (imdb_url, t.show, t.link));
    let entries = block(move || state.db.handle(ImdbSeasonsRequest { show }))
        .await
        .map_err(err_msg)?;
    let entries = block(move || trakt_watched_seasons_worker(&link, &imdb_url, &entries))
        .await
        .map_err(err_msg)?;
    let resp = HttpResponse::Ok()
        .content_type("text/html; charset=utf-8")
        .body(entries);
    Ok(resp)
}

pub async fn trakt_watched_list(
    path: Path<(String, i32)>,
    _: LoggedUser,
    state: Data<AppState>,
) -> Result<HttpResponse, Error> {
    let (imdb_url, season) = path.into_inner();

    let req = WatchedListRequest { imdb_url, season };
    let x = block(move || state.db.handle(req)).await.map_err(err_msg)?;
    form_http_response(x)
}

pub async fn trakt_watched_action(
    path: Path<(String, String, i32, i32)>,
    _: LoggedUser,
    state: Data<AppState>,
) -> Result<HttpResponse, Error> {
    let (action, imdb_url, season, episode) = path.into_inner();

    let req = WatchedActionRequest {
        action: TraktActions::from_command(&action),
        imdb_url,
        season,
        episode,
    };
    let x = block(move || state.db.handle(req)).await.map_err(err_msg)?;
    form_http_response(x)
}

fn trakt_cal_worker(entries: &[String]) -> Result<HttpResponse, Error> {
    let previous = r#"<a href="javascript:updateMainArticle('/list/tvshows')">Go Back</a><br>"#;
    let entries = format!(
        r#"{}<table border="0">{}</table>"#,
        previous,
        entries.join("\n")
    );
    let resp = HttpResponse::Ok()
        .content_type("text/html; charset=utf-8")
        .body(entries);
    Ok(resp)
}

pub async fn trakt_cal(_: LoggedUser, state: Data<AppState>) -> Result<HttpResponse, Error> {
    let req = TraktCalRequest {};
    let entries = block(move || state.db.handle(req)).await.map_err(err_msg)?;
    trakt_cal_worker(&entries)
}
