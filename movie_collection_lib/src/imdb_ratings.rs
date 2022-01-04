use anyhow::Error;
use chrono::{DateTime, Utc};
use log::debug;
use postgres_query::{query, query_dyn, FromSqlRow, Parameter};
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
        let source_str = self.source.as_ref().map_or(StackString::new(), |s| {
            StackString::from_display(s).unwrap()
        });
        write!(
            f,
            "{} {} {} {} {} {} ",
            self.show,
            option_string_wrapper(self.title.as_ref()),
            self.link,
            self.rating.unwrap_or(-1.0),
            self.istv.unwrap_or(false),
            source_str,
        )
    }
}

impl ImdbRatings {
    pub async fn insert_show(&self, pool: &PgPool) -> Result<(), Error> {
        let source = self
            .source
            .as_ref()
            .map(|s| StackString::from_display(s).unwrap());
        let query = query!(
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
        let conn = pool.get().await?;
        query.execute(&conn).await.map(|_| ()).map_err(Into::into)
    }

    pub async fn update_show(&self, pool: &PgPool) -> Result<(), Error> {
        let mut bindings = Vec::new();
        let query = format!(
            r#"
                UPDATE imdb_ratings
                SET last_modified=now(){}{}{}{}
                WHERE show=$show
            "#,
            self.title.as_ref().map_or("", |title| {
                bindings.push(("title", title as Parameter));
                ",title=$title"
            }),
            self.rating.as_ref().map_or("", |rating| {
                bindings.push(("rating", rating as Parameter));
                ",rating=$rating"
            }),
            self.istv.as_ref().map_or("", |istv| {
                bindings.push(("istv", istv as Parameter));
                ",istv=$istv"
            }),
            self.source.as_ref().map_or(",source=null", |source| {
                bindings.push(("source", source as Parameter));
                ",source=$source"
            }),
        );
        let query = query_dyn!(&query, show = self.show, ..bindings)?;
        let conn = pool.get().await?;
        query.execute(&conn).await.map(|_| ()).map_err(Into::into)
    }

    pub async fn get_show_by_link(link: &str, pool: &PgPool) -> Result<Option<Self>, Error> {
        let query = query!(
            r#"
                SELECT index, show, title, link, rating, istv, source
                FROM imdb_ratings
                WHERE (link = $link OR show = $link)
            "#,
            link = link
        );
        let conn = pool.get().await?;
        query.fetch_opt(&conn).await.map_err(Into::into)
    }

    pub async fn get_shows_after_timestamp(
        timestamp: DateTime<Utc>,
        pool: &PgPool,
    ) -> Result<Vec<Self>, Error> {
        let query = query!(
            r#"
                SELECT index, show, title, link, rating, istv, source
                FROM imdb_ratings
                WHERE last_modified >= $timestamp
            "#,
            timestamp = timestamp
        );
        let conn = pool.get().await?;
        query.fetch(&conn).await.map_err(Into::into)
    }

    pub fn get_string_vec(&self) -> Vec<StackString> {
        vec![
            self.show.clone(),
            option_string_wrapper(self.title.as_ref()).into(),
            self.link.clone(),
            StackString::from_display(self.rating.unwrap_or(-1.0)).unwrap(),
            StackString::from_display(self.istv.unwrap_or(false)).unwrap(),
            self.source.as_ref().map_or(StackString::new(), |s| {
                StackString::from_display(s).unwrap()
            }),
        ]
    }
}
