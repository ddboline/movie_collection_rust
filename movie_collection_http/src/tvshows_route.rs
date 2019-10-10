#![allow(clippy::needless_pass_by_value)]

use actix_web::web::Data;
use actix_web::HttpResponse;
use failure::Error;
use futures::future::Future;
use std::collections::{HashMap, HashSet};

use super::logged_user::LoggedUser;
use super::movie_queue_app::AppState;
use super::movie_queue_requests::{TvShowsRequest, WatchlistShowsRequest};
use movie_collection_lib::common::movie_collection::TvShowsResult;
use movie_collection_lib::common::trakt_utils::WatchListShow;
use movie_collection_lib::common::tv_show_source::TvShowSource;

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

fn tvshows_worker(
    res1: Result<TvShowsMap, Error>,
    tvshows: Vec<TvShowsResult>,
) -> Result<HttpResponse, Error> {
    let tvshows: HashMap<String, _> = tvshows
        .into_iter()
        .map(|s| {
            let item: ProcessShowItem = s.into();
            (item.link.clone(), item)
        })
        .collect();
    let watchlist: HashMap<String, _> = res1.map(|w| {
        w.into_iter()
            .map(|(link, (show, s, source))| {
                let item = ProcessShowItem {
                    show,
                    title: s.title,
                    link: s.link,
                    source,
                };
                (link, item)
            })
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
    _: LoggedUser,
    state: Data<AppState>,
) -> impl Future<Item = HttpResponse, Error = Error> {
    state
        .db
        .send(TvShowsRequest {})
        .from_err()
        .join(state.db.send(WatchlistShowsRequest {}).from_err())
        .and_then(move |(res0, res1)| {
            res0.and_then(|tvshows| {
                tvshows_worker(res1, tvshows)
            })
        })
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
                    format!(r#"<a href="/list/{}">{}</a>"#, item.show, item.title)
                } else {
                    format!(
                        r#"<a href="/list/trakt/watched/list/{}">{}</a>"#,
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
                    format!(r#"<a href="/list/trakt/watched/list/{}">watchlist</a>"#, item.link)
                } else {
                    "".to_string()
                },
                if !has_watchlist {
                    button_add.replace("SHOW", &item.link)
                } else {
                    button_rm.replace("SHOW", &item.link)
                },
            )
        })
        .collect();
    Ok(shows)
}
