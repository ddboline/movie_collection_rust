use anyhow::{format_err, Error};
use serde::{Deserialize, Serialize};
use stack_string::StackString;

use movie_collection_lib::{
    config::Config,
    imdb_ratings::ImdbRatings,
    movie_collection::{MovieCollection, MovieCollectionRow},
    movie_queue::{MovieQueueDB, MovieQueueResult, MovieQueueRow},
    parse_imdb::{ParseImdb, ParseImdbOptions},
    pgpool::PgPool,
    trakt_connection::TraktConnection,
    trakt_utils::{get_watchlist_shows_db_map, TraktActions, WatchListShow},
    tv_show_source::TvShowSource,
};

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
        let patterns: Vec<_> = self.patterns.iter().map(StackString::as_str).collect();
        let queue = MovieQueueDB::with_pool(config, pool)
            .print_movie_queue(&patterns)
            .await?;
        Ok((queue, self.patterns))
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
    pub async fn handle(self, pool: &PgPool, config: &Config) -> Result<StackString, Error> {
        let watchlist = get_watchlist_shows_db_map(pool).await?;
        let pi = ParseImdb::with_pool(config, pool)?;
        let body = pi.parse_imdb_http_worker(&self.into(), &watchlist).await?;
        Ok(body)
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
    pub async fn handle(&self, pool: &PgPool, config: &Config) -> Result<(), Error> {
        let mq = MovieQueueDB::with_pool(config, pool);
        let mc = MovieCollection::with_pool(config, pool)?;
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
    pub async fn handle(&self, pool: &PgPool, config: &Config) -> Result<(), Error> {
        let mc = MovieCollection::with_pool(config, pool)?;
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
        match &self.action {
            TraktActions::Add => {
                trakt.init().await;
                if let Some(show) = trakt.get_watchlist_shows().await?.get(&self.imdb_url) {
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
