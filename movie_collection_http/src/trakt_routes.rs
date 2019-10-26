#![allow(clippy::needless_pass_by_value)]

use actix_web::web::{Data, Path};
use actix_web::HttpResponse;
use failure::Error;
use futures::Future;
use std::collections::HashMap;

use super::logged_user::LoggedUser;
use super::movie_queue_app::AppState;
use super::movie_queue_requests::{
    ImdbRatingsRequest, ImdbSeasonsRequest, TraktCalRequest, WatchedActionRequest,
    WatchedListRequest, WatchlistActionRequest, WatchlistShowsRequest,
};
use super::{form_http_response, generic_route};
use movie_collection_lib::common::movie_collection::ImdbSeason;
use movie_collection_lib::common::trakt_instance::TraktInstance;
use movie_collection_lib::common::trakt_utils::{TraktActions, WatchListShow};
use movie_collection_lib::common::tv_show_source::TvShowSource;

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

pub fn trakt_watchlist(
    _: LoggedUser,
    state: Data<AppState>,
) -> impl Future<Item = HttpResponse, Error = Error> {
    generic_route(WatchlistShowsRequest {}, state, watchlist_worker)
}

fn watchlist_action_worker(action: TraktActions, imdb_url: &str) -> Result<HttpResponse, Error> {
    let ti = TraktInstance::new();

    let body = match action {
        TraktActions::Add => ti.add_watchlist_show(&imdb_url)?.to_string(),
        TraktActions::Remove => ti.remove_watchlist_show(&imdb_url)?.to_string(),
        _ => "".to_string(),
    };
    let resp = HttpResponse::Ok()
        .content_type("text/html; charset=utf-8")
        .body(body);
    Ok(resp)
}

pub fn trakt_watchlist_action(
    path: Path<(String, String)>,
    _: LoggedUser,
    state: Data<AppState>,
) -> impl Future<Item = HttpResponse, Error = Error> {
    let (action, imdb_url) = path.into_inner();
    let action = TraktActions::from_command(&action);

    generic_route(
        WatchlistActionRequest { action, imdb_url },
        state,
        move |imdb_url| watchlist_action_worker(action, &imdb_url),
    )
}

fn trakt_watched_seasons_worker(
    link: &str,
    imdb_url: &str,
    entries: &[ImdbSeason],
) -> Result<HttpResponse, Error> {
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

    let resp = HttpResponse::Ok()
        .content_type("text/html; charset=utf-8")
        .body(entries);
    Ok(resp)
}

pub fn trakt_watched_seasons(
    path: Path<String>,
    _: LoggedUser,
    state: Data<AppState>,
) -> impl Future<Item = HttpResponse, Error = Error> {
    let imdb_url = path.into_inner();

    state
        .db
        .send(ImdbRatingsRequest { imdb_url })
        .map(move |show_opt| {
            let empty = || ("".to_string(), "".to_string(), "".to_string());
            let (imdb_url, show, link) = show_opt
                .map(|s| {
                    s.map(|(imdb_url, t)| (imdb_url, t.show, t.link))
                        .unwrap_or_else(empty)
                })
                .unwrap_or_else(|_| empty());
            state
                .db
                .send(ImdbSeasonsRequest { show })
                .from_err()
                .map(|res| (imdb_url, link, res))
        })
        .flatten()
        .and_then(|(imdb_url, link, res)| {
            res.and_then(|entries| trakt_watched_seasons_worker(&link, &imdb_url, &entries))
        })
}

pub fn trakt_watched_list(
    path: Path<(String, i32)>,
    _: LoggedUser,
    state: Data<AppState>,
) -> impl Future<Item = HttpResponse, Error = Error> {
    let (imdb_url, season) = path.into_inner();

    generic_route(
        WatchedListRequest { imdb_url, season },
        state,
        form_http_response,
    )
}

pub fn trakt_watched_action(
    path: Path<(String, String, i32, i32)>,
    _: LoggedUser,
    state: Data<AppState>,
) -> impl Future<Item = HttpResponse, Error = Error> {
    let (action, imdb_url, season, episode) = path.into_inner();

    generic_route(
        WatchedActionRequest {
            action: TraktActions::from_command(&action),
            imdb_url,
            season,
            episode,
        },
        state,
        form_http_response,
    )
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

pub fn trakt_cal(
    _: LoggedUser,
    state: Data<AppState>,
) -> impl Future<Item = HttpResponse, Error = Error> {
    generic_route(TraktCalRequest {}, state, |entries| {
        trakt_cal_worker(&entries)
    })
}
