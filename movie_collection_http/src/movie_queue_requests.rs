use anyhow::{format_err, Error};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use stack_string::StackString;
use std::path;

use movie_collection_lib::{
    imdb_episodes::ImdbEpisodes,
    imdb_ratings::ImdbRatings,
    movie_collection::{
        find_new_episodes_http_worker, ImdbSeason, LastModifiedResponse, MovieCollection,
        MovieCollectionRow, TvShowsResult,
    },
    movie_queue::{MovieQueueDB, MovieQueueResult, MovieQueueRow},
    parse_imdb::{ParseImdb, ParseImdbOptions},
    pgpool::PgPool,
    trakt_utils::{
        get_watchlist_shows_db_map, trakt_cal_http_worker, watch_list_http_worker,
        watched_action_http_worker, TraktActions, WatchListMap, WatchListShow, TRAKT_CONN,
    },
    tv_show_source::TvShowSource,
};

#[derive(Clone, Copy)]
pub struct TvShowsRequest {}

impl TvShowsRequest {
    pub async fn handle(self, pool: &PgPool) -> Result<Vec<TvShowsResult>, Error> {
        MovieCollection::with_pool(pool)?.print_tv_shows().await
    }
}

#[derive(Clone, Copy)]
pub struct WatchlistShowsRequest {}

impl WatchlistShowsRequest {
    pub async fn handle(self, pool: &PgPool) -> Result<WatchListMap, Error> {
        get_watchlist_shows_db_map(pool).await
    }
}

pub struct QueueDeleteRequest {
    pub path: StackString,
}

impl QueueDeleteRequest {
    pub async fn handle(self, pool: &PgPool) -> Result<StackString, Error> {
        if path::Path::new(self.path.as_str()).exists() {
            MovieQueueDB::with_pool(pool)
                .remove_from_queue_by_path(&self.path)
                .await?;
        }
        Ok(self.path)
    }
}

#[derive(Debug)]
pub struct MovieQueueRequest {
    pub patterns: Vec<StackString>,
}

impl MovieQueueRequest {
    pub async fn handle(
        self,
        pool: &PgPool,
    ) -> Result<(Vec<MovieQueueResult>, Vec<StackString>), Error> {
        let patterns: Vec<_> = self.patterns.iter().map(StackString::as_str).collect();
        let queue = MovieQueueDB::with_pool(pool)
            .print_movie_queue(&patterns)
            .await?;
        Ok((queue, self.patterns))
    }
}

#[derive(Clone, Copy)]
pub struct MoviePathRequest {
    pub idx: i32,
}

impl MoviePathRequest {
    pub async fn handle(self, pool: &PgPool) -> Result<StackString, Error> {
        MovieCollection::with_pool(pool)?
            .get_collection_path(self.idx)
            .await
    }
}

pub struct ImdbRatingsRequest {
    pub imdb_url: StackString,
}

impl ImdbRatingsRequest {
    pub async fn handle(self, pool: &PgPool) -> Result<Option<(StackString, ImdbRatings)>, Error> {
        ImdbRatings::get_show_by_link(&self.imdb_url, pool)
            .await
            .map(|s| s.map(|sh| (self.imdb_url, sh)))
    }
}

pub struct ImdbSeasonsRequest {
    pub show: StackString,
}

impl ImdbSeasonsRequest {
    pub async fn handle(&self, pool: &PgPool) -> Result<Vec<ImdbSeason>, Error> {
        if &self.show == "" {
            Ok(Vec::new())
        } else {
            MovieCollection::with_pool(pool)?
                .print_imdb_all_seasons(&self.show)
                .await
        }
    }
}

pub struct WatchlistActionRequest {
    pub action: TraktActions,
    pub imdb_url: StackString,
}

