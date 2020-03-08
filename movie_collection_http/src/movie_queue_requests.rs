use anyhow::Error;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use futures::future::try_join_all;
use serde::{Deserialize, Serialize};
use std::path;

use super::HandleRequest;
use movie_collection_lib::{
    imdb_episodes::ImdbEpisodes,
    imdb_ratings::ImdbRatings,
    movie_collection::{
        find_new_episodes_http_worker, ImdbSeason, MovieCollection, MovieCollectionRow,
        TvShowsResult,
    },
    movie_queue::{MovieQueueDB, MovieQueueResult, MovieQueueRow},
    parse_imdb::{ParseImdb, ParseImdbOptions},
    pgpool::PgPool,
    trakt_instance,
    trakt_utils::{
        get_watched_shows_db, get_watchlist_shows_db_map, trakt_cal_http_worker,
        watch_list_http_worker, watched_action_http_worker, TraktActions, WatchListMap,
        WatchListShow, WatchedEpisode,
    },
    tv_show_source::TvShowSource,
};

pub struct TvShowsRequest {}

#[async_trait]
impl HandleRequest<TvShowsRequest> for PgPool {
    type Result = Result<Vec<TvShowsResult>, Error>;

    async fn handle(&self, _: TvShowsRequest) -> Self::Result {
        MovieCollection::with_pool(&self)?.print_tv_shows().await
    }
}

pub struct WatchlistShowsRequest {}

#[async_trait]
impl HandleRequest<WatchlistShowsRequest> for PgPool {
    type Result = Result<WatchListMap, Error>;

    async fn handle(&self, _: WatchlistShowsRequest) -> Self::Result {
        get_watchlist_shows_db_map(&self).await
    }
}

pub struct QueueDeleteRequest {
    pub path: String,
}

#[async_trait]
impl HandleRequest<QueueDeleteRequest> for PgPool {
    type Result = Result<String, Error>;
    async fn handle(&self, msg: QueueDeleteRequest) -> Self::Result {
        if path::Path::new(&msg.path).exists() {
            MovieQueueDB::with_pool(&self)
                .remove_from_queue_by_path(&msg.path)
                .await?;
        }
        Ok(msg.path)
    }
}

pub struct MovieQueueRequest {
    pub patterns: Vec<String>,
}

#[async_trait]
impl HandleRequest<MovieQueueRequest> for PgPool {
    type Result = Result<(Vec<MovieQueueResult>, Vec<String>), Error>;

    async fn handle(&self, msg: MovieQueueRequest) -> Self::Result {
        let patterns: Vec<_> = msg.patterns.iter().map(String::as_str).collect();
        let queue = MovieQueueDB::with_pool(&self)
            .print_movie_queue(&patterns)
            .await?;
        Ok((queue, msg.patterns))
    }
}

pub struct MoviePathRequest {
    pub idx: i32,
}

#[async_trait]
impl HandleRequest<MoviePathRequest> for PgPool {
    type Result = Result<String, Error>;

    async fn handle(&self, msg: MoviePathRequest) -> Self::Result {
        MovieCollection::with_pool(&self)?
            .get_collection_path(msg.idx)
            .await
    }
}

pub struct ImdbRatingsRequest {
    pub imdb_url: String,
}

#[async_trait]
impl HandleRequest<ImdbRatingsRequest> for PgPool {
    type Result = Result<Option<(String, ImdbRatings)>, Error>;

    async fn handle(&self, msg: ImdbRatingsRequest) -> Self::Result {
        ImdbRatings::get_show_by_link(&msg.imdb_url, &self)
            .await
            .map(|s| s.map(|sh| (msg.imdb_url, sh)))
    }
}

pub struct ImdbSeasonsRequest {
    pub show: String,
}

#[async_trait]
impl HandleRequest<ImdbSeasonsRequest> for PgPool {
    type Result = Result<Vec<ImdbSeason>, Error>;

    async fn handle(&self, msg: ImdbSeasonsRequest) -> Self::Result {
        if msg.show.as_str() == "" {
            Ok(Vec::new())
        } else {
            MovieCollection::with_pool(&self)?
                .print_imdb_all_seasons(&msg.show)
                .await
        }
    }
}

pub struct WatchlistActionRequest {
    pub action: TraktActions,
    pub imdb_url: String,
}

#[async_trait]
impl HandleRequest<WatchlistActionRequest> for PgPool {
    type Result = Result<String, Error>;

