use anyhow::format_err;
use chrono::{DateTime, Utc};
use rweb::Schema;
use serde::{Deserialize, Serialize};
use stack_string::StackString;
use stdout_channel::{MockStdout, StdoutChannel};

use movie_collection_lib::{
    config::Config,
    imdb_episodes::ImdbEpisodes,
    imdb_ratings::ImdbRatings,
    movie_collection::{
        find_new_episodes_http_worker, ImdbSeason, LastModifiedResponse, MovieCollection,
        MovieCollectionRow,
    },
    movie_queue::{MovieQueueDB, MovieQueueResult, MovieQueueRow},
    parse_imdb::{ParseImdb, ParseImdbOptions},
    pgpool::PgPool,
    trakt_connection::TraktConnection,
    trakt_utils::{
        get_watched_shows_db, get_watchlist_shows_db_map, TraktActions, WatchListMap,
        WatchListShow, WatchedEpisode,
    },
};

use crate::{
    errors::ServiceError as Error, ImdbEpisodesWrapper, ImdbRatingsWrapper,
    MovieCollectionRowWrapper, MovieQueueRowWrapper, TvShowSourceWrapper,
};

pub struct WatchlistShowsRequest {}

impl WatchlistShowsRequest {
    pub async fn handle(&self, pool: &PgPool) -> Result<WatchListMap, Error> {
        get_watchlist_shows_db_map(pool).await.map_err(Into::into)
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
        config: &Config,
    ) -> Result<(Vec<MovieQueueResult>, Vec<StackString>), Error> {
        let mock_stdout = MockStdout::new();
        let stdout = StdoutChannel::with_mock_stdout(mock_stdout.clone(), mock_stdout.clone());

        let patterns: Vec<_> = self.patterns.iter().map(StackString::as_str).collect();
        let queue = MovieQueueDB::new(config, pool, &stdout)
            .print_movie_queue(&patterns)
            .await?;
        Ok((queue, self.patterns))
    }
}

pub struct MoviePathRequest {
    pub idx: i32,
}

impl MoviePathRequest {
    pub async fn handle(&self, pool: &PgPool, config: &Config) -> Result<StackString, Error> {
        let mock_stdout = MockStdout::new();
        let stdout = StdoutChannel::with_mock_stdout(mock_stdout.clone(), mock_stdout.clone());

        MovieCollection::new(config, pool, &stdout)
            .get_collection_path(self.idx)
            .await
            .map_err(Into::into)
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
            .map_err(Into::into)
    }
}

pub struct ImdbSeasonsRequest {
    pub show: StackString,
}

impl ImdbSeasonsRequest {
    pub async fn handle(&self, pool: &PgPool, config: &Config) -> Result<Vec<ImdbSeason>, Error> {
        let mock_stdout = MockStdout::new();
        let stdout = StdoutChannel::with_mock_stdout(mock_stdout.clone(), mock_stdout.clone());

        if &self.show == "" {
            Ok(Vec::new())
        } else {
            MovieCollection::new(config, pool, &stdout)
                .print_imdb_all_seasons(&self.show)
                .await
                .map_err(Into::into)
        }
    }
}

pub struct WatchlistActionRequest {
    pub action: TraktActions,
    pub imdb_url: StackString,
}

impl WatchlistActionRequest {
    pub async fn handle(
        self,
        pool: &PgPool,
        trakt: &TraktConnection,
    ) -> Result<StackString, Error> {
        match self.action {
            TraktActions::Add => {
                trakt.init().await;
                if let Some(show) = trakt.get_watchlist_shows().await?.get(&self.imdb_url) {
                    show.insert_show(pool).await?;
                }
            }
            TraktActions::Remove => {
                if let Some(show) = WatchListShow::get_show_by_link(&self.imdb_url, pool).await? {
                    show.delete_show(pool).await?;
                }
            }
            _ => {}
        }
        Ok(self.imdb_url)
    }
}

pub struct WatchedShowsRequest {
    pub show: StackString,
    pub season: i32,
}

impl WatchedShowsRequest {
    pub async fn handle(&self, pool: &PgPool) -> Result<Vec<WatchedEpisode>, Error> {
        get_watched_shows_db(pool, &self.show, Some(self.season))
            .await
            .map_err(Into::into)
    }
}

pub struct ImdbEpisodesRequest {
    pub show: StackString,
    pub season: Option<i32>,
}

