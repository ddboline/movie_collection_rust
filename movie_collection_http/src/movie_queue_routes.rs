#![allow(clippy::needless_pass_by_value)]

use actix_web::{
    web::{Data, Json, Path, Query},
    HttpResponse,
};
use anyhow::format_err;
use itertools::Itertools;
use maplit::hashmap;
use serde::{Deserialize, Serialize};
use stack_string::StackString;
use std::{
    borrow::Borrow,
    collections::{HashMap, HashSet},
    fs::remove_file,
    hash::{Hash, Hasher},
    os::unix::fs::symlink,
    path,
};

use movie_collection_lib::{
    make_queue::movie_queue_http,
    movie_collection::{ImdbSeason, TvShowsResult},
    movie_queue::MovieQueueResult,
    pgpool::PgPool,
    trakt_utils::{TraktActions, WatchListShow, TRAKT_CONN},
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
        ImdbRatingsRequest, ImdbRatingsSyncRequest, ImdbRatingsUpdateRequest, ImdbSeasonsRequest,
        ImdbShowRequest, LastModifiedRequest, MovieCollectionSyncRequest,
        MovieCollectionUpdateRequest, MoviePathRequest, MovieQueueRequest, MovieQueueSyncRequest,
        MovieQueueUpdateRequest, ParseImdbRequest, QueueDeleteRequest, TraktCalRequest,
        TvShowsRequest, WatchedActionRequest, WatchedListRequest, WatchlistActionRequest,
        WatchlistShowsRequest,
    },
    HandleRequest,
};

pub type HttpResult = Result<HttpResponse, Error>;

fn form_http_response(body: String) -> HttpResult {
    Ok(HttpResponse::Ok()
        .content_type("text/html; charset=utf-8")
        .body(body))
}

