use anyhow::{format_err, Error};
use futures::future::try_join_all;
use itertools::Itertools;
use log::debug;
use postgres_query::{query, query_dyn, Error as PqError, FromSqlRow, Parameter, Query};
use serde::{Deserialize, Serialize};
use stack_string::{format_sstr, StackString};
use std::{convert::TryInto, fmt, fmt::Write, path::Path};
use stdout_channel::StdoutChannel;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{
    config::Config,
    date_time_wrapper::DateTimeWrapper,
    movie_collection::MovieCollection,
    pgpool::PgPool,
    utils::{option_string_wrapper, parse_file_stem},
};

#[derive(Serialize, Deserialize, Debug, Copy, Clone, PartialEq, Eq)]
pub enum OrderBy {
    #[serde(rename = "asc")]
    Asc,
    #[serde(rename = "desc")]
    Desc,
}

impl OrderBy {
    fn to_str(self) -> &'static str {
        match self {
            Self::Asc => "asc",
            Self::Desc => "desc",
        }
    }
}

impl fmt::Display for OrderBy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_str())
    }
}

#[derive(Default, Serialize, PartialEq, Eq, Clone)]
pub struct MovieQueueResult {
    pub idx: i32,
    pub path: StackString,
    pub link: Option<StackString>,
    pub istv: bool,
    pub show: Option<StackString>,
    pub eplink: Option<StackString>,
    pub season: Option<i32>,
    pub episode: Option<i32>,
}

impl fmt::Display for MovieQueueResult {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "{} {} {}",
            self.idx,
            self.path,
            option_string_wrapper(self.eplink.as_ref()),
        )
    }
}

#[derive(Default, Debug, Serialize, Deserialize, FromSqlRow)]
pub struct MovieQueueRow {
    pub idx: i32,
    pub collection_idx: Uuid,
    pub path: StackString,
    pub show: StackString,
    pub last_modified: Option<DateTimeWrapper>,
}

#[derive(Clone, Default)]
pub struct MovieQueueDB {
    pub config: Config,
    pub pool: PgPool,
    pub stdout: StdoutChannel<StackString>,
}

impl MovieQueueDB {
    #[must_use]
    pub fn new(config: &Config, pool: &PgPool, stdout: &StdoutChannel<StackString>) -> Self {
        Self {
            config: config.clone(),
            pool: pool.clone(),
            stdout: stdout.clone(),
        }
    }

    /// # Errors
    /// Return error if db queries fail
    pub async fn reorder_queue(&self) -> Result<u64, Error> {
        let conn = self.pool.get().await?;

        let query = query!(
            r#"
                CREATE TEMP TABLE temp_movie_queue AS (
                    SELECT idx,
                           last_modified,
                           collection_idx,
                           row_number() OVER (
                            ORDER BY (idx, last_modified)
                           ) AS new_idx
                    FROM movie_queue
                )
            "#
        );
        query.execute(&conn).await?;

        let query = query!(
            r#"
                UPDATE movie_queue
                SET idx = (
                    SELECT new_idx - 1
                    FROM temp_movie_queue
                    WHERE collection_idx = movie_queue.collection_idx
                ),last_modified=now()
            "#,
        );
        query.execute(&conn).await.map_err(Into::into)
    }

    /// # Errors
    /// Return error if db queries fail
    pub async fn remove_from_queue_by_idx(&self, idx: i32) -> Result<u64, Error> {
        let conn = self.pool.get().await?;

        let query = query!(r"DELETE FROM movie_queue WHERE idx = $idx", idx = idx);
        query.execute(&conn).await.map_err(Into::into)
    }

    /// # Errors
    /// Return error if db queries fail
    pub async fn remove_from_queue_by_collection_idx(
        &self,
        collection_idx: Uuid,
    ) -> Result<(), Error> {
        if let Some(idx) = self.get_idx_from_collection_idx(collection_idx).await? {
            self.remove_from_queue_by_idx(idx).await?;
        }
        Ok(())
    }