impl WatchlistActionRequest {
    pub async fn handle(self, pool: &PgPool) -> Result<StackString, Error> {
        match &self.action {
            TraktActions::Add => {
                TRAKT_CONN.init().await;
                if let Some(show) = TRAKT_CONN.get_watchlist_shows().await?.get(&self.imdb_url) {
                    show.insert_show(pool).await?;
                }
            }
            TraktActions::Remove => {
                if let Some(show) = WatchListShow::get_show_by_link(&&self.imdb_url, pool).await? {
                    show.delete_show(pool).await?;
                }
            }
            _ => {}
        }
        Ok(self.imdb_url)
    }
}

pub struct WatchedListRequest {
    pub imdb_url: StackString,
    pub season: i32,
}

impl WatchedListRequest {
    pub async fn handle(&self, pool: &PgPool) -> Result<StackString, Error> {
        watch_list_http_worker(pool, &self.imdb_url, self.season).await
    }
}

pub struct WatchedActionRequest {
    pub action: TraktActions,
    pub imdb_url: StackString,
    pub season: i32,
    pub episode: i32,
}

impl WatchedActionRequest {
    pub async fn handle(&self, pool: &PgPool) -> Result<StackString, Error> {
        watched_action_http_worker(pool, self.action, &self.imdb_url, self.season, self.episode)
            .await
    }
}

#[derive(Deserialize, Default)]
pub struct ParseImdbRequest {
    pub all: Option<bool>,
    pub database: Option<bool>,
    pub tv: Option<bool>,
    pub update: Option<bool>,
    pub link: Option<StackString>,
    pub season: Option<i32>,
}