impl ImdbEpisodesRequest {
    pub async fn handle(&self, pool: &PgPool, config: &Config) -> Result<Vec<ImdbEpisodes>, Error> {
        let mock_stdout = MockStdout::new();
        let stdout = StdoutChannel::with_mock_stdout(mock_stdout.clone(), mock_stdout.clone());

        MovieCollection::new(config, pool, &stdout)
            .print_imdb_episodes(&self.show, self.season)
            .await
            .map_err(Into::into)
    }
}

#[derive(Deserialize, Default, Schema)]
pub struct ParseImdbRequest {
    #[schema(description="All Entries Flag")]
    pub all: Option<bool>,
    #[schema(description="Database Flag")]
    pub database: Option<bool>,
    #[schema(description="IsTv Flag")]
    pub tv: Option<bool>,
    #[schema(description="Update Flag")]
    pub update: Option<bool>,
    #[schema(description="IMDB ID")]
    pub link: Option<StackString>,
    #[schema(description="Season")]
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
    pub async fn handle(self, pool: &PgPool, config: &Config) -> Result<StackString, Error> {
        let mock_stdout = MockStdout::new();
        let stdout = StdoutChannel::with_mock_stdout(mock_stdout.clone(), mock_stdout.clone());

        let watchlist = get_watchlist_shows_db_map(pool).await?;
        let pi = ParseImdb::new(config, pool, &stdout);
        let body = pi.parse_imdb_http_worker(&self.into(), &watchlist).await?;
        Ok(body)
    }
}

#[derive(Serialize, Deserialize, Schema)]
pub struct FindNewEpisodeRequest {
    #[schema(description="TV Show Source")]
    pub source: Option<TvShowSourceWrapper>,
    #[schema(description="TV Show")]
    pub shows: Option<StackString>,
}

impl FindNewEpisodeRequest {
    pub async fn handle(self, pool: &PgPool, config: &Config) -> Result<Vec<StackString>, Error> {
        let mock_stdout = MockStdout::new();
        let stdout = StdoutChannel::with_mock_stdout(mock_stdout.clone(), mock_stdout.clone());

        find_new_episodes_http_worker(
            config,
            pool,
            &stdout,
            self.shows,
            self.source.map(Into::into),
        )
        .await
        .map_err(Into::into)
    }
}

#[derive(Serialize, Deserialize, Debug, Schema)]
pub struct ImdbEpisodesSyncRequest {
    #[schema(description="Start Timestamp")]
    pub start_timestamp: DateTime<Utc>,
}

impl ImdbEpisodesSyncRequest {
    pub async fn handle(&self, pool: &PgPool) -> Result<Vec<ImdbEpisodes>, Error> {
        ImdbEpisodes::get_episodes_after_timestamp(self.start_timestamp.into(), pool)
            .await
            .map_err(Into::into)
    }
}

#[derive(Serialize, Deserialize, Schema)]
pub struct ImdbRatingsSyncRequest {
    pub start_timestamp: DateTime<Utc>,
}

impl ImdbRatingsSyncRequest {
    pub async fn handle(&self, pool: &PgPool) -> Result<Vec<ImdbRatings>, Error> {
        ImdbRatings::get_shows_after_timestamp(self.start_timestamp.into(), pool)
            .await
            .map_err(Into::into)
    }
}

#[derive(Serialize, Deserialize, Schema)]
pub struct MovieQueueSyncRequest {
    pub start_timestamp: DateTime<Utc>,
}

impl MovieQueueSyncRequest {
    pub async fn handle(
        &self,
        pool: &PgPool,
        config: &Config,
    ) -> Result<Vec<MovieQueueRow>, Error> {
        let mock_stdout = MockStdout::new();
        let stdout = StdoutChannel::with_mock_stdout(mock_stdout.clone(), mock_stdout.clone());

        let mq = MovieQueueDB::new(config, pool, &stdout);
        mq.get_queue_after_timestamp(self.start_timestamp.into())
            .await
            .map_err(Into::into)
    }
}

#[derive(Serialize, Deserialize, Schema)]
pub struct MovieCollectionSyncRequest {
    pub start_timestamp: DateTime<Utc>,
}

impl MovieCollectionSyncRequest {
    pub async fn handle(
        &self,
        pool: &PgPool,
        config: &Config,
    ) -> Result<Vec<MovieCollectionRow>, Error> {
        let mock_stdout = MockStdout::new();
        let stdout = StdoutChannel::with_mock_stdout(mock_stdout.clone(), mock_stdout.clone());

        let mc = MovieCollection::new(config, pool, &stdout);
        mc.get_collection_after_timestamp(self.start_timestamp.into())
            .await
            .map_err(Into::into)
    }
}