    async fn handle(&self, msg: WatchlistActionRequest) -> Self::Result {
        match msg.action {
            TraktActions::Add => {
                if let Some(show) = trakt_instance::get_watchlist_shows()?.get(&msg.imdb_url) {
                    show.insert_show(&self).await?;
                }
            }
            TraktActions::Remove => {
                if let Some(show) = WatchListShow::get_show_by_link(&msg.imdb_url, &self).await? {
                    show.delete_show(&self).await?;
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

#[async_trait]
impl HandleRequest<WatchedShowsRequest> for PgPool {
    type Result = Result<Vec<WatchedEpisode>, Error>;

    async fn handle(&self, msg: WatchedShowsRequest) -> Self::Result {
        get_watched_shows_db(&self, &msg.show, Some(msg.season)).await
    }
}

pub struct ImdbEpisodesRequest {
    pub show: String,
    pub season: Option<i32>,
}

#[async_trait]
impl HandleRequest<ImdbEpisodesRequest> for PgPool {
    type Result = Result<Vec<ImdbEpisodes>, Error>;

    async fn handle(&self, msg: ImdbEpisodesRequest) -> Self::Result {
        MovieCollection::with_pool(&self)?
            .print_imdb_episodes(&msg.show, msg.season)
            .await
    }
}

pub struct WatchedListRequest {
    pub imdb_url: String,
    pub season: i32,
}

#[async_trait]
impl HandleRequest<WatchedListRequest> for PgPool {
    type Result = Result<String, Error>;

    async fn handle(&self, msg: WatchedListRequest) -> Self::Result {
        watch_list_http_worker(&self, &msg.imdb_url, msg.season).await
    }
}

pub struct WatchedActionRequest {
    pub action: TraktActions,
    pub imdb_url: String,
    pub season: i32,
    pub episode: i32,
}

#[async_trait]
impl HandleRequest<WatchedActionRequest> for PgPool {
    type Result = Result<String, Error>;

    async fn handle(&self, msg: WatchedActionRequest) -> Self::Result {
        watched_action_http_worker(&self, msg.action, &msg.imdb_url, msg.season, msg.episode).await
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
        Self {
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
        Self {
            show: opts.show,
            ..opts.query.into()
        }
    }
}

#[async_trait]
impl HandleRequest<ImdbShowRequest> for PgPool {
    type Result = Result<String, Error>;

    async fn handle(&self, msg: ImdbShowRequest) -> Self::Result {
        let watchlist = get_watchlist_shows_db_map(&self).await?;
        let pi = ParseImdb::with_pool(&self)?;
        let body = pi.parse_imdb_http_worker(&msg.into(), &watchlist).await?;
        Ok(body)
    }
}

pub struct TraktCalRequest {}

#[async_trait]
impl HandleRequest<TraktCalRequest> for PgPool {
    type Result = Result<Vec<String>, Error>;

    async fn handle(&self, _: TraktCalRequest) -> Self::Result {
        trakt_cal_http_worker(&self).await
    }
}

#[derive(Serialize, Deserialize)]
pub struct FindNewEpisodeRequest {
    pub source: Option<TvShowSource>,
    pub shows: Option<String>,
}

#[async_trait]
impl HandleRequest<FindNewEpisodeRequest> for PgPool {
    type Result = Result<Vec<String>, Error>;

    async fn handle(&self, msg: FindNewEpisodeRequest) -> Self::Result {
        find_new_episodes_http_worker(&self, msg.shows, msg.source).await
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ImdbEpisodesSyncRequest {
    pub start_timestamp: DateTime<Utc>,
}

#[async_trait]
impl HandleRequest<ImdbEpisodesSyncRequest> for PgPool {
    type Result = Result<Vec<ImdbEpisodes>, Error>;

    async fn handle(&self, msg: ImdbEpisodesSyncRequest) -> Self::Result {
        ImdbEpisodes::get_episodes_after_timestamp(msg.start_timestamp, &self).await
    }
}

#[derive(Serialize, Deserialize)]
pub struct ImdbRatingsSyncRequest {
    pub start_timestamp: DateTime<Utc>,
}

#[async_trait]
impl HandleRequest<ImdbRatingsSyncRequest> for PgPool {
    type Result = Result<Vec<ImdbRatings>, Error>;

    async fn handle(&self, msg: ImdbRatingsSyncRequest) -> Self::Result {
        ImdbRatings::get_shows_after_timestamp(msg.start_timestamp, &self).await
    }
}

#[derive(Serialize, Deserialize)]
pub struct MovieQueueSyncRequest {
    pub start_timestamp: DateTime<Utc>,
}

#[async_trait]
impl HandleRequest<MovieQueueSyncRequest> for PgPool {
    type Result = Result<Vec<MovieQueueRow>, Error>;

    async fn handle(&self, msg: MovieQueueSyncRequest) -> Self::Result {
        let mq = MovieQueueDB::with_pool(&self);
        mq.get_queue_after_timestamp(msg.start_timestamp).await
    }
}

#[derive(Serialize, Deserialize)]
pub struct MovieCollectionSyncRequest {
    pub start_timestamp: DateTime<Utc>,
}

#[async_trait]
impl HandleRequest<MovieCollectionSyncRequest> for PgPool {
    type Result = Result<Vec<MovieCollectionRow>, Error>;

    async fn handle(&self, msg: MovieCollectionSyncRequest) -> Self::Result {
        let mc = MovieCollection::with_pool(&self)?;
        mc.get_collection_after_timestamp(msg.start_timestamp).await
    }
}

#[derive(Serialize, Deserialize)]
pub struct ImdbEpisodesUpdateRequest {
    pub episodes: Vec<ImdbEpisodes>,
}

#[async_trait]
impl HandleRequest<ImdbEpisodesUpdateRequest> for PgPool {
    type Result = Result<(), Error>;

    async fn handle(&self, msg: ImdbEpisodesUpdateRequest) -> Self::Result {
        for episode in msg.episodes {
            match episode.get_index(&self).await? {
                Some(_) => episode.update_episode(&self).await?,
                None => episode.insert_episode(&self).await?,
            }
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize)]
pub struct ImdbRatingsUpdateRequest {
    pub shows: Vec<ImdbRatings>,
}

#[async_trait]
impl HandleRequest<ImdbRatingsUpdateRequest> for PgPool {
    type Result = Result<(), Error>;

    async fn handle(&self, msg: ImdbRatingsUpdateRequest) -> Self::Result {
        for show in msg.shows {
            match ImdbRatings::get_show_by_link(&show.link, &self).await? {
                Some(_) => show.update_show(&self).await?,
                None => show.insert_show(&self).await?,
            }
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize)]
pub struct MovieQueueUpdateRequest {
    pub queue: Vec<MovieQueueRow>,
}

#[async_trait]
impl HandleRequest<MovieQueueUpdateRequest> for PgPool {
    type Result = Result<(), Error>;

    async fn handle(&self, msg: MovieQueueUpdateRequest) -> Self::Result {
        let mq = MovieQueueDB::with_pool(&self);
        let mc = MovieCollection::with_pool(&self)?;
        for entry in msg.queue {
            let cidx = if let Some(i) = mc.get_collection_index(&entry.path).await? {
                i
            } else {
                mc.insert_into_collection_by_idx(entry.collection_idx, &entry.path)
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

#[async_trait]
impl HandleRequest<MovieCollectionUpdateRequest> for PgPool {
    type Result = Result<(), Error>;

    async fn handle(&self, msg: MovieCollectionUpdateRequest) -> Self::Result {
        let mc = MovieCollection::with_pool(&self)?;
        for entry in msg.collection {
            if let Some(cidx) = mc.get_collection_index(&entry.path).await? {
                if cidx == entry.idx {
                    continue;
                }
                mc.remove_from_collection(&entry.path).await?;
            };
            mc.insert_into_collection_by_idx(entry.idx, &entry.path)
                .await?;
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

#[async_trait]
impl HandleRequest<LastModifiedRequest> for PgPool {
    type Result = Result<Vec<LastModifiedResponse>, Error>;

    async fn handle(&self, _: LastModifiedRequest) -> Self::Result {
        let tables = vec![
            "imdb_episodes",
            "imdb_ratings",
            "movie_collection",
            "movie_queue",
        ];

        let futures = tables.into_iter().map(|table| async move {
            let query = format!("SELECT max(last_modified) FROM {}", table);
            if let Some(row) = self.get().await?.query(query.as_str(), &[]).await?.get(0) {
                let last_modified: DateTime<Utc> = row.try_get(0)?;
                Ok(Some(LastModifiedResponse {
                    table: (*table).to_string(),
                    last_modified,
                }))
            } else {
                Ok(None)
            }
        });
        let results: Result<Vec<_>, Error> = try_join_all(futures).await;
        let results: Vec<_> = results?.into_iter().filter_map(|x| x).collect();
        Ok(results)
    }
}
