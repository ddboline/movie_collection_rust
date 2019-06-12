#![allow(clippy::needless_pass_by_value)]

use actix_web::web::Data;
use actix_web::HttpResponse;
use failure::Error;
use futures::future::Future;
use std::collections::HashMap;

use super::logged_user::LoggedUser;
use super::movie_queue_app::AppState;
use super::movie_queue_requests::{TvShowsRequest, WatchlistShowsRequest};
use movie_collection_lib::common::movie_collection::TvShowsResult;
use movie_collection_lib::common::trakt_utils::WatchListShow;
use movie_collection_lib::common::tv_show_source::TvShowSource;

type TvShowsMap = HashMap<String, (String, WatchListShow, Option<TvShowSource>)>;

fn tvshows_worker(
    res1: Result<TvShowsMap, Error>,
    tvshows: Vec<TvShowsResult>,
) -> Result<HttpResponse, Error> {
    let tvshows: HashMap<String, _> = tvshows
        .into_iter()
        .map(|s| (s.link.clone(), (s.show, s.title, s.link, s.source)))
        .collect();
    let watchlist: HashMap<String, _> = res1.map(|w| {
        w.into_iter()
            .map(|(link, (show, s, source))| (link, (show, s.title, s.link, source)))
            .collect()
    })?;

    let shows = process_shows(tvshows, watchlist)?;

    let body =
        include_str!("../../templates/tvshows_template.html").replace("BODY", &shows.join("\n"));

    let resp = HttpResponse::Ok()
        .content_type("text/html; charset=utf-8")
        .body(body);
    Ok(resp)
}

pub fn tvshows(
    user: LoggedUser,
    state: Data<AppState>,
) -> impl Future<Item = HttpResponse, Error = Error> {
    state
        .db
        .send(TvShowsRequest {})
        .from_err()
        .join(state.db.send(WatchlistShowsRequest {}).from_err())
        .and_then(move |(res0, res1)| match res0 {
            Ok(tvshows) => {
                if !state.user_list.is_authorized(&user) {
                    return Ok(HttpResponse::Unauthorized().json("Unauthorized"));
                }
                tvshows_worker(res1, tvshows)
            }
            Err(err) => Err(err.into()),
        })
}

fn process_shows(
    tvshows: HashMap<String, (String, String, String, Option<TvShowSource>)>,
    watchlist: HashMap<String, (String, String, String, Option<TvShowSource>)>,
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
                match source {
                    Some(TvShowSource::Netflix) => r#"<a href="https://netflix.com">netflix</a>"#,
                    Some(TvShowSource::Hulu) => r#"<a href="https://hulu.com">hulu</a>"#,
                    Some(TvShowSource::Amazon) => r#"<a href="https://amazon.com">amazon</a>"#,
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
