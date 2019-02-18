use actix::{Handler, Message};
use failure::{err_msg, Error};
use std::collections::{HashMap, HashSet};
use std::path;

use crate::common::imdb_episodes::ImdbEpisodes;
use crate::common::imdb_ratings::ImdbRatings;
use crate::common::movie_collection::{
    ImdbSeason, MovieCollection, MovieCollectionDB, TvShowsResult,
};
use crate::common::movie_queue::{MovieQueueDB, MovieQueueResult};
use crate::common::parse_imdb::{ParseImdb, ParseImdbOptions};
use crate::common::pgpool::PgPool;
use crate::common::trakt_utils::{
    get_watched_shows_db, get_watchlist_shows_db_map, TraktActions, TraktConnection, WatchListMap,
    WatchListShow, WatchedEpisode, WatchedMovie,
};
use crate::common::utils::map_result_vec;

pub struct TvShowsRequest {}

impl Message for TvShowsRequest {
    type Result = Result<Vec<TvShowsResult>, Error>;
}

impl Handler<TvShowsRequest> for PgPool {
    type Result = Result<Vec<TvShowsResult>, Error>;

    fn handle(&mut self, _: TvShowsRequest, _: &mut Self::Context) -> Self::Result {
        MovieCollectionDB::with_pool(&self).print_tv_shows()
    }
}

pub struct WatchlistShowsRequest {}

impl Message for WatchlistShowsRequest {
    type Result = Result<WatchListMap, Error>;
}

impl Handler<WatchlistShowsRequest> for PgPool {
    type Result = Result<WatchListMap, Error>;

    fn handle(&mut self, _: WatchlistShowsRequest, _: &mut Self::Context) -> Self::Result {
        get_watchlist_shows_db_map(&self)
    }
}

pub struct QueueDeleteRequest {
    pub path: String,
}

impl Message for QueueDeleteRequest {
    type Result = Result<(), Error>;
}

impl Handler<QueueDeleteRequest> for PgPool {
    type Result = Result<(), Error>;
    fn handle(&mut self, msg: QueueDeleteRequest, _: &mut Self::Context) -> Self::Result {
        if path::Path::new(&msg.path).exists() {
            MovieQueueDB::with_pool(&self).remove_from_queue_by_path(&msg.path)
        } else {
            Ok(())
        }
    }
}

pub struct MovieQueueRequest {
    pub patterns: Vec<String>,
}

impl Message for MovieQueueRequest {
    type Result = Result<Vec<MovieQueueResult>, Error>;
}

impl Handler<MovieQueueRequest> for PgPool {
    type Result = Result<Vec<MovieQueueResult>, Error>;

    fn handle(&mut self, msg: MovieQueueRequest, _: &mut Self::Context) -> Self::Result {
        MovieQueueDB::with_pool(&self).print_movie_queue(&msg.patterns)
    }
}

pub struct MoviePathRequest {
    pub idx: i32,
}

impl Message for MoviePathRequest {
    type Result = Result<String, Error>;
}

impl Handler<MoviePathRequest> for PgPool {
    type Result = Result<String, Error>;

    fn handle(&mut self, msg: MoviePathRequest, _: &mut Self::Context) -> Self::Result {
        MovieCollectionDB::with_pool(&self).get_collection_path(msg.idx)
    }
}

pub struct ImdbRatingsRequest {
    pub imdb_url: String,
}

impl Message for ImdbRatingsRequest {
    type Result = Result<Option<ImdbRatings>, Error>;
}

impl Handler<ImdbRatingsRequest> for PgPool {
    type Result = Result<Option<ImdbRatings>, Error>;

    fn handle(&mut self, msg: ImdbRatingsRequest, _: &mut Self::Context) -> Self::Result {
        ImdbRatings::get_show_by_link(&msg.imdb_url, &self)
    }
}

pub struct ImdbSeasonsRequest {
    pub show: String,
}

impl Message for ImdbSeasonsRequest {
    type Result = Result<Vec<ImdbSeason>, Error>;
}

impl Handler<ImdbSeasonsRequest> for PgPool {
    type Result = Result<Vec<ImdbSeason>, Error>;

    fn handle(&mut self, msg: ImdbSeasonsRequest, _: &mut Self::Context) -> Self::Result {
        if msg.show.as_str() == "" {
            Ok(Vec::new())
        } else {
            MovieCollectionDB::with_pool(&self).print_imdb_all_seasons(&msg.show)
        }
    }
}

pub struct WatchlistActionRequest {
    pub action: TraktActions,
    pub imdb_url: String,
}

impl Message for WatchlistActionRequest {
    type Result = Result<(), Error>;
}

impl Handler<WatchlistActionRequest> for PgPool {
    type Result = Result<(), Error>;

