use anyhow::{format_err, Error};
use chrono::{DateTime, Utc};
use futures::future::try_join_all;
use log::debug;
use postgres_query::FromSqlRow;
use serde::{Deserialize, Serialize};
use stack_string::StackString;
use std::{fmt, path::Path};

use crate::{config::Config, movie_collection::MovieCollection, pgpool::PgPool};

use crate::utils::{option_string_wrapper, parse_file_stem};

#[derive(Default, Serialize)]
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
            option_string_wrapper(&self.eplink),
        )
    }
}

#[derive(Clone)]
pub struct MovieQueueDB {
    pool: PgPool,
}

impl Default for MovieQueueDB {
    fn default() -> Self {
        Self::new()
    }
}

impl MovieQueueDB {
    pub fn new() -> Self {
        let config = Config::with_config().expect("Init config failed");
        Self {
            pool: PgPool::new(&config.pgurl),
        }
    }

    pub fn with_pool(pool: &PgPool) -> Self {
        Self { pool: pool.clone() }
    }

    pub async fn remove_from_queue_by_idx(&self, idx: i32) -> Result<(), Error> {
        let mut conn = self.pool.get().await?;
        let tran = conn.transaction().await?;

        let query = r#"SELECT max(idx) FROM movie_queue"#;
        let max_idx: i32 = tran
            .query(query, &[])
            .await?
            .get(0)
            .map_or(-1, |r| r.get(0));
        if idx > max_idx || idx < 0 {
            return Ok(());
        }
        let diff = max_idx - idx;

        let query =
            postgres_query::query!(r#"DELETE FROM movie_queue WHERE idx = $idx"#, idx = idx);
        tran.execute(query.sql(), query.parameters()).await?;

        let query = postgres_query::query!(
            r#"
                UPDATE movie_queue
                SET idx = idx + $diff, last_modified = now()
                WHERE idx > $idx
            "#,
            diff = diff,
            idx = idx
        );
        tran.execute(query.sql(), query.parameters()).await?;

        let query = postgres_query::query!(
            r#"
                UPDATE movie_queue
                SET idx = idx - $diff - 1, last_modified = now()
                WHERE idx > $idx
            "#,
            diff = diff,
            idx = idx
        );
        tran.execute(query.sql(), query.parameters()).await?;

        tran.commit().await.map_err(Into::into)
    }

    pub async fn remove_from_queue_by_collection_idx(
        &self,
        collection_idx: i32,
    ) -> Result<(), Error> {
        let query = postgres_query::query!(
            r#"SELECT idx FROM movie_queue WHERE collection_idx=$idx"#,
            idx = collection_idx
        );
        if let Some(row) = self
            .pool
            .get()
            .await?
            .query(query.sql(), query.parameters())
            .await?
            .get(0)
        {
            let idx = row.try_get("idx")?;
            self.remove_from_queue_by_idx(idx).await?;
        }
        Ok(())
    }

    pub async fn remove_from_queue_by_path(&self, path: &str) -> Result<(), Error> {
        let mc = MovieCollection::with_pool(&self.pool)?;
        if let Some(collection_idx) = mc.get_collection_index(&path).await? {
            self.remove_from_queue_by_collection_idx(collection_idx)
                .await
        } else {
            Ok(())
        }
    }

    pub async fn insert_into_queue(&self, idx: i32, path: &str) -> Result<(), Error> {
        if !Path::new(&path).exists() {
            return Err(format_err!("File doesn't exist"));
        }
        let mc = MovieCollection::with_pool(&self.pool)?;
        let collection_idx = if let Some(i) = mc.get_collection_index(&path).await? {
            i
        } else {
            mc.insert_into_collection(&path).await?;
            mc.get_collection_index(&path)
                .await?
                .ok_or_else(|| format_err!("Path not found"))?
        };

        self.insert_into_queue_by_collection_idx(idx, collection_idx)
            .await
    }

    pub async fn insert_into_queue_by_collection_idx(
        &self,
        idx: i32,
        collection_idx: i32,
    ) -> Result<(), Error> {
        let query = postgres_query::query!(
            r#"SELECT idx FROM movie_queue WHERE collection_idx = $idx"#,
            idx = collection_idx
        );
        let current_idx: i32 = self
            .pool
            .get()
            .await?
            .query(query.sql(), query.parameters())
            .await?
            .iter()
            .last()
            .map_or(Ok(-1), |row| row.try_get("idx"))?;

        if current_idx != -1 {
            self.remove_from_queue_by_idx(current_idx).await?;
        }

        let mut conn = self.pool.get().await?;
        let tran = conn.transaction().await?;

        let query = r#"SELECT max(idx) FROM movie_queue"#;
        let max_idx: i32 = tran
            .query(query, &[])
            .await?
            .get(0)
            .map_or(-1, |r| r.get(0));
        let diff = max_idx - idx + 2;
        debug!("{} {} {}", max_idx, idx, diff);

        let query = postgres_query::query!(
            r#"
                UPDATE movie_queue
                SET idx = idx + $diff, last_modified = now()
                WHERE idx >= $idx
            "#,
            diff = diff,
            idx = idx
        );
        tran.execute(query.sql(), query.parameters()).await?;

        let query = postgres_query::query!(
            r#"
                INSERT INTO movie_queue (idx, collection_idx, last_modified)
                VALUES ($idx, $collection_idx, now())
            "#,
            idx = idx,
            collection_idx = collection_idx
        );
        tran.execute(query.sql(), query.parameters()).await?;

        let query = postgres_query::query!(
            r#"
                UPDATE movie_queue
                SET idx = idx - $diff + 1, last_modified = now()
                WHERE idx > $idx
            "#,
            diff = diff,
            idx = idx
        );
        tran.execute(query.sql(), query.parameters()).await?;

        tran.commit().await.map_err(Into::into)
    }

    pub async fn get_max_queue_index(&self) -> Result<i32, Error> {
        let query = r#"SELECT max(idx) FROM movie_queue"#;
        if let Some(row) = self.pool.get().await?.query(query, &[]).await?.get(0) {
            let max_idx: i32 = row.try_get(0)?;
            Ok(max_idx)
        } else {
            Ok(-1)
        }
    }

    pub async fn print_movie_queue(
        &self,
        patterns: &[&str],
    ) -> Result<Vec<MovieQueueResult>, Error> {
        #[derive(FromSqlRow)]
        struct PrintMovieQueue {
            idx: i32,
            path: StackString,
            link: Option<StackString>,
            istv: Option<bool>,
        }

        let query = postgres_query::query_dyn!(&format!(
            r#"
                SELECT a.idx, b.path, c.link, c.istv
                FROM movie_queue a
                JOIN movie_collection b ON a.collection_idx = b.idx
                LEFT JOIN imdb_ratings c ON b.show_id = c.index
                {}
                ORDER BY a.idx
            "#,
            if patterns.is_empty() {
                "".to_string()
            } else {
                let constraints: Vec<_> = patterns
                    .iter()
                    .map(|p| format!("b.path like '%{}%'", p))
                    .collect();
                format!("WHERE {}", constraints.join(" OR "))
            }
        ),)?;

        let results: Result<Vec<_>, Error> = self
            .pool
            .get()
            .await?
            .query(query.sql(), &[])
            .await?
            .iter()
            .map(|row| {
                let row = PrintMovieQueue::from_row(row)?;
                Ok(MovieQueueResult {
                    idx: row.idx,
                    path: row.path,
                    link: row.link,
                    istv: row.istv.unwrap_or(false),
                    ..MovieQueueResult::default()
                })
            })
            .collect();

        let futures = results?.into_iter().map(|mut result| async {
            if result.istv {
                let file_stem = Path::new(result.path.as_str())
                    .file_stem()
                    .unwrap()
                    .to_string_lossy();
                let (show, season, episode) = parse_file_stem(&file_stem);
                let query = postgres_query::query!(
                    r#"
                            SELECT epurl
                            FROM imdb_episodes
                            WHERE show = $show AND season = $season AND episode = $episode
                        "#,
                    show = show,
                    season = season,
                    episode = episode
                );
                if let Some(row) = self
                    .pool
                    .get()
                    .await?
                    .query(query.sql(), query.parameters())
                    .await?
                    .get(0)
                {
                    let epurl: String = row.try_get("epurl")?;
                    result.eplink = Some(epurl.into());
                    result.show = Some(show.to_string().into());
                    result.season = Some(season);
                    result.episode = Some(episode);
                }
            }
            Ok(result)
        });
        let results: Result<Vec<_>, Error> = try_join_all(futures).await;
        let mut results = results?;

        results.sort_by_key(|r| r.idx);
        Ok(results)
    }

    pub async fn get_queue_after_timestamp(
        &self,
        timestamp: DateTime<Utc>,
    ) -> Result<Vec<MovieQueueRow>, Error> {
        let query = postgres_query::query!(
            r#"
                SELECT a.idx, a.collection_idx, b.path, b.show
                FROM movie_queue a
                JOIN movie_collection b ON a.collection_idx = b.idx
                WHERE a.last_modified >= $timestamp
            "#,
            timestamp = timestamp
        );
        self.pool
            .get()
            .await?
            .query(query.sql(), query.parameters())
            .await?
            .iter()
            .map(|row| MovieQueueRow::from_row(row).map_err(Into::into))
            .collect()
    }
}

#[derive(Default, Debug, Serialize, Deserialize, FromSqlRow)]
pub struct MovieQueueRow {
    pub idx: i32,
    pub collection_idx: i32,
    pub path: StackString,
    pub show: StackString,
}
