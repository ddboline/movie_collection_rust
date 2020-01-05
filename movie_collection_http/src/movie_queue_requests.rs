use chrono::{DateTime, Utc};
use failure::Error;
use serde::{Deserialize, Serialize};
use std::path;

use super::logged_user::LoggedUser;
use super::HandleRequest;
use movie_collection_lib::common::imdb_episodes::ImdbEpisodes;
use movie_collection_lib::common::imdb_ratings::ImdbRatings;
use movie_collection_lib::common::movie_collection::{
    find_new_episodes_http_worker, ImdbSeason, MovieCollection, MovieCollectionRow, TvShowsResult,
};
use movie_collection_lib::common::movie_queue::{MovieQueueDB, MovieQueueResult, MovieQueueRow};
use movie_collection_lib::common::parse_imdb::{ParseImdb, ParseImdbOptions};
use movie_collection_lib::common::pgpool::PgPool;
use movie_collection_lib::common::trakt_instance::TraktInstance;
use movie_collection_lib::common::trakt_utils::{
    get_watched_shows_db, get_watchlist_shows_db_map, trakt_cal_http_worker,
    watch_list_http_worker, watched_action_http_worker, TraktActions, WatchListMap, WatchListShow,
    WatchedEpisode,
};
use movie_collection_lib::common::tv_show_source::TvShowSource;

pub struct TvShowsRequest {}

impl HandleRequest<TvShowsRequest> for PgPool {
    type Result = Result<Vec<TvShowsResult>, Error>;

    fn handle(&self, _: TvShowsRequest) -> Self::Result {
        MovieCollection::with_pool(&self)?.print_tv_shows()
    }
}

pub struct WatchlistShowsRequest {}

impl HandleRequest<WatchlistShowsRequest> for PgPool {
    type Result = Result<WatchListMap, Error>;

    fn handle(&self, _: WatchlistShowsRequest) -> Self::Result {
        get_watchlist_shows_db_map(&self)
    }
}

pub struct QueueDeleteRequest {
    pub path: String,
}

impl HandleRequest<QueueDeleteRequest> for PgPool {
    type Result = Result<String, Error>;
    fn handle(&self, msg: QueueDeleteRequest) -> Self::Result {
        if path::Path::new(&msg.path).exists() {
            MovieQueueDB::with_pool(&self).remove_from_queue_by_path(&msg.path)?;
        }
        Ok(msg.path)
    }
}

pub struct MovieQueueRequest {
    pub patterns: Vec<String>,
}

impl HandleRequest<MovieQueueRequest> for PgPool {
    type Result = Result<(Vec<MovieQueueResult>, Vec<String>), Error>;

    fn handle(&self, msg: MovieQueueRequest) -> Self::Result {
        let patterns: Vec<_> = msg.patterns.iter().map(|s| s.as_str()).collect();
        let queue = MovieQueueDB::with_pool(&self).print_movie_queue(&patterns)?;
        Ok((queue, msg.patterns))
    }
}

pub struct MoviePathRequest {
    pub idx: i32,
}

impl HandleRequest<MoviePathRequest> for PgPool {
    type Result = Result<String, Error>;

    fn handle(&self, msg: MoviePathRequest) -> Self::Result {
        MovieCollection::with_pool(&self)?.get_collection_path(msg.idx)
    }
}

pub struct ImdbRatingsRequest {
    pub imdb_url: String,
}

impl HandleRequest<ImdbRatingsRequest> for PgPool {
    type Result = Result<Option<(String, ImdbRatings)>, Error>;

    fn handle(&self, msg: ImdbRatingsRequest) -> Self::Result {
        ImdbRatings::get_show_by_link(&msg.imdb_url, &self).map(|s| s.map(|sh| (msg.imdb_url, sh)))
    }
}

pub struct ImdbSeasonsRequest {
    pub show: String,
}

impl HandleRequest<ImdbSeasonsRequest> for PgPool {
    type Result = Result<Vec<ImdbSeason>, Error>;

    fn handle(&self, msg: ImdbSeasonsRequest) -> Self::Result {
        if msg.show.as_str() == "" {
            Ok(Vec::new())
        } else {
            MovieCollection::with_pool(&self)?.print_imdb_all_seasons(&msg.show)
        }
    }
}

pub struct WatchlistActionRequest {
    pub action: TraktActions,
    pub imdb_url: String,
}

impl HandleRequest<WatchlistActionRequest> for PgPool {
    type Result = Result<String, Error>;