#[derive(Serialize, Deserialize, Schema)]
pub struct ImdbEpisodesUpdateRequest {
    pub episodes: Vec<ImdbEpisodesWrapper>,
}

impl ImdbEpisodesUpdateRequest {
    pub async fn handle(self, pool: &PgPool) -> Result<(), Error> {
        for episode in self.episodes {
            let episode: ImdbEpisodes = episode.into();
            match episode.get_index(pool).await? {
                Some(_) => episode.update_episode(pool).await?,
                None => episode.insert_episode(pool).await?,
            }
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize, Schema)]
pub struct ImdbRatingsUpdateRequest {
    pub shows: Vec<ImdbRatingsWrapper>,
}

impl ImdbRatingsUpdateRequest {
    pub async fn handle(self, pool: &PgPool) -> Result<(), Error> {
        for show in self.shows {
            let show: ImdbRatings = show.into();
            match ImdbRatings::get_show_by_link(show.link.as_ref(), pool).await? {
                Some(_) => show.update_show(pool).await?,
                None => show.insert_show(pool).await?,
            }
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize, Schema)]
pub struct ImdbRatingsSetSourceRequest {
    #[schema(description="IMDB ID")]
    pub link: StackString,
    #[schema(description="TV Show Source")]
    pub source: TvShowSourceWrapper,
}

impl ImdbRatingsSetSourceRequest {
    pub async fn handle(&self, pool: &PgPool) -> Result<(), Error> {
        let mut imdb = ImdbRatings::get_show_by_link(self.link.as_ref(), pool)
            .await?
            .ok_or_else(|| format_err!("No show found for {}", self.link))?;
        imdb.source = if self.source == TvShowSourceWrapper::All {
            None
        } else {
            Some(self.source.into())
        };
        imdb.update_show(pool).await?;
        Ok(())
    }
}

#[derive(Serialize, Deserialize, Schema)]
pub struct MovieQueueUpdateRequest {
    pub queue: Vec<MovieQueueRowWrapper>,
}

impl MovieQueueUpdateRequest {
    pub async fn handle(&self, pool: &PgPool, config: &Config) -> Result<(), Error> {
        let mock_stdout = MockStdout::new();
        let stdout = StdoutChannel::with_mock_stdout(mock_stdout.clone(), mock_stdout.clone());

        let mq = MovieQueueDB::new(config, pool, &stdout);
        let mc = MovieCollection::new(config, pool, &stdout);
        for entry in &self.queue {
            let cidx = if let Some(i) = mc.get_collection_index(entry.path.as_ref()).await? {
                i
            } else {
                mc.insert_into_collection(entry.path.as_ref(), false)
                    .await?;
                entry.collection_idx
            };
            if cidx != entry.collection_idx {
                return Err(format_err!("{} != {}", cidx, entry.collection_idx).into());
            }
            mq.remove_from_queue_by_collection_idx(entry.collection_idx)
                .await?;
            mq.insert_into_queue_by_collection_idx(entry.idx, entry.collection_idx)
                .await?;
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize, Schema)]
pub struct MovieCollectionUpdateRequest {
    pub collection: Vec<MovieCollectionRowWrapper>,
}

impl MovieCollectionUpdateRequest {
    pub async fn handle(&self, pool: &PgPool, config: &Config) -> Result<(), Error> {
        let mock_stdout = MockStdout::new();
        let stdout = StdoutChannel::with_mock_stdout(mock_stdout.clone(), mock_stdout.clone());

        let mc = MovieCollection::new(config, pool, &stdout);
        for entry in &self.collection {
            if let Some(cidx) = mc.get_collection_index(entry.path.as_ref()).await? {
                if cidx == entry.idx {
                    continue;
                }
                mc.remove_from_collection(entry.path.as_ref()).await?;
            };
            mc.insert_into_collection(entry.path.as_ref(), false)
                .await?;
        }
        Ok(())
    }
}

pub struct LastModifiedRequest {}

impl LastModifiedRequest {
    pub async fn handle(&self, pool: &PgPool) -> Result<Vec<LastModifiedResponse>, Error> {
        LastModifiedResponse::get_last_modified(pool)
            .await
            .map_err(Into::into)
    }
}
