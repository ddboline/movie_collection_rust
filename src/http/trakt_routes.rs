#![allow(clippy::needless_pass_by_value)]

use actix_web::{
    http::StatusCode, AsyncResponder, FutureResponse, HttpRequest, HttpResponse, Path,
};
use futures::future::Future;
use std::collections::HashMap;

use super::logged_user::LoggedUser;
use super::movie_queue_app::AppState;
use super::movie_queue_requests::{
    ImdbRatingsRequest, ImdbSeasonsRequest, TraktCalRequest, WatchedActionRequest,
    WatchedListRequest, WatchlistActionRequest, WatchlistShowsRequest,
};
use super::{form_http_response, get_auth_fut, unauthbody};
use crate::common::movie_collection::ImdbSeason;
use crate::common::trakt_utils::{TraktActions, TraktConnection, WatchListShow};
use crate::common::tv_show_source::TvShowSource;

fn watchlist_worker(
    shows: HashMap<String, (String, WatchListShow, Option<TvShowSource>)>,
) -> Result<HttpResponse, actix_web::Error> {
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

    let body = body.replace("BODY", &shows.join("\n"));

    let resp = HttpResponse::build(StatusCode::OK)
        .content_type("text/html; charset=utf-8")
        .body(body);
    Ok(resp)
}

pub fn trakt_watchlist(
    user: LoggedUser,
    request: HttpRequest<AppState>,
) -> FutureResponse<HttpResponse> {
    let resp = request
        .state()
        .db
        .send(WatchlistShowsRequest {})
        .from_err()
        .and_then(move |res| match res {
            Ok(shows) => watchlist_worker(shows),
            Err(err) => Err(err.into()),
        })
        .responder();

    if request.state().user_list.is_authorized(&user) {
        resp
    } else {
        get_auth_fut(&user, &request)
            .and_then(move |res| match res {
                Ok(true) => resp,
                _ => unauthbody(),
            })
            .responder()
    }
}

fn watchlist_action_worker(
    action: TraktActions,
    imdb_url: &str,
) -> Result<HttpResponse, actix_web::Error> {
    let ti = TraktConnection::new();

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

pub fn trakt_watchlist_action(
    path: Path<(String, String)>,
    user: LoggedUser,
    request: HttpRequest<AppState>,
) -> FutureResponse<HttpResponse> {
    let (action, imdb_url) = path.into_inner();
    let action = TraktActions::from_command(&action);

    let resp = request
        .state()
        .db
        .send(WatchlistActionRequest {
            action: action.clone(),
            imdb_url: imdb_url.clone(),
        })
        .from_err()
        .and_then(move |res| match res {
            Ok(_) => watchlist_action_worker(action, &imdb_url),
            Err(err) => Err(err.into()),
        })
        .responder();

    if request.state().user_list.is_authorized(&user) {
        resp
    } else {
        get_auth_fut(&user, &request)
            .and_then(move |res| match res {
                Ok(true) => resp,
                _ => unauthbody(),
            })
            .responder()
    }
}

fn trakt_watched_seasons_worker(
    link: &str,
    imdb_url: &str,
    entries: &[ImdbSeason],
) -> Result<HttpResponse, actix_web::Error> {
    let button_add = r#"<td><button type="submit" id="ID" onclick="imdb_update('SHOW', 'LINK', SEASON);">update database</button></td>"#;
    let body = include_str!("../../templates/watchlist_template.html")
        .replace("PREVIOUS", "/list/trakt/watchlist");

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
                    .replace("LINK", &link)
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

pub fn trakt_watched_seasons(
    path: Path<String>,
    user: LoggedUser,
    request: HttpRequest<AppState>,
) -> FutureResponse<HttpResponse> {
    let imdb_url = path.into_inner();

    let request_clone = request.clone();

    let resp = request
        .state()
        .db
        .send(ImdbRatingsRequest {
            imdb_url: imdb_url.clone(),
        })
        .map(move |show_opt| {
            let empty = || ("".to_string(), "".to_string());
            let (show, link) = show_opt
                .map(|s| {
                    s.map(|t| (t.show.clone(), t.link.clone()))
                        .unwrap_or_else(empty)
                })
                .unwrap_or_else(|_| empty());
            request_clone
                .state()
                .db
                .send(ImdbSeasonsRequest { show })
                .from_err()
                .map(|res| (link, res))
        })
        .flatten()
        .and_then(move |(link, res)| match res {
            Ok(entries) => trakt_watched_seasons_worker(&link, &imdb_url, &entries),
            Err(err) => Err(err.into()),
        })
        .responder();

    if request.state().user_list.is_authorized(&user) {
        resp
    } else {
        get_auth_fut(&user, &request)
            .and_then(move |res| match res {
                Ok(true) => resp,
                _ => unauthbody(),
            })
            .responder()
    }
}

pub fn trakt_watched_list(
    path: Path<(String, i32)>,
    user: LoggedUser,
    request: HttpRequest<AppState>,
) -> FutureResponse<HttpResponse> {
    let (imdb_url, season) = path.into_inner();

    let resp = request
        .state()
        .db
        .send(WatchedListRequest { imdb_url, season })
        .from_err()
        .and_then(move |res| match res {
            Ok(body) => Ok(form_http_response(body)),
            Err(err) => Err(err.into()),
        })
        .responder();

    if request.state().user_list.is_authorized(&user) {
        resp
    } else {
        get_auth_fut(&user, &request)
            .and_then(move |res| match res {
                Ok(true) => resp,
                _ => unauthbody(),
            })
            .responder()
    }
}

pub fn trakt_watched_action(
    path: Path<(String, String, i32, i32)>,
    user: LoggedUser,
    request: HttpRequest<AppState>,
) -> FutureResponse<HttpResponse> {
    let (action, imdb_url, season, episode) = path.into_inner();

    let resp = request
        .state()
        .db
        .send(WatchedActionRequest {
            action: TraktActions::from_command(&action),
            imdb_url,
            season,
            episode,
        })
        .from_err()
        .and_then(move |res| match res {
            Ok(body) => Ok(form_http_response(body)),
            Err(err) => Err(err.into()),
        })
        .responder();

    if request.state().user_list.is_authorized(&user) {
        resp
    } else {
        get_auth_fut(&user, &request)
            .and_then(move |res| match res {
                Ok(true) => resp,
                _ => unauthbody(),
            })
            .responder()
    }
}

fn trakt_cal_worker(entries: &[String]) -> Result<HttpResponse, actix_web::Error> {
    let body =
        include_str!("../../templates/watched_template.html").replace("PREVIOUS", "/list/tvshows");
    let body = body.replace("BODY", &entries.join("\n"));
    let resp = HttpResponse::build(StatusCode::OK)
        .content_type("text/html; charset=utf-8")
        .body(body);
    Ok(resp)
}

pub fn trakt_cal(user: LoggedUser, request: HttpRequest<AppState>) -> FutureResponse<HttpResponse> {
    let resp = request
        .state()
        .db
        .send(TraktCalRequest {})
        .from_err()
        .and_then(move |res| match res {
            Ok(entries) => trakt_cal_worker(&entries),
            Err(err) => Err(err.into()),
        })
        .responder();

    if request.state().user_list.is_authorized(&user) {
        resp
    } else {
        get_auth_fut(&user, &request)
            .and_then(move |res| match res {
                Ok(true) => resp,
                _ => unauthbody(),
            })
            .responder()
    }
}
