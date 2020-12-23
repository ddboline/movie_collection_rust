#![allow(clippy::needless_pass_by_value)]

use actix_web::{
    web::{Data, Path, Query},
    HttpResponse,
};
use anyhow::format_err;
use itertools::Itertools;
use serde::{Deserialize, Serialize};
use stack_string::StackString;
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::Arc,
};
use stdout_channel::{MockStdout, StdoutChannel};
use tokio::{fs::remove_file, task::spawn_blocking};

use movie_collection_lib::{
    config::Config,
    imdb_episodes::ImdbEpisodes,
    imdb_ratings::ImdbRatings,
    make_list::FileLists,
    movie_collection::{ImdbSeason, MovieCollection},
    movie_queue::MovieQueueDB,
    pgpool::PgPool,
    trakt_connection::TraktConnection,
    trakt_utils::{
        get_watched_shows_db, get_watchlist_shows_db_map, TraktActions, WatchListShow,
        WatchedEpisode, WatchedMovie,
    },
    transcode_service::{transcode_status, TranscodeService, TranscodeServiceRequest},
    tv_show_source::TvShowSource,
};

use super::{
    app::{AppState, CONFIG},
    errors::ServiceError as Error,
    logged_user::LoggedUser,
    requests::{ImdbSeasonsRequest, WatchlistActionRequest},
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
) -> Result<StackString, Error> {
    trakt.init().await;
    let body = match action {
        TraktActions::Add => trakt.add_watchlist_show(&imdb_url).await?.to_string(),
        TraktActions::Remove => trakt.remove_watchlist_show(&imdb_url).await?.to_string(),
        _ => "".to_string(),
    };
    Ok(body.into())
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
    form_http_response(body.into())
}

fn trakt_watched_seasons_worker(
    link: &str,
    imdb_url: &str,
    entries: &[ImdbSeason],
) -> Result<StackString, Error> {
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

    let mock_stdout = MockStdout::new();
    let stdout = StdoutChannel::with_mock_stdout(mock_stdout.clone(), mock_stdout.clone());

    let body = watch_list_http_worker(&CONFIG, &state.db, &stdout, &imdb_url, season).await?;
    form_http_response(body.into())
}

pub async fn trakt_watched_action(
    path: Path<(StackString, StackString, i32, i32)>,
    _: LoggedUser,
    state: Data<AppState>,
) -> HttpResult {
    let (action, imdb_url, season, episode) = path.into_inner();

    let mock_stdout = MockStdout::new();
    let stdout = StdoutChannel::with_mock_stdout(mock_stdout.clone(), mock_stdout.clone());

    let body = watched_action_http_worker(
        &state.trakt,
        &state.db,
        action.parse().expect("impossible"),
        &imdb_url,
        season,
        episode,
        &CONFIG,
        &stdout,
    )
    .await?;
    form_http_response(body.into())
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

pub async fn trakt_cal(_: LoggedUser, state: Data<AppState>) -> HttpResult {
    let entries = trakt_cal_http_worker(&state.trakt, &state.db).await?;
    let body = trakt_cal_worker(&entries);
    form_http_response(body.into())
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
) -> Result<StackString, Error> {
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
) -> Result<StackString, Error> {
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
