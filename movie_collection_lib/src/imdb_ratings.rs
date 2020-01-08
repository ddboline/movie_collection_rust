use anyhow::Error;
use chrono::{DateTime, Utc};
use log::debug;
use postgres_query::FromSqlRow;
use serde::{Deserialize, Serialize};
use std::fmt;

use crate::pgpool::PgPool;
use crate::tv_show_source::TvShowSource;
use crate::utils::option_string_wrapper;

#[derive(Default, Clone, Debug, Serialize, Deserialize, FromSqlRow)]
pub struct ImdbRatings {
    pub index: i32,
    pub show: String,
    pub title: Option<String>,
    pub link: String,
    pub rating: Option<f64>,
    pub istv: Option<bool>,
    pub source: Option<TvShowSource>,
}

impl fmt::Display for ImdbRatings {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "{} {} {} {} {} {} ",
            self.show,
            option_string_wrapper(&self.title),
            self.link,
            self.rating.unwrap_or(-1.0),
            self.istv.unwrap_or(false),
            self.source
                .as_ref()
                .map_or_else(|| "".to_string(), ToString::to_string),
        )
    }
}

impl ImdbRatings {
    pub fn insert_show(&self, pool: &PgPool) -> Result<(), Error> {
        let source = self.source.as_ref().map(ToString::to_string);
        let query = postgres_query::query!(
            r#"
                INSERT INTO imdb_ratings
                (show, title, link, rating, istv, source, last_modified)
                VALUES
                ($show, $title, $link, $rating, $istv, $source, now())
            "#,
            show = self.show,
            title = self.title,
            link = self.link,
            rating = self.rating,
            istv = self.istv,
            source = source
        );
        debug!("{:?}", self);
        pool.get()?
            .execute(query.sql, &query.parameters)
            .map(|_| ())
            .map_err(Into::into)
    }

    pub fn update_show(&self, pool: &PgPool) -> Result<(), Error> {
        let query = r#"
            UPDATE imdb_ratings
            SET rating=$1,title=$2,last_modified=now()
            WHERE show=$3
        "#;
        pool.get()?
            .execute(query, &[&self.rating, &self.title, &self.show])
            .map(|_| ())
            .map_err(Into::into)
    }

    pub fn get_show_by_link(link: &str, pool: &PgPool) -> Result<Option<Self>, Error> {
        let query = r#"
            SELECT index, show, title, link, rating, istv, source
            FROM imdb_ratings
            WHERE (link = $1 OR show = $1)
        "#;
        if let Some(row) = pool.get()?.query(query, &[&link])?.get(0) {
            Ok(Some(Self::from_row(row)?))
        } else {
            Ok(None)
        }
    }

    pub fn get_shows_after_timestamp(
        timestamp: DateTime<Utc>,
        pool: &PgPool,
    ) -> Result<Vec<Self>, Error> {
        let query = r#"
            SELECT index, show, title, link, rating, istv, source
            FROM imdb_ratings
            WHERE last_modified >= $1
        "#;
        pool.get()?
            .query(query, &[&timestamp])?
            .iter()
            .map(|row| Ok(Self::from_row(row)?))
            .collect()
    }

    pub fn get_string_vec(&self) -> Vec<String> {
        vec![
            self.show.to_string(),
            option_string_wrapper(&self.title).to_string(),
            self.link.to_string(),
            self.rating.unwrap_or(-1.0).to_string(),
            self.istv.unwrap_or(false).to_string(),
            self.source
                .as_ref()
                .map_or_else(|| "".to_string(), ToString::to_string),
        ]
    }
}
