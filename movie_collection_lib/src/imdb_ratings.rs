use anyhow::Error;
use chrono::{DateTime, Utc};
use log::debug;
use postgres_query::{FromSqlRow, Parameter};
use serde::{Deserialize, Serialize};
use stack_string::StackString;
use std::fmt;

use crate::{pgpool::PgPool, tv_show_source::TvShowSource, utils::option_string_wrapper};

#[derive(Default, Clone, Debug, Serialize, Deserialize, FromSqlRow)]
pub struct ImdbRatings {
    pub index: i32,
    pub show: StackString,
    pub title: Option<StackString>,
    pub link: StackString,
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
            option_string_wrapper(self.title.as_ref()),
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
    pub async fn insert_show(&self, pool: &PgPool) -> Result<(), Error> {
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
        pool.get()
            .await?
            .execute(query.sql(), query.parameters())
            .await
            .map(|_| ())
            .map_err(Into::into)
    }

    pub async fn update_show(&self, pool: &PgPool) -> Result<(), Error> {
        let mut bindings = Vec::new();
        let query = format!(
            r#"
                UPDATE imdb_ratings
                SET last_modified=now(){}{}{}{}
                WHERE show=$show
            "#,
            if let Some(title) = &self.title {
                bindings.push(("title", title as Parameter));
                ",title=$title"
            } else {
                ""
            },
            if let Some(rating) = &self.rating {
                bindings.push(("rating", rating as Parameter));
                ",rating=$rating"
            } else {
                ""
            },
            if let Some(istv) = &self.istv {
                bindings.push(("istv", istv as Parameter));
                ",istv=$istv"
            } else {
                ""
            },
            if let Some(source) = &self.source {
                bindings.push(("source", source as Parameter));
                ",source=$source"
            } else {
                ""
            }
        );
        let query = postgres_query::query_dyn!(&query, show = self.show, ..bindings)?;
        pool.get()
            .await?
            .execute(query.sql(), query.parameters())
            .await
            .map(|_| ())
            .map_err(Into::into)
    }

    pub async fn get_show_by_link(link: &str, pool: &PgPool) -> Result<Option<Self>, Error> {
        let query = postgres_query::query!(
            r#"
                SELECT index, show, title, link, rating, istv, source
                FROM imdb_ratings
                WHERE (link = $link OR show = $link)
            "#,
            link = link
        );
        if let Some(row) = pool
            .get()
            .await?
            .query(query.sql(), query.parameters())
            .await?
            .get(0)
        {
            Ok(Some(Self::from_row(row)?))
        } else {
            Ok(None)
        }
    }

    pub async fn get_shows_after_timestamp(
        timestamp: DateTime<Utc>,
        pool: &PgPool,
    ) -> Result<Vec<Self>, Error> {
        let query = postgres_query::query!(
            r#"
                SELECT index, show, title, link, rating, istv, source
                FROM imdb_ratings
                WHERE last_modified >= $timestamp
            "#,
            timestamp = timestamp
        );
        pool.get()
            .await?
            .query(query.sql(), query.parameters())
            .await?
            .iter()
            .map(|row| Ok(Self::from_row(row)?))
            .collect()
    }

    pub fn get_string_vec(&self) -> Vec<StackString> {
        vec![
            self.show.clone(),
            option_string_wrapper(self.title.as_ref()).into(),
            self.link.clone(),
            self.rating.unwrap_or(-1.0).to_string().into(),
            self.istv.unwrap_or(false).to_string().into(),
            self.source
                .as_ref()
                .map_or_else(|| "".to_string(), ToString::to_string)
                .into(),
        ]
    }
}
