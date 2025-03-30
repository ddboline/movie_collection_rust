use anyhow::format_err;
use futures::TryStreamExt;
use serde::{Deserialize, Serialize};
use stack_string::StackString;
use stdout_channel::{MockStdout, StdoutChannel};
use time::OffsetDateTime;
use utoipa::ToSchema;
use utoipa_helper::derive_utoipa_schema;

use movie_collection_lib::{
    config::Config,
    date_time_wrapper::DateTimeWrapper,
    imdb_episodes::{ImdbEpisodes, ImdbSeason},
    imdb_ratings::ImdbRatings,
    movie_collection::{MovieCollection, MovieCollectionRow},
    movie_queue::{MovieQueueDB, MovieQueueResult, MovieQueueRow, OrderBy},
    parse_imdb::{ParseImdb, ParseImdbOptions},
    pgpool::PgPool,
    trakt_utils::{get_watched_shows_db, get_watchlist_shows_db_map, WatchedEpisode},
    tv_show_source::TvShowSource,
};

use crate::{
    errors::ServiceError as Error, movie_queue_elements::parse_imdb_http_body, ImdbEpisodesWrapper,
    ImdbRatingsWrapper, MovieCollectionRowWrapper, MovieQueueRowWrapper, TvShowSourceWrapper,
};

#[derive(Debug, Default)]
pub struct MovieQueueRequest {
    pub patterns: Vec<StackString>,
    pub offset: Option<usize>,
    pub limit: Option<usize>,
    pub order_by: Option<OrderBy>,
}

impl MovieQueueRequest {
    /// # Errors
    /// Return error if `print_movie_queue` fails
    pub async fn process(
        self,
        pool: &PgPool,
        config: &Config,
    ) -> Result<(Vec<MovieQueueResult>, Vec<StackString>), Error> {
        let mock_stdout = MockStdout::new();
        let stdout = StdoutChannel::with_mock_stdout(mock_stdout.clone(), mock_stdout);

        let patterns: Vec<_> = self.patterns.iter().map(StackString::as_str).collect();
        let queue = MovieQueueDB::new(config, pool, &stdout)
            .print_movie_queue(&patterns, self.offset, self.limit, self.order_by)
            .await?;
        Ok((queue, self.patterns))
    }
}

pub struct ImdbRatingsRequest {
    pub imdb_url: StackString,
}

