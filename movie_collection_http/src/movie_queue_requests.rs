use anyhow::{format_err, Error};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use stack_string::StackString;
use stdout_channel::{MockStdout, StdoutChannel};

use movie_collection_lib::{
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
    tv_show_source::TvShowSource,
};

pub struct WatchlistShowsRequest {}

impl WatchlistShowsRequest {
    pub async fn handle(&self, pool: &PgPool) -> Result<WatchListMap, Error> {
        get_watchlist_shows_db_map(&pool).await
    }
}

#[derive(Debug)]
pub struct MovieQueueRequest {
    pub patterns: Vec<StackString>,
}

impl MovieQueueRequest {
    pub async fn handle(
        &self,
        msg: MovieQueueRequest,
    ) -> Result<(Vec<MovieQueueResult>, Vec<StackString>), Error> {
        let mock_stdout = MockStdout::new();
        let stdout = StdoutChannel::with_mock_stdout(mock_stdout.clone(), mock_stdout.clone());

        let patterns: Vec<_> = msg.patterns.iter().map(StackString::as_str).collect();
        let queue = MovieQueueDB::new(&CONFIG, &self, &stdout)
            .print_movie_queue(&patterns)
            .await?;
        Ok((queue, msg.patterns))
    }
}

pub struct MoviePathRequest {
    pub idx: i32,
}

impl MoviePathRequest {
    pub async fn handle(&self, msg: MoviePathRequest) -> Result<StackString, Error> {
        let mock_stdout = MockStdout::new();
        let stdout = StdoutChannel::with_mock_stdout(mock_stdout.clone(), mock_stdout.clone());

        MovieCollection::new(&CONFIG, &self, &stdout)
            .get_collection_path(msg.idx)
            .await
    }
}

pub struct ImdbRatingsRequest {
    pub imdb_url: StackString,
}

impl ImdbRatingsRequest {
    pub async fn handle(
        &self,
        msg: ImdbRatingsRequest,
    ) -> Result<Option<(StackString, ImdbRatings)>, Error> {
        ImdbRatings::get_show_by_link(&msg.imdb_url, &self)
            .await
            .map(|s| s.map(|sh| (msg.imdb_url, sh)))
    }
}

pub struct ImdbSeasonsRequest {
    pub show: StackString,
}

