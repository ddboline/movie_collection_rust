#![allow(clippy::needless_pass_by_value)]

extern crate actix;
extern crate actix_web;
extern crate futures;
extern crate rust_auth_server;
extern crate subprocess;

use actix_web::{
    http::StatusCode, AsyncResponder, FutureResponse, HttpMessage, HttpRequest, HttpResponse, Path,
    Query,
};
use failure::{err_msg, Error};
use futures::future::Future;
use rust_auth_server::auth_handler::LoggedUser;
use std::collections::{HashMap, HashSet};
use std::path;
use subprocess::Exec;

use super::movie_queue_app::AppState;
use super::movie_queue_requests::{
    ImdbRatingsRequest, ImdbSeasonsRequest, MoviePathRequest, MovieQueueRequest,
    QueueDeleteRequest, TvShowsRequest, WatchlistActionRequest, WatchlistShowsRequest,
};
use crate::common::imdb_ratings::ImdbRatings;
use crate::common::make_queue::movie_queue_http;
use crate::common::movie_collection::{MovieCollection, MovieCollectionDB};
use crate::common::movie_queue::MovieQueueDB;
use crate::common::parse_imdb::parse_imdb_worker;
use crate::common::trakt_utils::{
    get_watched_shows_db, get_watchlist_shows_db_map, TraktActions, TraktConnection,
    WatchedEpisode, WatchedMovie,
};
use crate::common::utils::{map_result_vec, remcom_single_file};

