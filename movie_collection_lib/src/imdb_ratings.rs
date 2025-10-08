use anyhow::Error;
use futures::Stream;
use log::debug;
use postgres_query::{query, query_dyn, Error as PqError, FromSqlRow, Parameter, Query};
use serde::{Deserialize, Serialize};
use slug::slugify;
use smallvec::{smallvec, SmallVec};
use stack_string::{format_sstr, StackString};
use std::{convert::TryInto, fmt};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{
    imdb_utils::ImdbConnection, pgpool::PgPool, tv_show_source::TvShowSource,
    utils::option_string_wrapper,
};

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
    pub async fn insert_show(&self, pool: &PgPool) -> Result<u64, Error> {
        let source = self.source.as_ref().map(StackString::from_display);
        let query = query!(
            r#"
                INSERT INTO imdb_ratings
                (show, title, link, rating, istv, source, index)
                VALUES
                ($show, $title, $link, $rating, $istv, $source, $index)
            "#,
            show = self.show,
            title = self.title,
            link = self.link,
            rating = self.rating,
            istv = self.istv,
            source = source,
            index = self.index,
        );
        debug!("{self:?}",);
        let conn = pool.get().await?;
        query.execute(&conn).await.map_err(Into::into)
    }

    /// # Errors
    /// Returns error if db query fails
    pub async fn update_show(&self, pool: &PgPool) -> Result<u64, Error> {
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
            r"
                UPDATE imdb_ratings
                SET {}
                WHERE show=$show
            ",
            updates.join(","),
        );
        let query = query_dyn!(&query, show = self.show, ..bindings)?;
        let conn = pool.get().await?;
        query.execute(&conn).await.map_err(Into::into)
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

    fn get_imdb_ratings_query<'a>(
        select_str: &'a str,
        order_str: &'a str,
        timestamp: Option<&'a OffsetDateTime>,
        offset: Option<usize>,
        limit: Option<usize>,
    ) -> Result<Query<'a>, PqError> {
        let mut constraints = Vec::new();
        let mut query_bindings = Vec::new();
        if let Some(timestamp) = timestamp {
            constraints.push("last_modified >= $timestamp");
            query_bindings.push(("timestamp", timestamp as Parameter));
        }
        let where_str = if constraints.is_empty() {
            "".into()
        } else {
            format_sstr!("WHERE {}", constraints.join(" AND "))
        };
        let mut query = format_sstr!(
            r"
                SELECT {select_str}
                FROM imdb_ratings
                {where_str}
                {order_str}
            "
        );
        if let Some(offset) = &offset {
            query.push_str(&format_sstr!(" OFFSET {offset}"));
        }
        if let Some(limit) = &limit {
            query.push_str(&format_sstr!(" LIMIT {limit}"));
        }
        query_bindings.shrink_to_fit();
        debug!("query:\n{query}",);
        query_dyn!(&query, ..query_bindings)
    }

    /// # Errors
    /// Returns error if db query fails
    pub async fn get_shows_after_timestamp(
        pool: &PgPool,
        timestamp: Option<OffsetDateTime>,
        offset: Option<usize>,
        limit: Option<usize>,
    ) -> Result<impl Stream<Item = Result<Self, PqError>>, Error> {
        let query =
            Self::get_imdb_ratings_query("*", "ORDER BY show", timestamp.as_ref(), offset, limit)?;
        let conn = pool.get().await?;
        query.fetch_streaming(&conn).await.map_err(Into::into)
    }

    /// # Errors
    /// Returns error if db query fails
    pub async fn get_total(
        pool: &PgPool,
        timestamp: Option<OffsetDateTime>,
    ) -> Result<usize, Error> {
        #[derive(FromSqlRow)]
        struct Count {
            count: i64,
        }

        let query = Self::get_imdb_ratings_query("count(*)", "", timestamp.as_ref(), None, None)?;
        let conn = pool.get().await?;
        let count: Count = query.fetch_one(&conn).await?;

        Ok(count.count.try_into()?)
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

    /// # Errors
    /// Returns error if db query fails
    pub async fn get_shows_in_trakt_not_recorded_in_ratings(
        pool: &PgPool,
    ) -> Result<Vec<StackString>, Error> {
        let query = query!(
            r#"
                SELECT twm.link
                FROM trakt_watched_movies twm
                LEFT JOIN imdb_ratings ir ON ir.link = twm.link
                WHERE ir.show IS NULL
            "#
        );
        let conn = pool.get().await?;
        let rows: Vec<_> = query.fetch(&conn).await?;
        Ok(rows.into_iter().map(|(x,)| x).collect())
    }

    /// # Errors
    /// Returns error if db query fails
    pub async fn get_shows_in_trakt_not_in_episodes(
        pool: &PgPool,
    ) -> Result<Vec<StackString>, Error> {
        let query = query!(
            r#"
                SELECT distinct twe.link
                FROM trakt_watched_episodes twe
                LEFT JOIN imdb_ratings ir ON ir.link = twe.link
                WHERE ir.show IS NULL
            "#
        );
        let conn = pool.get().await?;
        let rows: Vec<_> = query.fetch(&conn).await?;
        Ok(rows.into_iter().map(|(x,)| x).collect())
    }

    /// # Errors
    /// Returns error if db query fails
    pub async fn fill_in_missing_ratings_from_trakt(pool: &PgPool) -> Result<(), Error> {
        let imdb_conn = ImdbConnection::new();
        let mut links = ImdbRatings::get_shows_in_trakt_not_recorded_in_ratings(pool).await?;
        links.extend(ImdbRatings::get_shows_in_trakt_not_in_episodes(pool).await?);
        for link in links {
            println!("link {link}");
            for result in imdb_conn.get_suggestions(&link).await? {
                println!("result {}", result.link);
            }
            if let Some(result) = imdb_conn.get_suggestions(&link).await?.first() {
                let title = result.title.clone();
                let show: StackString = slugify(&title).replace('-', "_").into();
                let rating = result.rating;
                let istv = result.title.contains("TV Series")
                    || result.title.contains("TV Mini-Series")
                    || result.title.contains("TV Mini Series");

                println!("show {show} link {link} title {title} rating {rating} istv {istv}");
                ImdbRatings {
                    show,
                    link,
                    title: Some(title),
                    rating: Some(rating),
                    istv: Some(istv),
                    index: Uuid::new_v4(),
                    ..ImdbRatings::default()
                }
                .insert_show(pool)
                .await?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use anyhow::Error;

    use crate::{config::Config, imdb_ratings::ImdbRatings, pgpool::PgPool};

    #[tokio::test]
    #[ignore]
    async fn test_get_shows_in_trakt_not_recorded_in_ratings() -> Result<(), Error> {
        let config = Config::with_config()?;
        let pool = PgPool::new(&config.pgurl)?;

        ImdbRatings::fill_in_missing_ratings_from_trakt(&pool).await?;
        Ok(())
    }

    #[tokio::test]
    #[ignore]
    async fn test_get_shows_in_trakt_not_in_episodes() -> Result<(), Error> {
        let config = Config::with_config()?;
        let pool = PgPool::new(&config.pgurl)?;

        let links = ImdbRatings::get_shows_in_trakt_not_in_episodes(&pool).await?;
        println!("links {links:?}");

        ImdbRatings::fill_in_missing_ratings_from_trakt(&pool).await?;

        Ok(())
    }
}
