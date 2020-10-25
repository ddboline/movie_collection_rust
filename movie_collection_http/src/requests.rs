use anyhow::{format_err, Error};
use serde::{Deserialize, Serialize};
use stack_string::StackString;

use movie_collection_lib::{
    imdb_ratings::ImdbRatings,
    parse_imdb::{ParseImdb, ParseImdbOptions},
    pgpool::PgPool,
    trakt_utils::get_watchlist_shows_db_map,
    tv_show_source::TvShowSource,
};

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