    fn handle(&self, msg: WatchlistActionRequest) -> Self::Result {
        let ti = TraktInstance::new();

        match msg.action {
            TraktActions::Add => {
                if let Some(show) = ti.get_watchlist_shows()?.get(&msg.imdb_url) {
                    show.insert_show(&self)?;
                }
            }
            TraktActions::Remove => {
                if let Some(show) = WatchListShow::get_show_by_link(&msg.imdb_url, &self)? {
                    show.delete_show(&self)?;
                }
            }
            _ => {}
        }
        Ok(msg.imdb_url)
    }
}

pub struct WatchedShowsRequest {
    pub show: String,
    pub season: i32,
}

impl HandleRequest<WatchedShowsRequest> for PgPool {
    type Result = Result<Vec<WatchedEpisode>, Error>;

    fn handle(&self, msg: WatchedShowsRequest) -> Self::Result {
        get_watched_shows_db(&self, &msg.show, Some(msg.season))
    }
}

pub struct ImdbEpisodesRequest {
    pub show: String,
    pub season: Option<i32>,
}

impl HandleRequest<ImdbEpisodesRequest> for PgPool {
    type Result = Result<Vec<ImdbEpisodes>, Error>;

    fn handle(&self, msg: ImdbEpisodesRequest) -> Self::Result {
        MovieCollection::with_pool(&self)?.print_imdb_episodes(&msg.show, msg.season)
    }
}

pub struct WatchedListRequest {
    pub imdb_url: String,
    pub season: i32,
}

impl HandleRequest<WatchedListRequest> for PgPool {
    type Result = Result<String, Error>;

    fn handle(&self, msg: WatchedListRequest) -> Self::Result {
        watch_list_http_worker(&self, &msg.imdb_url, msg.season)
    }
}

pub struct WatchedActionRequest {
    pub action: TraktActions,
    pub imdb_url: String,
    pub season: i32,
    pub episode: i32,
}

impl HandleRequest<WatchedActionRequest> for PgPool {
    type Result = Result<String, Error>;

