use anyhow::Error;
use futures::Stream;
use log::debug;
use postgres_query::{query, query_dyn, Error as PqError, FromSqlRow, Parameter};
use serde::{Deserialize, Serialize};
use smallvec::{smallvec, SmallVec};
use stack_string::{format_sstr, StackString};
use std::fmt;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{pgpool::PgPool, tv_show_source::TvShowSource, utils::option_string_wrapper};

#[derive(Default, Clone, Debug, Serialize, Deserialize, FromSqlRow, PartialEq)]
pub struct ImdbRatings {
    pub index: Uuid,
    pub show: StackString,
    pub title: Option<StackString>,
    pub link: StackString,
    pub rating: Option<f64>,
    pub istv: Option<bool>,
    pub source: Option<TvShowSource>,
}

impl fmt::Display for ImdbRatings {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let source_str = self
            .source
            .as_ref()
            .map_or(StackString::new(), StackString::from_display);
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
    /// # Errors
    /// Returns error if db query fails
    pub async fn insert_show(&self, pool: &PgPool) -> Result<(), Error> {
        let source = self.source.as_ref().map(StackString::from_display);
        let query = query!(
            r#"
                INSERT INTO imdb_ratings
                (show, title, link, rating, istv, source, index, last_modified)
                VALUES
                ($show, $title, $link, $rating, $istv, $source, $index, now())
            "#,
            show = self.show,
            title = self.title,
            link = self.link,
            rating = self.rating,
            istv = self.istv,
            source = source,
            index = self.index,
        );
        debug!("{:?}", self);
        let conn = pool.get().await?;
        query.execute(&conn).await.map(|_| ()).map_err(Into::into)
    }

    /// # Errors
    /// Returns error if db query fails
    pub async fn update_show(&self, pool: &PgPool) -> Result<(), Error> {
        let mut bindings = Vec::new();
        let mut updates: SmallVec<[&'static str; 6]> = smallvec!["last_modified=now()"];
        if let Some(title) = &self.title {
            bindings.push(("title", title as Parameter));
            updates.push("title=$title");
        }
        if let Some(rating) = &self.rating {
            bindings.push(("rating", rating as Parameter));
            updates.push("rating=$rating");
        }
        if let Some(istv) = &self.istv {
            bindings.push(("istv", istv as Parameter));
            updates.push("istv=$istv");
        }
        if let Some(source) = &self.source {
            bindings.push(("source", source as Parameter));
            updates.push("source=$source");
        } else {
            updates.push("source=null");
        }

        let query = format_sstr!(
            r#"
                UPDATE imdb_ratings
                SET {}
                WHERE show=$show
            "#,
            updates.join(","),
        );
        let query = query_dyn!(&query, show = self.show, ..bindings)?;
        let conn = pool.get().await?;
        query.execute(&conn).await.map(|_| ()).map_err(Into::into)
    }

    /// # Errors
    /// Returns error if db query fails
    pub async fn get_show_by_link(link: &str, pool: &PgPool) -> Result<Option<Self>, Error> {
        let query = query!(
            r#"
                SELECT *
                FROM imdb_ratings
                WHERE (link = $link OR show = $link)
            "#,
            link = link
        );
        let conn = pool.get().await?;
        query.fetch_opt(&conn).await.map_err(Into::into)
    }

    /// # Errors
    /// Returns error if db query fails
    pub async fn get_shows_after_timestamp(
        timestamp: OffsetDateTime,
        pool: &PgPool,
    ) -> Result<impl Stream<Item = Result<Self, PqError>>, Error> {
        let query = query!(
            r#"
                SELECT *
                FROM imdb_ratings
                WHERE last_modified >= $timestamp
            "#,
            timestamp = timestamp
        );
        let conn = pool.get().await?;
        query.fetch_streaming(&conn).await.map_err(Into::into)
    }

    #[must_use]
    pub fn get_string_vec(&self) -> Vec<StackString> {
        vec![
            self.show.clone(),
            option_string_wrapper(self.title.as_ref()).into(),
            self.link.clone(),
            StackString::from_display(self.rating.unwrap_or(-1.0)),
            StackString::from_display(self.istv.unwrap_or(false)),
            self.source
                .as_ref()
                .map_or(StackString::new(), StackString::from_display),
        ]
    }
}