    /// # Errors
    /// Return error if db queries fail
    pub async fn get_idx_from_collection_idx(
        &self,
        collection_idx: Uuid,
    ) -> Result<Option<i32>, Error> {
        let query = query!(
            "SELECT idx FROM movie_queue WHERE collection_idx=$collection_idx",
            collection_idx = collection_idx,
        );
        let conn = self.pool.get().await?;
        let x: Option<(i32,)> = query.fetch_opt(&conn).await?;
        Ok(x.map(|(x,)| x))
    }

    /// # Errors
    /// Return error if db queries fail
    pub async fn remove_from_queue_by_path(&self, path: &str) -> Result<(), Error> {
        let mc = MovieCollection::new(&self.config, &self.pool, &self.stdout);
        if let Some(collection_idx) = mc.get_collection_index(path).await? {
            self.remove_from_queue_by_collection_idx(collection_idx)
                .await
        } else {
            Ok(())
        }
    }

    /// # Errors
    /// Return error if db queries fail
    pub async fn insert_into_queue(&self, idx: i32, path: &str) -> Result<u64, Error> {
        if !Path::new(path).exists() {
            return Err(format_err!("File doesn't exist"));
        }
        let mc = MovieCollection::new(&self.config, &self.pool, &self.stdout);
        let collection_idx = if let Some(i) = mc.get_collection_index(path).await? {
            i
        } else {
            mc.insert_into_collection(path, true).await?;
            mc.get_collection_index(path)
                .await?
                .ok_or_else(|| format_err!("Path not found"))?
        };

        self.insert_into_queue_by_collection_idx(idx, collection_idx)
            .await
    }

    /// # Errors
    /// Return error if db queries fail
    pub async fn insert_into_queue_by_collection_idx(
        &self,
        idx: i32,
        collection_idx: Uuid,
    ) -> Result<u64, Error> {
        let query = query!(
            r#"
                INSERT INTO movie_queue (idx, collection_idx)
                VALUES ($idx, $collection_idx)
                ON CONFLICT (collection_idx)
                DO UPDATE SET idx=$idx,last_modified=now()
            "#,
            idx = idx,
            collection_idx = collection_idx
        );
        let conn = self.pool.get().await?;
        query.execute(&conn).await.map_err(Into::into)
    }

    /// # Errors
    /// Return error if db queries fail
    pub async fn get_max_queue_index(&self) -> Result<i32, Error> {
        let query = r"SELECT max(idx) FROM movie_queue";
        if let Some(row) = self.pool.get().await?.query(query, &[]).await?.first() {
            let max_idx: i32 = row.try_get(0)?;
            Ok(max_idx)
        } else {
            Ok(-1)
        }
    }