    fn handle(&self, msg: WatchedActionRequest) -> Self::Result {
        watched_action_http_worker(&self, msg.action, &msg.imdb_url, msg.season, msg.episode)
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

impl HandleRequest<ImdbShowRequest> for PgPool {
    type Result = Result<String, Error>;

    fn handle(&self, msg: ImdbShowRequest) -> Self::Result {
        let watchlist = get_watchlist_shows_db_map(&self)?;
        let pi = ParseImdb::with_pool(&self)?;
        let body = pi.parse_imdb_http_worker(&msg.into(), &watchlist)?;
        Ok(body)
    }
}

pub struct TraktCalRequest {}

impl HandleRequest<TraktCalRequest> for PgPool {
    type Result = Result<Vec<String>, Error>;

    fn handle(&self, _: TraktCalRequest) -> Self::Result {
        trakt_cal_http_worker(&self)
    }
}

#[derive(Serialize, Deserialize)]
pub struct FindNewEpisodeRequest {
    pub source: Option<TvShowSource>,
    pub shows: Option<String>,
}

impl HandleRequest<FindNewEpisodeRequest> for PgPool {
    type Result = Result<Vec<String>, Error>;

    fn handle(&self, msg: FindNewEpisodeRequest) -> Self::Result {
        find_new_episodes_http_worker(&self, msg.shows, msg.source)
    }
}

pub struct AuthorizedUserRequest {
    pub user: LoggedUser,
}

impl HandleRequest<AuthorizedUserRequest> for PgPool {
    type Result = Result<bool, Error>;
    fn handle(&self, msg: AuthorizedUserRequest) -> Self::Result {
        msg.user.is_authorized(self)
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ImdbEpisodesSyncRequest {
    pub start_timestamp: DateTime<Utc>,
}

impl HandleRequest<ImdbEpisodesSyncRequest> for PgPool {
    type Result = Result<Vec<ImdbEpisodes>, Error>;

    fn handle(&self, msg: ImdbEpisodesSyncRequest) -> Self::Result {
        ImdbEpisodes::get_episodes_after_timestamp(msg.start_timestamp, &self)
    }
}

#[derive(Serialize, Deserialize)]
pub struct ImdbRatingsSyncRequest {
    pub start_timestamp: DateTime<Utc>,
}

impl HandleRequest<ImdbRatingsSyncRequest> for PgPool {
    type Result = Result<Vec<ImdbRatings>, Error>;

    fn handle(&self, msg: ImdbRatingsSyncRequest) -> Self::Result {
        ImdbRatings::get_shows_after_timestamp(msg.start_timestamp, &self)
    }
}

#[derive(Serialize, Deserialize)]
pub struct MovieQueueSyncRequest {
    pub start_timestamp: DateTime<Utc>,
}

impl HandleRequest<MovieQueueSyncRequest> for PgPool {
    type Result = Result<Vec<MovieQueueRow>, Error>;

    fn handle(&self, msg: MovieQueueSyncRequest) -> Self::Result {
        let mq = MovieQueueDB::with_pool(&self);
        mq.get_queue_after_timestamp(msg.start_timestamp)
    }
}

#[derive(Serialize, Deserialize)]
pub struct MovieCollectionSyncRequest {
    pub start_timestamp: DateTime<Utc>,
}

impl HandleRequest<MovieCollectionSyncRequest> for PgPool {
    type Result = Result<Vec<MovieCollectionRow>, Error>;

    fn handle(&self, msg: MovieCollectionSyncRequest) -> Self::Result {
        let mc = MovieCollection::with_pool(&self)?;
        mc.get_collection_after_timestamp(msg.start_timestamp)
    }
}

#[derive(Serialize, Deserialize)]
pub struct ImdbEpisodesUpdateRequest {
    pub episodes: Vec<ImdbEpisodes>,
}

impl HandleRequest<ImdbEpisodesUpdateRequest> for PgPool {
    type Result = Result<(), Error>;

    fn handle(&self, msg: ImdbEpisodesUpdateRequest) -> Self::Result {
        for episode in msg.episodes {
            match episode.get_index(&self)? {
                Some(_) => episode.update_episode(&self)?,
                None => episode.insert_episode(&self)?,
            }
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize)]
pub struct ImdbRatingsUpdateRequest {
    pub shows: Vec<ImdbRatings>,
}

impl HandleRequest<ImdbRatingsUpdateRequest> for PgPool {
    type Result = Result<(), Error>;

    fn handle(&self, msg: ImdbRatingsUpdateRequest) -> Self::Result {
        for show in msg.shows {
            match ImdbRatings::get_show_by_link(&show.link, &self)? {
                Some(_) => show.update_show(&self)?,
                None => show.insert_show(&self)?,
            }
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize)]
pub struct MovieQueueUpdateRequest {
    pub queue: Vec<MovieQueueRow>,
}

impl HandleRequest<MovieQueueUpdateRequest> for PgPool {
    type Result = Result<(), Error>;

    fn handle(&self, msg: MovieQueueUpdateRequest) -> Self::Result {
        let mq = MovieQueueDB::with_pool(&self);
        let mc = MovieCollection::with_pool(&self)?;
        for entry in msg.queue {
            let cidx = match mc.get_collection_index(&entry.path)? {
                Some(i) => i,
                None => {
                    mc.insert_into_collection_by_idx(entry.collection_idx, &entry.path)?;
                    entry.collection_idx
                }
            };
            assert_eq!(cidx, entry.collection_idx);
            mq.insert_into_queue_by_collection_idx(entry.idx, entry.collection_idx)?;
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize)]
pub struct MovieCollectionUpdateRequest {
    pub collection: Vec<MovieCollectionRow>,
}

impl HandleRequest<MovieCollectionUpdateRequest> for PgPool {
    type Result = Result<(), Error>;

    fn handle(&self, msg: MovieCollectionUpdateRequest) -> Self::Result {
        let mc = MovieCollection::with_pool(&self)?;
        for entry in msg.collection {
            if let Some(cidx) = mc.get_collection_index(&entry.path)? {
                if cidx == entry.idx {
                    continue;
                }
                mc.remove_from_collection(&entry.path)?;
            };
            mc.insert_into_collection_by_idx(entry.idx, &entry.path)?;
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize)]
pub struct LastModifiedResponse {
    pub table: String,
    pub last_modified: DateTime<Utc>,
}

pub struct LastModifiedRequest {}

impl HandleRequest<LastModifiedRequest> for PgPool {
    type Result = Result<Vec<LastModifiedResponse>, Error>;

    fn handle(&self, _: LastModifiedRequest) -> Self::Result {
        let tables = vec![
            "imdb_episodes",
            "imdb_ratings",
            "movie_collection",
            "movie_queue",
        ];

        tables
            .iter()
            .map(|table| {
                let query = format!("SELECT max(last_modified) FROM {}", table);
                let r = match self.get()?.query(query.as_str(), &[])?.get(0) {
                    Some(row) => {
                        let last_modified: DateTime<Utc> = row.try_get(0)?;
                        Some(LastModifiedResponse {
                            table: (*table).to_string(),
                            last_modified,
                        })
                    }
                    None => None,
                };

                Ok(r)
            })
            .filter_map(|x| x.transpose())
            .collect()
    }
}