impl ImdbRatingsRequest {
    /// # Errors
    /// Return error if `get_show_by_link` fails
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
    /// # Errors
    /// Return error if `print_imdb_all_seasons` fails
    pub async fn process(&self, pool: &PgPool, config: &Config) -> Result<Vec<ImdbSeason>, Error> {
        let mock_stdout = MockStdout::new();
        let stdout = StdoutChannel::with_mock_stdout(mock_stdout.clone(), mock_stdout);

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

pub struct WatchedShowsRequest {
    pub show: StackString,
    pub season: i32,
}

impl WatchedShowsRequest {
    /// # Errors
    /// Return error if `get_watched_shows_db` fails
    pub async fn handle(&self, pool: &PgPool) -> Result<Vec<WatchedEpisode>, Error> {
        let episodes = get_watched_shows_db(pool, &self.show, Some(self.season))
            .await?
            .try_collect()
            .await?;
        Ok(episodes)
    }
}

pub struct ImdbEpisodesRequest {
    pub show: StackString,
    pub season: Option<i32>,
}

impl ImdbEpisodesRequest {
    /// # Errors
    /// Return error if `print_imdb_episodes` fails
    pub async fn handle(&self, pool: &PgPool, config: &Config) -> Result<Vec<ImdbEpisodes>, Error> {
        let mock_stdout = MockStdout::new();
        let stdout = StdoutChannel::with_mock_stdout(mock_stdout.clone(), mock_stdout);

        MovieCollection::new(config, pool, &stdout)
            .print_imdb_episodes(&self.show, self.season)
            .await
            .map_err(Into::into)
    }
}

#[derive(Deserialize, Default, ToSchema)]
pub struct ParseImdbRequest {
    // All Entries Flag
    pub all: Option<bool>,
    // Database Flag
    pub database: Option<bool>,
    // IsTv Flag
    pub tv: Option<bool>,
    // Update Flag
    pub update: Option<bool>,
    // IMDB ID
    pub link: Option<StackString>,
    // Season
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
    /// # Errors
    /// Return error if `parse_imdb_http_worker` fails
    pub async fn process(self, pool: &PgPool, config: &Config) -> Result<StackString, Error> {
        let mock_stdout = MockStdout::new();
        let stdout = StdoutChannel::with_mock_stdout(mock_stdout.clone(), mock_stdout);

        let watchlist =
            get_watchlist_shows_db_map(pool, Some(&self.show), None, None, None).await?;
        let pi = ParseImdb::new(config, pool, &stdout);
        let body = parse_imdb_http_body(&pi, &self.into(), watchlist)
            .await?
            .into();
        Ok(body)
    }
}

#[derive(ToSchema)]
#[allow(dead_code)]
struct _ImdbEpisodesSyncRequest {
    // Start Timestamp
    pub start_timestamp: OffsetDateTime,
    // Offset
    pub offset: Option<usize>,
    // Limit
    pub limit: Option<usize>,
}

#[derive(Serialize, Deserialize)]
pub struct MovieQueueSyncRequest {
    pub start_timestamp: Option<DateTimeWrapper>,
    pub offset: Option<usize>,
    pub limit: Option<usize>,
}

derive_utoipa_schema!(MovieQueueSyncRequest, _ImdbEpisodesSyncRequest);

impl MovieQueueSyncRequest {
    /// # Errors
    /// Return error if `get_queue_after_timestamp` fails
    pub async fn get_queue(
        &self,
        pool: &PgPool,
        config: &Config,
    ) -> Result<(usize, Vec<MovieQueueRow>), Error> {
        let mock_stdout = MockStdout::new();
        let stdout = StdoutChannel::with_mock_stdout(mock_stdout.clone(), mock_stdout);

        let timestamp = self.start_timestamp.map(Into::into);
        let offset = self.offset;
        let limit = self.limit;

        let mq = MovieQueueDB::new(config, pool, &stdout);
        let total = mq.get_total(timestamp).await?;
        let data = mq
            .get_queue_after_timestamp(timestamp, offset, limit)
            .await?;
        Ok((total, data))
    }
}

#[derive(Serialize, Deserialize)]
pub struct MovieCollectionSyncRequest {
    pub start_timestamp: Option<DateTimeWrapper>,
    pub offset: Option<usize>,
    pub limit: Option<usize>,
}

derive_utoipa_schema!(MovieCollectionSyncRequest, _ImdbEpisodesSyncRequest);

impl MovieCollectionSyncRequest {
    /// # Errors
    /// Return error if `get_collection_after_timestamp` fails
    pub async fn get_collection(
        &self,
        pool: &PgPool,
        config: &Config,
    ) -> Result<(usize, Vec<MovieCollectionRow>), Error> {
        let mock_stdout = MockStdout::new();
        let stdout = StdoutChannel::with_mock_stdout(mock_stdout.clone(), mock_stdout);

        let timestamp = self.start_timestamp.map(Into::into);
        let offset = self.offset;
        let limit = self.limit;

        let mc = MovieCollection::new(config, pool, &stdout);
        let total = mc.get_total(timestamp).await?;
        let data = mc
            .get_collection_after_timestamp(timestamp, offset, limit)
            .await?;
        Ok((total, data))
    }
}

#[derive(Serialize, Deserialize, ToSchema)]
pub struct ImdbEpisodesUpdateRequest {
    pub episodes: Vec<ImdbEpisodesWrapper>,
}

impl ImdbEpisodesUpdateRequest {
    /// # Errors
    /// Return error if db queries fail
    pub async fn run_update(self, pool: &PgPool) -> Result<(), Error> {
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

#[derive(Serialize, Deserialize, ToSchema)]
pub struct ImdbRatingsUpdateRequest {
    pub shows: Vec<ImdbRatingsWrapper>,
}

impl ImdbRatingsUpdateRequest {
    /// # Errors
    /// Return error if db queries fail
    pub async fn run_update(self, pool: &PgPool) -> Result<(), Error> {
        for show in self.shows {
            let show: ImdbRatings = show.into();
            match ImdbRatings::get_show_by_link(show.link.as_ref(), pool).await? {
                Some(_) => {
                    show.update_show(pool).await?;
                }
                None => {
                    show.insert_show(pool).await?;
                }
            }
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize, ToSchema)]
pub struct ImdbRatingsSetSourceRequest {
    // IMDB ID
    pub link: StackString,
    // TV Show Source
    pub source: TvShowSourceWrapper,
}

impl ImdbRatingsSetSourceRequest {
    /// # Errors
    /// Return error if db queries fail
    pub async fn set_source(&self, pool: &PgPool) -> Result<(), Error> {
        let link = &self.link;
        let mut imdb = ImdbRatings::get_show_by_link(link.as_str(), pool)
            .await?
            .ok_or_else(|| format_err!("No show found for {link}"))?;
        let source: TvShowSource = self.source.into();
        imdb.source = if source == TvShowSource::All {
            None
        } else {
            Some(self.source.into())
        };
        imdb.update_show(pool).await?;
        Ok(())
    }
}

#[derive(Serialize, Deserialize, ToSchema)]
pub struct MovieQueueUpdateRequest {
    pub queue: Vec<MovieQueueRowWrapper>,
}

impl MovieQueueUpdateRequest {
    /// # Errors
    /// Return error if db queries fail
    pub async fn run_update(self, pool: &PgPool, config: &Config) -> Result<(), Error> {
        let mock_stdout = MockStdout::new();
        let stdout = StdoutChannel::with_mock_stdout(mock_stdout.clone(), mock_stdout);

        let mq = MovieQueueDB::new(config, pool, &stdout);
        let mc = MovieCollection::new(config, pool, &stdout);
        for entry in self.queue {
            let mut entry: MovieQueueRow = entry.into();
            let cidx = if let Some(i) = mc.get_collection_index(entry.path.as_ref()).await? {
                i
            } else {
                mc.insert_into_collection(entry.path.as_ref(), false)
                    .await?;
                entry.collection_idx
            };
            entry.collection_idx = cidx;

            if mq.get_idx_from_collection_idx(cidx).await?.is_none() {
                mq.insert_into_queue_by_collection_idx(entry.idx, entry.collection_idx)
                    .await?;
            }
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize, ToSchema)]
pub struct MovieCollectionUpdateRequest {
    pub collection: Vec<MovieCollectionRowWrapper>,
}

impl MovieCollectionUpdateRequest {
    /// # Errors
    /// Return error if db queries fail
    pub async fn run_update(&self, pool: &PgPool, config: &Config) -> Result<(), Error> {
        let mock_stdout = MockStdout::new();
        let stdout = StdoutChannel::with_mock_stdout(mock_stdout.clone(), mock_stdout);

        let mc = MovieCollection::new(config, pool, &stdout);
        for entry in &self.collection {
            mc.insert_into_collection(entry.path.as_ref(), false)
                .await?;
        }
        Ok(())
    }
}