impl From<ParseImdbRequest> for ParseImdbOptions {
    fn from(opts: ParseImdbRequest) -> Self {
        Self {
            show: "".into(),
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
    pub show: StackString,
    pub query: ParseImdbRequest,
}

impl From<ImdbShowRequest> for ParseImdbOptions {
    fn from(opts: ImdbShowRequest) -> Self {
        Self {
            show: opts.show,
            ..opts.query.into()
        }
    }
}

impl ImdbShowRequest {
    pub async fn handle(self, pool: &PgPool) -> Result<StackString, Error> {
        let watchlist = get_watchlist_shows_db_map(pool).await?;
        let pi = ParseImdb::with_pool(pool)?;
        let body = pi.parse_imdb_http_worker(&self.into(), &watchlist).await?;
        Ok(body)
    }
}

#[derive(Clone, Copy)]
pub struct TraktCalRequest {}

impl TraktCalRequest {
    pub async fn handle(self, pool: &PgPool) -> Result<Vec<StackString>, Error> {
        trakt_cal_http_worker(pool).await
    }
}

#[derive(Serialize, Deserialize)]
pub struct FindNewEpisodeRequest {
    pub source: Option<TvShowSource>,
    pub shows: Option<StackString>,
}

impl FindNewEpisodeRequest {
    pub async fn handle(self, pool: &PgPool) -> Result<Vec<StackString>, Error> {
        find_new_episodes_http_worker(pool, self.shows, self.source).await
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy)]
pub struct ImdbEpisodesSyncRequest {
    pub start_timestamp: DateTime<Utc>,
}

impl ImdbEpisodesSyncRequest {
    pub async fn handle(self, pool: &PgPool) -> Result<Vec<ImdbEpisodes>, Error> {
        ImdbEpisodes::get_episodes_after_timestamp(self.start_timestamp, pool).await
    }
}

#[derive(Serialize, Deserialize, Clone, Copy)]
pub struct ImdbRatingsSyncRequest {
    pub start_timestamp: DateTime<Utc>,
}

impl ImdbRatingsSyncRequest {
    pub async fn handle(self, pool: &PgPool) -> Result<Vec<ImdbRatings>, Error> {
        ImdbRatings::get_shows_after_timestamp(self.start_timestamp, pool).await
    }
}

#[derive(Serialize, Deserialize, Clone, Copy)]
pub struct MovieQueueSyncRequest {
    pub start_timestamp: DateTime<Utc>,
}

impl MovieQueueSyncRequest {
    pub async fn handle(self, pool: &PgPool) -> Result<Vec<MovieQueueRow>, Error> {
        let mq = MovieQueueDB::with_pool(pool);
        mq.get_queue_after_timestamp(self.start_timestamp).await
    }
}

#[derive(Serialize, Deserialize, Clone, Copy)]
pub struct MovieCollectionSyncRequest {
    pub start_timestamp: DateTime<Utc>,
}

impl MovieCollectionSyncRequest {
    pub async fn handle(self, pool: &PgPool) -> Result<Vec<MovieCollectionRow>, Error> {
        let mc = MovieCollection::with_pool(pool)?;
        mc.get_collection_after_timestamp(self.start_timestamp)
            .await
    }
}

#[derive(Serialize, Deserialize)]
pub struct ImdbEpisodesUpdateRequest {
    pub episodes: Vec<ImdbEpisodes>,
}

impl ImdbEpisodesUpdateRequest {
    pub async fn handle(&self, pool: &PgPool) -> Result<(), Error> {
        for episode in &self.episodes {
            match episode.get_index(pool).await? {
                Some(_) => episode.update_episode(pool).await?,
                None => episode.insert_episode(pool).await?,
            }
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize)]
pub struct ImdbRatingsUpdateRequest {
    pub shows: Vec<ImdbRatings>,
}

impl ImdbRatingsUpdateRequest {
    pub async fn handle(&self, pool: &PgPool) -> Result<(), Error> {
        for show in &self.shows {
            match ImdbRatings::get_show_by_link(show.link.as_ref(), pool).await? {
                Some(_) => show.update_show(pool).await?,
                None => show.insert_show(pool).await?,
            }
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize)]
pub struct ImdbRatingsSetSourceRequest {
    pub link: StackString,
    pub source: TvShowSource,
}

impl ImdbRatingsSetSourceRequest {
    pub async fn handle(&self, pool: &PgPool) -> Result<(), Error> {
        let mut imdb = ImdbRatings::get_show_by_link(self.link.as_ref(), pool)
            .await?
            .ok_or_else(|| format_err!("No show found for {}", self.link))?;
        imdb.source = if self.source == TvShowSource::All {
            None
        } else {
            Some(self.source)
        };
        imdb.update_show(pool).await?;
        Ok(())
    }
}

#[derive(Serialize, Deserialize)]
pub struct MovieQueueUpdateRequest {
    pub queue: Vec<MovieQueueRow>,
}

impl MovieQueueUpdateRequest {
    pub async fn handle(&self, pool: &PgPool) -> Result<(), Error> {
        let mq = MovieQueueDB::with_pool(pool);
        let mc = MovieCollection::with_pool(pool)?;
        for entry in &self.queue {
            let cidx = if let Some(i) = mc.get_collection_index(entry.path.as_ref()).await? {
                i
            } else {
                mc.insert_into_collection_by_idx(entry.collection_idx, entry.path.as_ref())
                    .await?;
                entry.collection_idx
            };
            assert_eq!(cidx, entry.collection_idx);
            mq.insert_into_queue_by_collection_idx(entry.idx, entry.collection_idx)
                .await?;
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize)]
pub struct MovieCollectionUpdateRequest {
    pub collection: Vec<MovieCollectionRow>,
}

impl MovieCollectionUpdateRequest {
    pub async fn handle(&self, pool: &PgPool) -> Result<(), Error> {
        let mc = MovieCollection::with_pool(pool)?;
        for entry in &self.collection {
            if let Some(cidx) = mc.get_collection_index(entry.path.as_ref()).await? {
                if cidx == entry.idx {
                    continue;
                }
                mc.remove_from_collection(entry.path.as_ref()).await?;
            };
            mc.insert_into_collection_by_idx(entry.idx, entry.path.as_ref())
                .await?;
        }
        Ok(())
    }
}

#[derive(Clone, Copy)]
pub struct LastModifiedRequest {}

impl LastModifiedRequest {
    pub async fn handle(self, pool: &PgPool) -> Result<Vec<LastModifiedResponse>, Error> {
        LastModifiedResponse::get_last_modified(pool).await
    }
}