    /// # Errors
    /// Return error if db queries fail
    pub async fn print_movie_queue(
        &self,
        patterns: &[&str],
        offset: Option<usize>,
        limit: Option<usize>,
        order_by: Option<OrderBy>,
    ) -> Result<Vec<MovieQueueResult>, Error> {
        #[derive(FromSqlRow)]
        struct PrintMovieQueue {
            idx: i32,
            path: StackString,
            link: Option<StackString>,
            istv: Option<bool>,
        }
        let constraints = patterns
            .iter()
            .map(|p| format_sstr!("b.path like '%{p}%'"))
            .join(" OR ");
        let mut where_str = StackString::new();
        if !constraints.is_empty() {
            write!(where_str, "WHERE {constraints}")?;
        }
        let mut offset_str = StackString::new();
        if let Some(offset) = offset {
            write!(offset_str, "OFFSET {offset}")?;
        }
        let mut limit_str = StackString::new();
        if let Some(limit) = limit {
            write!(limit_str, "LIMIT {limit}")?;
        }
        let order_by = order_by.unwrap_or(OrderBy::Desc);
        let query = &format_sstr!(
            r#"
                SELECT a.idx, b.path, c.link, c.istv
                FROM movie_queue a
                JOIN movie_collection b ON a.collection_idx = b.idx
                LEFT JOIN imdb_ratings c ON b.show_id = c.index
                {where_str}
                ORDER BY a.idx {order_by}
                {offset_str}
                {limit_str}
            "#
        );
        let query = query_dyn!(query,)?;
        let conn = self.pool.get().await?;
        let results: Vec<PrintMovieQueue> = query.fetch(&conn).await?;

        let futures = results.into_iter().map(|row| async {
            let mut result = MovieQueueResult {
                idx: row.idx,
                path: row.path,
                link: row.link,
                istv: row.istv.unwrap_or(false),
                ..MovieQueueResult::default()
            };

            if result.istv {
                let file_stem = Path::new(result.path.as_str())
                    .file_stem()
                    .ok_or_else(|| format_err!("No file stem"))?
                    .to_string_lossy();
                let (show, season, episode) = parse_file_stem(&file_stem);
                let query = query!(
                    r#"
                            SELECT epurl
                            FROM imdb_episodes
                            WHERE show = $show AND season = $season AND episode = $episode
                        "#,
                    show = show,
                    season = season,
                    episode = episode
                );
                let conn = self.pool.get().await?;
                if let Some((epurl,)) = query.fetch_opt(&conn).await? {
                    let epurl: String = epurl;
                    result.eplink = Some(epurl.into());
                    result.show = Some(show);
                    result.season = Some(season);
                    result.episode = Some(episode);
                }
            }
            Ok(result)
        });
        let results: Result<Vec<_>, Error> = try_join_all(futures).await;
        let results = results?;
        Ok(results)
    }

    fn get_movie_queue_query<'a>(
        select_str: &'a str,
        order_str: &'a str,
        timestamp: &'a Option<OffsetDateTime>,
        offset: Option<usize>,
        limit: Option<usize>,
    ) -> Result<Query<'a>, PqError> {
        let mut constraints = Vec::new();
        let mut query_bindings = Vec::new();
        if let Some(timestamp) = timestamp {
            constraints.push("a.last_modified >= $timestamp");
            query_bindings.push(("timestamp", timestamp as Parameter));
        }
        let where_str = if constraints.is_empty() {
            "".into()
        } else {
            format_sstr!("WHERE {}", constraints.join(" AND "))
        };
        let mut query = format_sstr!(
            r#"
                SELECT {select_str}
                FROM movie_queue a
                JOIN movie_collection b ON a.collection_idx = b.idx
                {where_str}
                {order_str}
            "#,
        );
        if let Some(offset) = &offset {
            query.push_str(&format_sstr!(" OFFSET {offset}"));
        }
        if let Some(limit) = &limit {
            query.push_str(&format_sstr!(" LIMIT {limit}"));
        }
        query_bindings.shrink_to_fit();
        debug!("query:\n{}", query);
        query_dyn!(&query, ..query_bindings)
    }

    /// # Errors
    /// Return error if db queries fail
    pub async fn get_queue_after_timestamp(
        &self,
        timestamp: Option<OffsetDateTime>,
        offset: Option<usize>,
        limit: Option<usize>,
    ) -> Result<Vec<MovieQueueRow>, Error> {
        let query = Self::get_movie_queue_query(
            "a.idx, a.collection_idx, b.path, b.show, a.last_modified",
            "ORDER BY a.idx",
            &timestamp,
            offset,
            limit,
        )?;
        let conn = self.pool.get().await?;
        query.fetch(&conn).await.map_err(Into::into)
    }

    /// # Errors
    /// Return error if db queries fail
    pub async fn get_total(&self, timestamp: Option<OffsetDateTime>) -> Result<usize, Error> {
        #[derive(FromSqlRow)]
        struct Count {
            count: i64,
        }

        let query = Self::get_movie_queue_query("count(*)", "", &timestamp, None, None)?;
        let conn = self.pool.get().await?;
        let count: Count = query.fetch_one(&conn).await?;

        Ok(count.count.try_into()?)
    }
}