    fn handle(&mut self, msg: WatchlistActionRequest, _: &mut Self::Context) -> Self::Result {
        let ti = TraktConnection::new();
        let mc = MovieCollectionDB::with_pool(&self);

        match msg.action {
            TraktActions::Add => {
                if let Some(show) = ti.get_watchlist_shows()?.get(&msg.imdb_url) {
                    show.insert_show(&mc.pool)?;
                }
                Ok(())
            }
            TraktActions::Remove => {
                if let Some(show) = WatchListShow::get_show_by_link(&msg.imdb_url, &mc.pool)? {
                    show.delete_show(&mc.pool)?;
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }
}

pub struct WatchedShowsRequest {
    pub show: String,
    pub season: i32,
}

impl Message for WatchedShowsRequest {
    type Result = Result<Vec<WatchedEpisode>, Error>;
}

impl Handler<WatchedShowsRequest> for PgPool {
    type Result = Result<Vec<WatchedEpisode>, Error>;

    fn handle(&mut self, msg: WatchedShowsRequest, _: &mut Self::Context) -> Self::Result {
        get_watched_shows_db(&self, &msg.show, Some(msg.season))
    }
}

pub struct ImdbEpisodesRequest {
    pub show: String,
    pub season: Option<i32>,
}

impl Message for ImdbEpisodesRequest {
    type Result = Result<Vec<ImdbEpisodes>, Error>;
}

impl Handler<ImdbEpisodesRequest> for PgPool {
    type Result = Result<Vec<ImdbEpisodes>, Error>;

    fn handle(&mut self, msg: ImdbEpisodesRequest, _: &mut Self::Context) -> Self::Result {
        MovieCollectionDB::with_pool(&self).print_imdb_episodes(&msg.show, msg.season)
    }
}

pub struct WatchedListRequest {
    pub imdb_url: String,
    pub season: i32,
}

impl Message for WatchedListRequest {
    type Result = Result<String, Error>;
}

impl Handler<WatchedListRequest> for PgPool {
    type Result = Result<String, Error>;

    fn handle(&mut self, msg: WatchedListRequest, _: &mut Self::Context) -> Self::Result {
        let button_add = r#"<button type="submit" id="ID" onclick="watched_add('SHOW', SEASON, EPISODE);">add to watched</button>"#;
        let button_rm = r#"<button type="submit" id="ID" onclick="watched_rm('SHOW', SEASON, EPISODE);">remove from watched</button>"#;

        let body = include_str!("../../templates/watched_template.html").replace(
            "PREVIOUS",
            &format!("/list/trakt/watched/list/{}", msg.imdb_url),
        );

        let mc = MovieCollectionDB::with_pool(&self);
        let mq = MovieQueueDB::with_pool(&self);

        let show = ImdbRatings::get_show_by_link(&msg.imdb_url, &self)?
            .ok_or_else(|| err_msg("Show Doesn't exist"))?;

        let body = body
            .replace("SHOW", &show.show)
            .replace("LINK", &show.link)
            .replace("SEASON", &msg.season.to_string());

        let watched_episodes_db: HashSet<i32> =
            get_watched_shows_db(&self, &show.show, Some(msg.season))?
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

        let entries: Vec<_> = mc.print_imdb_episodes(&show.show, Some(msg.season))?;

        let collection_idx_map: Vec<Result<_, Error>> = entries
            .iter()
            .filter_map(
                |r| match queue.get(&(show.show.clone(), msg.season, r.episode)) {
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
                        s.epurl, msg.season, s.episode,
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
                            .replace("SEASON", &msg.season.to_string())
                            .replace("EPISODE", &s.episode.to_string())
                    } else {
                        button_add
                            .replace("SHOW", &show.link)
                            .replace("SEASON", &msg.season.to_string())
                            .replace("EPISODE", &s.episode.to_string())
                    }
                )
            })
            .collect();

        let body = body.replace("BODY", &entries.join("\n"));
        Ok(body)
    }
}

pub struct WatchedActionRequest {
    pub action: TraktActions,
    pub imdb_url: String,
    pub season: i32,
    pub episode: i32,
}

impl Message for WatchedActionRequest {
    type Result = Result<String, Error>;
}

impl Handler<WatchedActionRequest> for PgPool {
    type Result = Result<String, Error>;

