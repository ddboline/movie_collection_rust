#![allow(clippy::needless_pass_by_value)]

use actix_web::{
    web::{Data, Path, Query},
    HttpResponse,
};
use itertools::Itertools;
use serde::{Deserialize, Serialize};
use stack_string::StackString;
use std::{
    collections::{HashMap},
    path::PathBuf,
};
use stdout_channel::{MockStdout, StdoutChannel};
use tokio::{fs::remove_file, task::spawn_blocking};

use movie_collection_lib::{
    imdb_ratings::ImdbRatings,
    make_list::FileLists,
    movie_collection::{ImdbSeason, },
    trakt_connection::TraktConnection,
    trakt_utils::{get_watchlist_shows_db_map, trakt_cal_http_worker, TraktActions, WatchListShow},
    transcode_service::{transcode_status, TranscodeService, TranscodeServiceRequest},
    tv_show_source::TvShowSource,
};

use super::{
    errors::ServiceError as Error,
    logged_user::LoggedUser,
    app::{AppState, CONFIG},
    requests::{
        ImdbSeasonsRequest,
        WatchedActionRequest, WatchedListRequest, WatchlistActionRequest,
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

fn watchlist_worker(
    shows: HashMap<StackString, (StackString, WatchListShow, Option<TvShowSource>)>,
) -> Result<StackString, Error> {
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
                    r#"<a href="javascript:updateMainArticle('/list/trakt/watched/list/{}')">{}</a>"#,
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
    let entries = format!(r#"{}<table border="0">{}</table>"#, previous, shows).into();

    Ok(entries)
}

pub async fn trakt_watchlist(_: LoggedUser, state: Data<AppState>) -> HttpResult {
    let shows = get_watchlist_shows_db_map(&state.db).await?;
    let body = watchlist_worker(shows)?;
    form_http_response(body.into())
}

async fn watchlist_action_worker(
    trakt: &TraktConnection,
    action: TraktActions,
    imdb_url: &str,
) -> HttpResult {
    trakt.init().await;
    let body = match action {
        TraktActions::Add => trakt.add_watchlist_show(&imdb_url).await?.to_string(),
        TraktActions::Remove => trakt.remove_watchlist_show(&imdb_url).await?.to_string(),
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
    let imdb_url = req.handle(&state.db, &state.trakt).await?;
    watchlist_action_worker(&state.trakt, action, &imdb_url).await
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

    let show_opt = ImdbRatings::get_show_by_link(&imdb_url, &state.db)
    .await
    .map(|s| s.map(|sh| (imdb_url, sh)))?;

    let empty = || ("".into(), "".into(), "".into());
    let (imdb_url, show, link) =
        show_opt.map_or_else(empty, |(imdb_url, t)| (imdb_url, t.show, t.link));
    let req = ImdbSeasonsRequest { show };
    let entries = req.handle(&state.db).await?;
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
    let x = req.handle(&state.db).await?;
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
    let body = req.handle(&state.db, &state.trakt).await?;
    form_http_response(body.into())
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
    let entries = trakt_cal_http_worker(&state.trakt, &state.db).await?;
    trakt_cal_worker(&entries)
}

pub async fn user(user: LoggedUser) -> HttpResult {
    to_json(user)
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

pub async fn movie_queue_transcode_status(_: LoggedUser, _: Data<AppState>) -> HttpResult {
    let task = spawn_blocking(move || FileLists::get_file_lists(&CONFIG));
    let status = transcode_status(&CONFIG).await?;
    let file_lists = task.await.unwrap()?;
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