fn process_shows(
    tvshows: HashMap<String, (String, String, String, Option<String>)>,
    watchlist: HashMap<String, (String, String, String, Option<String>)>,
) -> Result<Vec<String>, Error> {
    let watchlist_shows: Vec<_> = watchlist
        .iter()
        .filter_map(|(_, (show, title, link, source))| match tvshows.get(link) {
            None => Some((show.clone(), title.clone(), link.clone(), source.clone())),
            Some(_) => None,
        })
        .collect();

    let mut shows: Vec<_> = tvshows
        .iter()
        .map(|(_, v)| v)
        .chain(watchlist_shows.iter())
        .collect();
    shows.sort_by_key(|(s, _, _, _)| s);

    let button_add = r#"<td><button type="submit" id="ID" onclick="watchlist_add('SHOW');">add to watchlist</button></td>"#;
    let button_rm = r#"<td><button type="submit" id="ID" onclick="watchlist_rm('SHOW');">remove from watchlist</button></td>"#;

    let shows: Vec<_> = shows
        .into_iter()
        .map(|(show, title, link, source)| {
            let has_watchlist = watchlist.contains_key(link);
            format!(
                r#"<tr><td>{}</td>
                <td><a href="https://www.imdb.com/title/{}">imdb</a></td><td>{}</td><td>{}</td><td>{}</td></tr>"#,
                if tvshows.contains_key(link) {
                    format!(r#"<a href="/list/{}">{}</a>"#, show, title)
                } else {
                    format!(
                        r#"<a href="/list/trakt/watched/list/{}">{}</a>"#,
                        link, title
                    )
                },
                link,
                match source.as_ref().map(|s| s.as_str()) {
                    Some("netflix") => r#"<a href="https://netflix.com">netflix</a>"#,
                    Some("hulu") => r#"<a href="https://hulu.com">netflix</a>"#,
                    Some("amazon") => r#"<a href="https://amazon.com">netflix</a>"#,
                    _ => "",
                },
                if has_watchlist {
                    format!(r#"<a href="/list/trakt/watched/list/{}">watchlist</a>"#, link)
                } else {
                    "".to_string()
                },
                if !has_watchlist {
                    button_add.replace("SHOW", link)
                } else {
                    button_rm.replace("SHOW", link)
                },
            )
        })
        .collect();
    Ok(shows)
}

fn send_unauthorized(request: HttpRequest<AppState>) -> FutureResponse<HttpResponse> {
    request
        .body()
        .from_err()
        .and_then(move |_| Ok(HttpResponse::Unauthorized().json("Unauthorized")))
        .responder()
}

pub fn tvshows(user: LoggedUser, request: HttpRequest<AppState>) -> FutureResponse<HttpResponse> {
    if user.email != "ddboline@gmail.com" {
        send_unauthorized(request)
    } else {
        request
            .state()
            .db
            .send(TvShowsRequest {})
            .from_err()
            .join(request.state().db.send(WatchlistShowsRequest {}).from_err())
            .and_then(move |(res0, res1)| match res0 {
                Ok(tvshows) => {
                    let tvshows: HashMap<String, _> = tvshows
                        .into_iter()
                        .map(|s| (s.link.clone(), (s.show, s.title, s.link, s.source)))
                        .collect();
                    let watchlist: HashMap<String, _> = res1.map(|w| {
                        w.into_iter()
                            .map(|(link, (show, s, source))| {
                                (link, (show, s.title, s.link, source))
                            })
                            .collect()
                    })?;

                    let shows = process_shows(tvshows, watchlist)?;

                    let body = include_str!("../../templates/tvshows_template.html")
                        .replace("BODY", &shows.join("\n"));

                    let resp = HttpResponse::build(StatusCode::OK)
                        .content_type("text/html; charset=utf-8")
                        .body(body);
                    Ok(resp)
                }
                Err(err) => Err(err.into()),
            })
            .responder()
    }
}

fn movie_queue_body(patterns: &[String], entries: &[String]) -> String {
    let watchlist_url = if patterns.is_empty() {
        "/list/trakt/watchlist".to_string()
    } else {
        format!("/list/trakt/watched/list/{}", patterns.join("_"))
    };

    let body = include_str!("../../templates/queue_list.html").replace("WATCHLIST", &watchlist_url);
    body.replace("BODY", &entries.join("\n"))
}

pub fn movie_queue(
    user: LoggedUser,
    request: HttpRequest<AppState>,
) -> FutureResponse<HttpResponse> {
    if user.email != "ddboline@gmail.com" {
        send_unauthorized(request)
    } else {
        request
            .state()
            .db
            .send(MovieQueueRequest {
                patterns: Vec::new(),
            })
            .from_err()
            .and_then(move |res| match res {
                Ok(queue) => {
                    let entries = movie_queue_http(&queue)?;
                    let body = movie_queue_body(&[], &entries);
                    let resp = HttpResponse::build(StatusCode::OK)
                        .content_type("text/html; charset=utf-8")
                        .body(body);
                    Ok(resp)
                }
                Err(err) => Err(err.into()),
            })
            .responder()
    }
}

pub fn movie_queue_show(
    path: Path<String>,
    user: LoggedUser,
    request: HttpRequest<AppState>,
) -> FutureResponse<HttpResponse> {
    if user.email != "ddboline@gmail.com" {
        send_unauthorized(request)
    } else {
        let path = path.into_inner();
        let patterns = vec![path];
        request
            .state()
            .db
            .send(MovieQueueRequest {
                patterns: patterns.clone(),
            })
            .from_err()
            .and_then(move |res| match res {
                Ok(queue) => {
                    let entries = movie_queue_http(&queue)?;
                    let body = movie_queue_body(&patterns, &entries);
                    let resp = HttpResponse::build(StatusCode::OK)
                        .content_type("text/html; charset=utf-8")
                        .body(body);
                    Ok(resp)
                }
                Err(err) => Err(err.into()),
            })
            .responder()
    }
}

pub fn movie_queue_delete(
    path: Path<String>,
    user: LoggedUser,
    request: HttpRequest<AppState>,
) -> FutureResponse<HttpResponse> {
    if user.email != "ddboline@gmail.com" {
        send_unauthorized(request)
    } else {
        let path = path.into_inner();

        request
            .state()
            .db
            .send(QueueDeleteRequest { path: path.clone() })
            .from_err()
            .and_then(move |res| match res {
                Ok(_) => {
                    let resp = HttpResponse::build(StatusCode::OK)
                        .content_type("text/html; charset=utf-8")
                        .body(path);
                    Ok(resp)
                }
                Err(err) => Err(err.into()),
            })
            .responder()
    }
}

pub fn movie_queue_transcode(
    path: Path<String>,
    user: LoggedUser,
    request: HttpRequest<AppState>,
) -> FutureResponse<HttpResponse> {
    if user.email != "ddboline@gmail.com" {
        send_unauthorized(request)
    } else {
        let path = path.into_inner();

        let patterns = vec![path];

        request
            .state()
            .db
            .send(MovieQueueRequest {
                patterns: patterns.clone(),
            })
            .from_err()
            .and_then(move |res| match res {
                Ok(entries) => {
                    let entries: Vec<Result<_, Error>> = entries
                        .iter()
                        .map(|entry| {
                            remcom_single_file(&entry.path, &None, false)?;
                            Ok(format!("{}", entry))
                        })
                        .collect();
                    let entries = map_result_vec(entries)?;
                    let resp = HttpResponse::build(StatusCode::OK)
                        .content_type("text/html; charset=utf-8")
                        .body(entries.join("\n"));
                    Ok(resp)
                }
                Err(err) => Err(err.into()),
            })
            .responder()
    }
}

pub fn movie_queue_transcode_directory(
    path: Path<(String, String)>,
    user: LoggedUser,
    request: HttpRequest<AppState>,
) -> FutureResponse<HttpResponse> {
    if user.email != "ddboline@gmail.com" {
        send_unauthorized(request)
    } else {
        let (directory, file) = path.into_inner();

        let patterns = vec![file];

        request
            .state()
            .db
            .send(MovieQueueRequest {
                patterns: patterns.clone(),
            })
            .from_err()
            .and_then(move |res| match res {
                Ok(entries) => {
                    let entries: Vec<Result<_, Error>> = entries
                        .iter()
                        .map(|entry| {
                            remcom_single_file(&entry.path, &Some(directory.clone()), false)?;
                            Ok(format!("{}", entry))
                        })
                        .collect();

                    let entries = map_result_vec(entries)?;

                    let resp = HttpResponse::build(StatusCode::OK)
                        .content_type("text/html; charset=utf-8")
                        .body(entries.join("\n"));
                    Ok(resp)
                }
                Err(err) => Err(err.into()),
            })
            .responder()
    }
}

pub fn movie_queue_play(
    idx: Path<i32>,
    user: LoggedUser,
    request: HttpRequest<AppState>,
) -> FutureResponse<HttpResponse> {
    if user.email != "ddboline@gmail.com" {
        send_unauthorized(request)
    } else {
        let idx = idx.into_inner();

        request
            .state()
            .db
            .send(MoviePathRequest { idx })
            .from_err()
            .and_then(move |res| match res {
                Ok(full_path) => {
                    let path = path::Path::new(&full_path);
                    let file_name = path.file_name().unwrap().to_str().unwrap();
                    let url = format!("/videos/partial/{}", file_name);

                    let body =
                        include_str!("../../templates/video_template.html").replace("VIDEO", &url);

                    let command = format!("rm -f /var/www/html/videos/partial/{}", file_name);
                    Exec::shell(&command).join().map_err(err_msg)?;
                    let command = format!(
                        "ln -s {} /var/www/html/videos/partial/{}",
                        full_path, file_name
                    );
                    Exec::shell(&command).join().map_err(err_msg)?;

                    let resp = HttpResponse::build(StatusCode::OK)
                        .content_type("text/html; charset=utf-8")
                        .body(body);
                    Ok(resp)
                }
                Err(err) => Err(err.into()),
            })
            .responder()
    }
}

pub fn trakt_watchlist(
    user: LoggedUser,
    request: HttpRequest<AppState>,
) -> FutureResponse<HttpResponse> {
    if user.email != "ddboline@gmail.com" {
        send_unauthorized(request)
    } else {
        request
            .state()
            .db
            .send(WatchlistShowsRequest {})
            .from_err()
            .and_then(move |res| match res {
                Ok(shows) => {
                    let mut shows: Vec<_> = shows
                        .into_iter()
                        .map(|(_, (_, s, source))| (s.title, s.link, source))
                        .collect();

                    shows.sort();

                    let body = include_str!("../../templates/watchlist_template.html")
                        .replace("PREVIOUS", "/list/tvshows");

                    let shows: Vec<_> = shows
                        .into_iter()
                        .map(|(title, link, source)| {
                            format!(
                                r#"<tr><td>{}</td>
                            <td><a href="https://www.imdb.com/title/{}">imdb</a> {} </tr>"#,
                                format!(
                                    r#"<a href="/list/trakt/watched/list/{}">{}</a>"#,
                                    link, title
                                ),
                                link,
                                match source.as_ref().map(|s| s.as_str()) {
                                    Some("netflix") => {
                                        r#"<td><a href="https://netflix.com">netflix</a>"#
                                    }
                                    Some("hulu") => r#"<td><a href="https://hulu.com">netflix</a>"#,
                                    Some("amazon") => {
                                        r#"<td><a href="https://amazon.com">netflix</a>"#
                                    }
                                    _ => "",
                                },
                            )
                        })
                        .collect();

                    let body = body.replace("BODY", &shows.join("\n"));

                    let resp = HttpResponse::build(StatusCode::OK)
                        .content_type("text/html; charset=utf-8")
                        .body(body);
                    Ok(resp)
                }
                Err(err) => Err(err.into()),
            })
            .responder()
    }
}

pub fn trakt_watchlist_action(
    path: Path<(String, String)>,
    user: LoggedUser,
    request: HttpRequest<AppState>,
) -> FutureResponse<HttpResponse> {
    if user.email != "ddboline@gmail.com" {
        send_unauthorized(request)
    } else {
        let (action, imdb_url) = path.into_inner();

        let action = TraktActions::from_command(&action);

        let ti = TraktConnection::new();

        request
            .state()
            .db
            .send(WatchlistActionRequest {
                action: action.clone(),
                imdb_url: imdb_url.clone(),
            })
            .from_err()
            .and_then(move |res| match res {
                Ok(_) => {
                    let body = match action {
                        TraktActions::Add => ti.add_watchlist_show(&imdb_url)?.to_string(),
                        TraktActions::Remove => ti.remove_watchlist_show(&imdb_url)?.to_string(),
                        _ => "".to_string(),
                    };
                    let resp = HttpResponse::build(StatusCode::OK)
                        .content_type("text/html; charset=utf-8")
                        .body(body);
                    Ok(resp)
                }
                Err(err) => Err(err.into()),
            })
            .responder()
    }
}

pub fn trakt_watched_seasons(
    path: Path<String>,
    user: LoggedUser,
    request: HttpRequest<AppState>,
) -> FutureResponse<HttpResponse> {
    if user.email != "ddboline@gmail.com" {
        send_unauthorized(request)
    } else {
        let imdb_url = path.into_inner();
        let button_add = r#"<td><button type="submit" id="ID" onclick="imdb_update('SHOW', 'LINK', SEASON);">update database</button></td>"#;
        let body = include_str!("../../templates/watchlist_template.html")
            .replace("PREVIOUS", "/list/trakt/watchlist");

        request
            .state()
            .db
            .send(ImdbRatingsRequest {
                imdb_url: imdb_url.clone(),
            })
            .map(move |show_opt| {
                let show = show_opt
                    .map(|s| s.map(|t| t.show.clone()).unwrap_or_else(|| "".to_string()))
                    .unwrap_or_else(|_| "".to_string());
                request
                    .state()
                    .db
                    .send(ImdbSeasonsRequest { show })
                    .from_err()
            })
            .flatten()
            .and_then(move |res| match res {
                Ok(entries) => {
                    let entries: Vec<_> = entries
                        .iter()
                        .map(|s| {
                            format!(
                                "<tr><td>{}<td>{}<td>{}<td>{}</tr>",
                                format!(
                                    r#"<a href="/list/trakt/watched/list/{}/{}">{}</t>"#,
                                    imdb_url, s.season, s.title
                                ),
                                s.season,
                                s.nepisodes,
                                button_add
                                    .replace("SHOW", &s.show)
                                    .replace("LINK", &imdb_url)
                                    .replace("SEASON", &s.season.to_string())
                            )
                        })
                        .collect();
                    let body = body.replace("BODY", &entries.join("\n"));

                    let resp = HttpResponse::build(StatusCode::OK)
                        .content_type("text/html; charset=utf-8")
                        .body(body);
                    Ok(resp)
                }
                Err(err) => Err(err.into()),
            })
            .responder()
    }
}

pub fn trakt_watched_list(
    path: Path<(String, i32)>,
    user: LoggedUser,
) -> Result<HttpResponse, Error> {
    if user.email != "ddboline@gmail.com" {
        return Ok(HttpResponse::Unauthorized().json("Unauthorized"));
    }

    let (imdb_url, season) = path.into_inner();

    let button_add = r#"<button type="submit" id="ID" onclick="watched_add('SHOW', SEASON, EPISODE);">add to watched</button>"#;
    let button_rm = r#"<button type="submit" id="ID" onclick="watched_rm('SHOW', SEASON, EPISODE);">remove from watched</button>"#;

    let body = include_str!("../../templates/watched_template.html").replace(
        "PREVIOUS",
        &format!("/list/trakt/watched/list/{}", imdb_url),
    );

    let mc = MovieCollectionDB::new();
    let mq = MovieQueueDB::with_pool(&mc.pool);

    let show = ImdbRatings::get_show_by_link(&imdb_url, &mc.pool)?
        .ok_or_else(|| err_msg("Show Doesn't exist"))?;

    let watched_episodes_db: HashSet<i32> =
        get_watched_shows_db(&mc.pool, &show.show, Some(season))?
            .into_iter()
            .map(|s| s.episode)
            .collect();

    let patterns = vec![show.show.clone()];

    let queue: HashMap<(String, i32, i32), _> = mq
        .print_movie_queue(&patterns)?
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

    let entries: Vec<_> = mc.print_imdb_episodes(&show.show, Some(season))?;

    let collection_idx_map: Vec<Result<_, Error>> = entries
        .iter()
        .filter_map(
            |r| match queue.get(&(show.show.clone(), season, r.episode)) {
                Some(row) => match mc.get_collection_index(&row.path) {
                    Ok(i) => match i {
                        Some(index) => Some(Ok((r.episode, index))),
                        None => None,
                    },
                    Err(e) => Some(Err(e)),
                },
                None => None,
            },
        )
        .collect();

    let collection_idx_map = map_result_vec(collection_idx_map)?;
    let collection_idx_map: HashMap<i32, i32> = collection_idx_map.into_iter().collect();

    let entries: Vec<_> = entries
        .iter()
        .map(|s| {
            let entry = if let Some(collection_idx) = collection_idx_map.get(&s.episode) {
                format!(
                    "<a href={}>{}</a>",
                    &format!(r#""{}/{}""#, "/list/play", collection_idx),
                    s.eptitle
                )
            } else {
                s.eptitle.clone()
            };

            format!(
                "<tr><td>{}</td><td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
                show.show,
                entry,
                format!(
                    r#"<a href="https://www.imdb.com/title/{}">s{} e{}</a>"#,
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
        .collect();

    let body = body.replace("BODY", &entries.join("\n"));

    let resp = HttpResponse::build(StatusCode::OK)
        .content_type("text/html; charset=utf-8")
        .body(body);
    Ok(resp)
}

pub fn trakt_watched_action(
    path: Path<(String, String, i32, i32)>,
    user: LoggedUser,
) -> Result<HttpResponse, Error> {
    if user.email != "ddboline@gmail.com" {
        return Ok(HttpResponse::Unauthorized().json("Unauthorized"));
    }

    let (action, imdb_url, season, episode) = path.into_inner();

    let ti = TraktConnection::new();
    let mc = MovieCollectionDB::new();

    let body = match action.as_str() {
        "add" => {
            let result = if season != -1 && episode != -1 {
                ti.add_episode_to_watched(&imdb_url, season, episode)?
            } else {
                ti.add_movie_to_watched(&imdb_url)?
            };
            if season != -1 && episode != -1 {
                WatchedEpisode {
                    imdb_url: imdb_url.clone(),
                    season,
                    episode,
                    ..Default::default()
                }
                .insert_episode(&mc.pool)?;
            } else {
                WatchedMovie {
                    imdb_url,
                    title: "".to_string(),
                }
                .insert_movie(&mc.pool)?;
            }

            format!("{}", result)
        }
        "rm" => {
            let result = if season != -1 && episode != -1 {
                ti.remove_episode_to_watched(&imdb_url, season, episode)?
            } else {
                ti.remove_movie_to_watched(&imdb_url)?
            };

            if season != -1 && episode != -1 {
                if let Some(epi_) =
                    WatchedEpisode::get_watched_episode(&mc.pool, &imdb_url, season, episode)?
                {
                    epi_.delete_episode(&mc.pool)?;
                }
            } else if let Some(movie) = WatchedMovie::get_watched_movie(&mc.pool, &imdb_url)? {
                movie.delete_movie(&mc.pool)?;
            };

            format!("{}", result)
        }
        _ => "".to_string(),
    };

    let resp = HttpResponse::build(StatusCode::OK)
        .content_type("text/html; charset=utf-8")
        .body(body);
    Ok(resp)
}

#[derive(Deserialize, Default)]
pub struct ParseImdbRequest {
    pub all: Option<bool>,
    pub database: Option<bool>,
    pub tv: Option<bool>,
    pub update: Option<bool>,
    pub link: Option<String>,
    pub season: Option<i32>,
}

pub fn imdb_show(
    path: Path<String>,
    query: Query<ParseImdbRequest>,
    user: LoggedUser,
) -> Result<HttpResponse, Error> {
    if user.email != "ddboline@gmail.com" {
        return Ok(HttpResponse::Unauthorized().json("Unauthorized"));
    }

    let mc = MovieCollectionDB::new();
    let watchlist = get_watchlist_shows_db_map(&mc.pool)?;

    let button_add = r#"<td><button type="submit" id="ID" onclick="watchlist_add('SHOW');">add to watchlist</button></td>"#;
    let button_rm = r#"<td><button type="submit" id="ID" onclick="watchlist_rm('SHOW');">remove from watchlist</button></td>"#;

    let show = path.into_inner();
    let query = query.into_inner();

    let output: Vec<_> = parse_imdb_worker(
        &mc,
        &show,
        query.tv.unwrap_or(false),
        query.link.clone(),
        query.all.unwrap_or(false),
        query.season,
        query.update.unwrap_or(false),
        query.database.unwrap_or(false),
    )?
    .into_iter()
    .map(|line| {
        let mut imdb_url = "".to_string();
        let tmp: Vec<_> = line
            .into_iter()
            .map(|i| {
                if i.starts_with("tt") {
                    imdb_url = i.clone();
                    format!(r#"<a href="https://www.imdb.com/title/{}">{}</a>"#, i, i)
                } else {
                    i.to_string()
                }
            })
            .collect();
        format!(
            "<tr><td>{}</td><td>{}</td></tr>",
            tmp.join("</td><td>"),
            if watchlist.contains_key(&imdb_url) {
                button_rm.replace("SHOW", &imdb_url)
            } else {
                button_add.replace("SHOW", &imdb_url)
            }
        )
    })
    .collect();

    let body = include_str!("../../templates/watchlist_template.html");

    let body = body.replace("BODY", &output.join("\n"));

    let resp = HttpResponse::build(StatusCode::OK)
        .content_type("text/html; charset=utf-8")
        .body(body);
    Ok(resp)
}

pub fn trakt_cal(user: LoggedUser) -> Result<HttpResponse, Error> {
    if user.email != "ddboline@gmail.com" {
        return Ok(HttpResponse::Unauthorized().json("Unauthorized"));
    }
    let body = "";
    let resp = HttpResponse::build(StatusCode::OK)
        .content_type("text/html; charset=utf-8")
        .body(body);
    Ok(resp)
}