    fn handle(&mut self, msg: WatchedActionRequest, _: &mut Self::Context) -> Self::Result {
        let ti = TraktConnection::new();
        let mc = MovieCollectionDB::with_pool(&self);

        let body = match msg.action {
            TraktActions::Add => {
                let result = if msg.season != -1 && msg.episode != -1 {
                    ti.add_episode_to_watched(&msg.imdb_url, msg.season, msg.episode)?
                } else {
                    ti.add_movie_to_watched(&msg.imdb_url)?
                };
                if msg.season != -1 && msg.episode != -1 {
                    WatchedEpisode {
                        imdb_url: msg.imdb_url,
                        season: msg.season,
                        episode: msg.episode,
                        ..Default::default()
                    }
                    .insert_episode(&mc.pool)?;
                } else {
                    WatchedMovie {
                        imdb_url: msg.imdb_url,
                        title: "".to_string(),
                    }
                    .insert_movie(&mc.pool)?;
                }

                format!("{}", result)
            }
            TraktActions::Remove => {
                let result = if msg.season != -1 && msg.episode != -1 {
                    ti.remove_episode_to_watched(&msg.imdb_url, msg.season, msg.episode)?
                } else {
                    ti.remove_movie_to_watched(&msg.imdb_url)?
                };

                if msg.season != -1 && msg.episode != -1 {
                    if let Some(epi_) = WatchedEpisode::get_watched_episode(
                        &mc.pool,
                        &msg.imdb_url,
                        msg.season,
                        msg.episode,
                    )? {
                        epi_.delete_episode(&mc.pool)?;
                    }
                } else if let Some(movie) =
                    WatchedMovie::get_watched_movie(&mc.pool, &msg.imdb_url)?
                {
                    movie.delete_movie(&mc.pool)?;
                };

                format!("{}", result)
            }
            _ => "".to_string(),
        };
        Ok(body)
    }
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

impl From<ParseImdbRequest> for ParseImdbOptions {
    fn from(opts: ParseImdbRequest) -> Self {
        ParseImdbOptions {
            show: "".to_string(),
            tv: opts.tv.unwrap_or(false),
            imdb_link: opts.link,
            all_seasons: opts.all.unwrap_or(false),
            season: opts.season,
            do_update: opts.update.unwrap_or(false),
            update_database: opts.database.unwrap_or(false),
        }
    }
}

pub struct ImdbShowRequest {
    pub show: String,
    pub query: ParseImdbRequest,
}

impl From<ImdbShowRequest> for ParseImdbOptions {
    fn from(opts: ImdbShowRequest) -> Self {
        ParseImdbOptions {
            show: opts.show,
            ..opts.query.into()
        }
    }
}

impl Message for ImdbShowRequest {
    type Result = Result<String, Error>;
}

impl Handler<ImdbShowRequest> for PgPool {
    type Result = Result<String, Error>;

    fn handle(&mut self, msg: ImdbShowRequest, _: &mut Self::Context) -> Self::Result {
        let button_add = r#"<td><button type="submit" id="ID" onclick="watchlist_add('SHOW');">add to watchlist</button></td>"#;
        let button_rm = r#"<td><button type="submit" id="ID" onclick="watchlist_rm('SHOW');">remove from watchlist</button></td>"#;

        let pi = ParseImdb::with_pool(&self);

        let watchlist = get_watchlist_shows_db_map(&self)?;

        let output: Vec<_> = pi
            .parse_imdb_worker(&msg.into())?
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
        Ok(body)
    }
}

pub struct TraktCalRequest {}

impl Message for TraktCalRequest {
    type Result = Result<Vec<String>, Error>;
}

impl Handler<TraktCalRequest> for PgPool {
    type Result = Result<Vec<String>, Error>;

    fn handle(&mut self, _: TraktCalRequest, _: &mut Self::Context) -> Self::Result {
        let button_add = r#"<td><button type="submit" id="ID" onclick="imdb_update('SHOW', 'LINK', SEASON);">update database</button></td>"#;
        let cal_list = TraktConnection::new().get_calendar()?;
        let entries: Vec<Result<_, Error>> = cal_list
            .into_iter()
            .map(|cal| {
                let show = match ImdbRatings::get_show_by_link(&cal.link, &self)? {
                    Some(s) => s.show,
                    None => "".to_string(),
                };
                let exists = if !show.is_empty() {
                    ImdbEpisodes {
                        show: show.clone(),
                        season: cal.season,
                        episode: cal.episode,
                        ..Default::default()
                    }
                    .get_index(&self)?
                    .is_some()
                } else {
                    false
                };

                let entry = format!(
                    "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td>{}</tr>",
                    format!(
                        r#"<a href="/list/trakt/watched/list/{}/{}>{}</a>"#,
                        cal.link, cal.season, cal.show,
                    ),
                    format!(
                        r#"<a href="https://www.imdb.com/title/{}">imdb</a>"#,
                        cal.link
                    ),
                    match cal.ep_link {
                        Some(link) => format!(
                            r#"<a href="https://www.imdb.com/title/{}">{} {}</a>"#,
                            link, cal.season, cal.episode,
                        ),
                        None => format!("{} {}", cal.season, cal.episode,),
                    },
                    cal.airdate,
                    if !exists {
                        button_add
                            .replace("SHOW", &show)
                            .replace("LINK", &cal.link)
                            .replace("SEASON", &cal.season.to_string())
                    } else {
                        "".to_string()
                    },
                );
                Ok(entry)
            })
            .collect();
        let entries = map_result_vec(entries)?;
        Ok(entries)
    }
}