impl ImdbSeasonsRequest {
    pub async fn handle(&self, msg: ImdbSeasonsRequest) -> Result<Vec<ImdbSeason>, Error> {
        let mock_stdout = MockStdout::new();
        let stdout = StdoutChannel::with_mock_stdout(mock_stdout.clone(), mock_stdout.clone());

        if &msg.show == "" {
            Ok(Vec::new())
        } else {
            MovieCollection::new(&CONFIG, &self, &stdout)
                .print_imdb_all_seasons(&msg.show)
                .await
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
                    show.insert_show(&pool).await?;
                }
            }
            TraktActions::Remove => {
                if let Some(show) = WatchListShow::get_show_by_link(&self.imdb_url, &pool).await? {
                    show.delete_show(&pool).await?;
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
    pub async fn handle(&self, msg: WatchedShowsRequest) -> Result<Vec<WatchedEpisode>, Error> {
        get_watched_shows_db(&self, &msg.show, Some(msg.season)).await
    }
}

pub struct ImdbEpisodesRequest {
    pub show: StackString,
    pub season: Option<i32>,
}

impl ImdbEpisodesRequest {
    pub async fn handle(&self, msg: ImdbEpisodesRequest) -> Result<Vec<ImdbEpisodes>, Error> {
        let mock_stdout = MockStdout::new();
        let stdout = StdoutChannel::with_mock_stdout(mock_stdout.clone(), mock_stdout.clone());

        MovieCollection::new(&CONFIG, &self, &stdout)
            .print_imdb_episodes(&msg.show, msg.season)
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
    pub async fn handle(&self, msg: ImdbShowRequest) -> Result<StackString, Error> {
        let mock_stdout = MockStdout::new();
        let stdout = StdoutChannel::with_mock_stdout(mock_stdout.clone(), mock_stdout.clone());

        let watchlist = get_watchlist_shows_db_map(&self).await?;
        let pi = ParseImdb::new(&CONFIG, self, &stdout);
        let body = pi.parse_imdb_http_worker(&msg.into(), &watchlist).await?;
        Ok(body)
    }
}

#[derive(Serialize, Deserialize)]
pub struct FindNewEpisodeRequest {
    pub source: Option<TvShowSource>,
    pub shows: Option<StackString>,
}

impl FindNewEpisodeRequest {
    pub async fn handle(&self, msg: FindNewEpisodeRequest) -> Result<Vec<StackString>, Error> {
        let mock_stdout = MockStdout::new();
        let stdout = StdoutChannel::with_mock_stdout(mock_stdout.clone(), mock_stdout.clone());

        find_new_episodes_http_worker(&CONFIG, &self, &stdout, msg.shows, msg.source).await
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ImdbEpisodesSyncRequest {
    pub start_timestamp: DateTime<Utc>,
}

impl ImdbEpisodesSyncRequest {
    pub async fn handle(&self, msg: ImdbEpisodesSyncRequest) -> Result<Vec<ImdbEpisodes>, Error> {
        ImdbEpisodes::get_episodes_after_timestamp(msg.start_timestamp, &self).await
    }
}

#[derive(Serialize, Deserialize)]
pub struct ImdbRatingsSyncRequest {
    pub start_timestamp: DateTime<Utc>,
}

impl ImdbRatingsSyncRequest {
    pub async fn handle(&self, msg: ImdbRatingsSyncRequest) -> Result<Vec<ImdbRatings>, Error> {
        ImdbRatings::get_shows_after_timestamp(msg.start_timestamp, &self).await
    }
}

#[derive(Serialize, Deserialize)]
pub struct MovieQueueSyncRequest {
    pub start_timestamp: DateTime<Utc>,
}

impl MovieQueueSyncRequest {
    pub async fn handle(&self, msg: MovieQueueSyncRequest) -> Result<Vec<MovieQueueRow>, Error> {
        let mock_stdout = MockStdout::new();
        let stdout = StdoutChannel::with_mock_stdout(mock_stdout.clone(), mock_stdout.clone());

        let mq = MovieQueueDB::new(&CONFIG, self, &stdout);
        mq.get_queue_after_timestamp(msg.start_timestamp).await
    }
}

#[derive(Serialize, Deserialize)]
pub struct MovieCollectionSyncRequest {
    pub start_timestamp: DateTime<Utc>,
}

impl MovieCollectionSyncRequest {
    pub async fn handle(
        &self,
        msg: MovieCollectionSyncRequest,
    ) -> Result<Vec<MovieCollectionRow>, Error> {
        let mock_stdout = MockStdout::new();
        let stdout = StdoutChannel::with_mock_stdout(mock_stdout.clone(), mock_stdout.clone());

        let mc = MovieCollection::new(&CONFIG, self, &stdout);
        mc.get_collection_after_timestamp(msg.start_timestamp).await
    }
}

#[derive(Serialize, Deserialize)]
pub struct ImdbEpisodesUpdateRequest {
    pub episodes: Vec<ImdbEpisodes>,
}

impl ImdbEpisodesUpdateRequest {
    pub async fn handle(&self, msg: ImdbEpisodesUpdateRequest) -> Result<(), Error> {
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

impl ImdbRatingsUpdateRequest {
    pub async fn handle(&self, msg: ImdbRatingsUpdateRequest) -> Result<(), Error> {
        for show in msg.shows {
            match ImdbRatings::get_show_by_link(show.link.as_ref(), &self).await? {
                Some(_) => show.update_show(&self).await?,
                None => show.insert_show(&self).await?,
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
    pub async fn handle(&self, msg: ImdbRatingsSetSourceRequest) -> Result<(), Error> {
        let mut imdb = ImdbRatings::get_show_by_link(msg.link.as_ref(), self)
            .await?
            .ok_or_else(|| format_err!("No show found for {}", msg.link))?;
        imdb.source = if msg.source == TvShowSource::All {
            None
        } else {
            Some(msg.source)
        };
        imdb.update_show(self).await?;
        Ok(())
    }
}

#[derive(Serialize, Deserialize)]
pub struct MovieQueueUpdateRequest {
    pub queue: Vec<MovieQueueRow>,
}

impl MovieQueueUpdateRequest {
    pub async fn handle(&self, msg: MovieQueueUpdateRequest) -> Result<(), Error> {
        let mock_stdout = MockStdout::new();
        let stdout = StdoutChannel::with_mock_stdout(mock_stdout.clone(), mock_stdout.clone());

        let mq = MovieQueueDB::new(&CONFIG, self, &stdout);
        let mc = MovieCollection::new(&CONFIG, self, &stdout);
        for entry in msg.queue {
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
    pub async fn handle(&self, msg: MovieCollectionUpdateRequest) -> Result<(), Error> {
        let mock_stdout = MockStdout::new();
        let stdout = StdoutChannel::with_mock_stdout(mock_stdout.clone(), mock_stdout.clone());

        let mc = MovieCollection::new(&CONFIG, self, &stdout);
        for entry in msg.collection {
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

pub struct LastModifiedRequest {}

impl LastModifiedRequest {
    pub async fn handle(&self, _: LastModifiedRequest) -> Result<Vec<LastModifiedResponse>, Error> {
        LastModifiedResponse::get_last_modified(self).await
    }
}

pub struct ImdbSeasonsRequest {
    pub show: StackString,
}

impl ImdbSeasonsRequest {
    pub async fn handle(&self, pool: &PgPool) -> Result<Vec<ImdbSeason>, Error> {
        let mock_stdout = MockStdout::new();
        let stdout = StdoutChannel::with_mock_stdout(mock_stdout.clone(), mock_stdout.clone());

        if &self.show == "" {
            Ok(Vec::new())
        } else {
            MovieCollection::new(&CONFIG, pool, &stdout)
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
    pub async fn handle(
        self,
        pool: &PgPool,
        trakt: &TraktConnection,
    ) -> Result<StackString, Error> {
        match self.action {
            TraktActions::Add => {
                trakt.init().await;
                if let Some(show) = trakt.get_watchlist_shows().await?.get(&self.imdb_url) {
                    show.insert_show(&pool).await?;
                }
            }
            TraktActions::Remove => {
                if let Some(show) = WatchListShow::get_show_by_link(&self.imdb_url, &pool).await? {
                    show.delete_show(&pool).await?;
                }
            }
            _ => {}
        }
        Ok(self.imdb_url)
    }
}