fn to_json<T>(js: T) -> HttpResult
where
    T: Serialize,
{
    Ok(HttpResponse::Ok().json(js))
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

async fn queue_body_resp(
    patterns: Vec<StackString>,
    queue: Vec<MovieQueueResult>,
    pool: &PgPool,
) -> HttpResult {
    let entries = movie_queue_http(&queue, pool).await?;
    let body = movie_queue_body(&patterns, &entries);
    form_http_response(body.into())
}

pub async fn movie_queue(_: LoggedUser, state: Data<AppState>) -> HttpResult {
    let req = MovieQueueRequest {
        patterns: Vec::new(),
    };
    let (queue, _) = state.db.handle(req).await?;
    queue_body_resp(Vec::new(), queue, &state.db).await
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
    queue_body_resp(patterns, queue, &state.db).await
}

pub async fn movie_queue_delete(
    path: Path<StackString>,
    _: LoggedUser,
    state: Data<AppState>,
) -> HttpResult {
    let path = path.into_inner();

    let req = QueueDeleteRequest { path };
    let body = state.db.handle(req).await?;
    form_http_response(body.into())
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
    let (entries, _) = state.db.handle(req).await?;
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
    let (entries, _) = state.db.handle(req).await?;
    transcode_worker(Some(&path::Path::new(directory.as_str())), &entries).await
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
        remove_file(&partial_path)?;
    }

    symlink(&full_path, &partial_path)?;
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
        <a href="javascript:updateMainArticle('/list/trakt/watchlist')">Watch List</a>
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
    let s = state.clone();
    let shows = s.db.handle(TvShowsRequest {}).await?;
    let res1 = state.db.handle(WatchlistShowsRequest {}).await?;
    let entries = tvshows_worker(res1, shows)?;
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
                        r#"<a href="javascript:updateMainArticle('/list/trakt/watched/list/{}')">{}</a>"#,
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
                    format!(r#"<a href="javascript:updateMainArticle('/list/trakt/watched/list/{}')">watchlist</a>"#, item.link)
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

fn watchlist_worker(
    shows: HashMap<StackString, (StackString, WatchListShow, Option<TvShowSource>)>,
) -> HttpResult {
    let mut shows: Vec<_> = shows
        .into_iter()
        .map(|(_, (_, s, source))| (s.title, s.link, source))
        .collect();

    shows.sort();

    let shows = shows
        .into_iter()
        .map(|(title, link, source)| {
            format!(
                r#"<tr><td>{}</td>
            <td><a href="https://www.imdb.com/title/{}" target="_blank">imdb</a> {} </tr>"#,
                format!(
                    r#"<a href="javascript:updateMainArticle('/list/trakt/watched/list/{}')">{}</a>"#,
                    link, title
                ),
                link,
                match source {
                    Some(TvShowSource::Netflix) => {
                        r#"<td><a href="https://netflix.com" target="_blank">netflix</a>"#
                    }
                    Some(TvShowSource::Hulu) => r#"<td><a href="https://hulu.com" target="_blank">netflix</a>"#,
                    Some(TvShowSource::Amazon) => r#"<td><a href="https://amazon.com" target="_blank">netflix</a>"#,
                    _ => "",
                },
            )
        })
        .join("");

    let previous = r#"<a href="javascript:updateMainArticle('/list/tvshows')">Go Back</a><br>"#;
    let entries = format!(r#"{}<table border="0">{}</table>"#, previous, shows);

    form_http_response(entries)
}

pub async fn trakt_watchlist(_: LoggedUser, state: Data<AppState>) -> HttpResult {
    let req = WatchlistShowsRequest {};
    let x = state.db.handle(req).await?;
    watchlist_worker(x)
}

async fn watchlist_action_worker(action: TraktActions, imdb_url: &str) -> HttpResult {
    TRAKT_CONN.init().await;
    let body = match action {
        TraktActions::Add => TRAKT_CONN.add_watchlist_show(&imdb_url).await?.to_string(),
        TraktActions::Remove => TRAKT_CONN
            .remove_watchlist_show(&imdb_url)
            .await?
            .to_string(),
        _ => "".to_string(),
    };
    form_http_response(body)
}

pub async fn trakt_watchlist_action(
    path: Path<(StackString, StackString)>,
    _: LoggedUser,
    state: Data<AppState>,
) -> HttpResult {
    let (action, imdb_url) = path.into_inner();
    let action = action.parse().expect("impossible");

    let req = WatchlistActionRequest { action, imdb_url };
    let imdb_url = state.db.handle(req).await?;
    watchlist_action_worker(action, &imdb_url).await
}

fn trakt_watched_seasons_worker(
    link: &str,
    imdb_url: &str,
    entries: &[ImdbSeason],
) -> Result<StackString, Error> {
    let button_add = r#"
        <td>
        <button type="submit" id="ID"
            onclick="imdb_update('SHOW', 'LINK', SEASON, '/list/trakt/watched/list/LINK');"
            >update database</button></td>"#;

    let entries = entries
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
        .join("");

    let previous =
        r#"<a href="javascript:updateMainArticle('/list/trakt/watchlist')">Go Back</a><br>"#;
    let entries = format!(r#"{}<table border="0">{}</table>"#, previous, entries).into();
    Ok(entries)
}

pub async fn trakt_watched_seasons(
    path: Path<StackString>,
    _: LoggedUser,
    state: Data<AppState>,
) -> HttpResult {
    let imdb_url = path.into_inner();
    let s = state.clone();
    let show_opt = s.db.handle(ImdbRatingsRequest { imdb_url }).await?;
    let empty = || ("".into(), "".into(), "".into());
    let (imdb_url, show, link) =
        show_opt.map_or_else(empty, |(imdb_url, t)| (imdb_url, t.show, t.link));
    let entries = state.db.handle(ImdbSeasonsRequest { show }).await?;
    let entries = trakt_watched_seasons_worker(&link, &imdb_url, &entries)?;
    form_http_response(entries.into())
}

pub async fn trakt_watched_list(
    path: Path<(StackString, i32)>,
    _: LoggedUser,
    state: Data<AppState>,
) -> HttpResult {
    let (imdb_url, season) = path.into_inner();

    let req = WatchedListRequest { imdb_url, season };
    let x = state.db.handle(req).await?;
    form_http_response(x.into())
}

pub async fn trakt_watched_action(
    path: Path<(StackString, StackString, i32, i32)>,
    _: LoggedUser,
    state: Data<AppState>,
) -> HttpResult {
    let (action, imdb_url, season, episode) = path.into_inner();

    let req = WatchedActionRequest {
        action: action.parse().expect("impossible"),
        imdb_url,
        season,
        episode,
    };
    let x = state.db.handle(req).await?;
    form_http_response(x.into())
}

fn trakt_cal_worker(entries: &[StackString]) -> HttpResult {
    let previous = r#"<a href="javascript:updateMainArticle('/list/tvshows')">Go Back</a><br>"#;
    let entries = format!(
        r#"{}<table border="0">{}</table>"#,
        previous,
        entries.join("")
    );
    form_http_response(entries)
}

pub async fn trakt_cal(_: LoggedUser, state: Data<AppState>) -> HttpResult {
    let req = TraktCalRequest {};
    let entries = state.db.handle(req).await?;
    trakt_cal_worker(&entries)
}

pub async fn user(user: LoggedUser) -> HttpResult {
    to_json(user)
}

pub async fn trakt_auth_url(_: LoggedUser, _: Data<AppState>) -> HttpResult {
    TRAKT_CONN.init().await;
    let url = TRAKT_CONN.get_auth_url().await?;
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
    _: Data<AppState>,
) -> HttpResult {
    TRAKT_CONN.init().await;
    TRAKT_CONN
        .exchange_code_for_auth_token(query.code.as_str(), query.state.as_str())
        .await?;
    let body = r#"
        <title>Trakt auth code received!</title>
        This window can be closed.
        <script language="JavaScript" type="text/javascript">window.close()</script>"#;
    form_http_response(body.to_string())
}

pub async fn refresh_auth(_: LoggedUser, _: Data<AppState>) -> HttpResult {
    TRAKT_CONN.init().await;
    TRAKT_CONN.exchange_refresh_token().await?;
    form_http_response("finished".to_string())
}

pub async fn movie_queue_transcode_status(_: LoggedUser, _: Data<AppState>) -> HttpResult {
    let status = transcode_status(&CONFIG).await?;
    form_http_response(status.to_string())
}
