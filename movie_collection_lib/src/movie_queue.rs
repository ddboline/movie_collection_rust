use anyhow::{format_err, Error};
use futures::future::try_join_all;
use itertools::Itertools;
use log::debug;
use postgres_query::{query, query_dyn, FromSqlRow};
use serde::{Deserialize, Serialize};
use stack_string::{format_sstr, StackString};
use std::{fmt, fmt::Write, path::Path};
use stdout_channel::StdoutChannel;
use time::OffsetDateTime;

use crate::{
    config::Config,
    movie_collection::MovieCollection,
    pgpool::PgPool,
    utils::{option_string_wrapper, parse_file_stem}, date_time_wrapper::DateTimeWrapper,
};

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
            option_string_wrapper(self.eplink.as_ref()),
        )
    }
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

        let query = query!(r#"DELETE FROM movie_queue WHERE idx = $idx"#, idx = idx);
        tran.execute(query.sql(), query.parameters()).await?;

        let query = query!(
            r#"
                UPDATE movie_queue
                SET idx = idx + $diff, last_modified = now()
                WHERE idx > $idx
            "#,
            diff = diff,
            idx = idx
        );
        tran.execute(query.sql(), query.parameters()).await?;

        let query = query!(
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

    /// # Errors
    /// Return error if db queries fail
    pub async fn remove_from_queue_by_collection_idx(
        &self,
        collection_idx: i32,
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
        collection_idx: i32,
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
    pub async fn insert_into_queue(&self, idx: i32, path: &str) -> Result<(), Error> {
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
        collection_idx: i32,
    ) -> Result<(), Error> {
        let query = query!(
            r#"SELECT idx FROM movie_queue WHERE collection_idx = $idx"#,
            idx = collection_idx
        );
        let conn = self.pool.get().await?;
        if let Some((current_idx,)) = query.fetch_opt(&conn).await? {
            self.remove_from_queue_by_idx(current_idx).await?;
        }

        let mut conn = self.pool.get().await?;
        let tran = conn.transaction().await?;

        let query = query!("SELECT idx FROM movie_queue WHERE idx=$idx", idx = idx);
        let exists = tran
            .query_opt(query.sql(), query.parameters())
            .await?
            .is_some();

        let query = r#"SELECT max(idx) FROM movie_queue"#;
        let max_idx: i32 = tran
            .query(query, &[])
            .await?
            .get(0)
            .map_or(-1, |r| r.get(0));
        let diff = max_idx - idx + 2;
        debug!("{} {} {}", max_idx, idx, diff);

        if exists {
            let query = query!(
                r#"
                    UPDATE movie_queue
                    SET idx = idx + $diff, last_modified = now()
                    WHERE idx >= $idx
                "#,
                diff = diff,
                idx = idx
            );
            tran.execute(query.sql(), query.parameters()).await?;
        }

        let query = query!(
            r#"
                INSERT INTO movie_queue (idx, collection_idx, last_modified)
                VALUES ($idx, $collection_idx, now())
            "#,
            idx = idx,
            collection_idx = collection_idx
        );
        tran.execute(query.sql(), query.parameters()).await?;

        if exists {
            let query = query!(
                r#"
                    UPDATE movie_queue
                    SET idx = idx - $diff + 1, last_modified = now()
                    WHERE idx > $idx
                "#,
                diff = diff,
                idx = idx
            );
            tran.execute(query.sql(), query.parameters()).await?;
        }

        tran.commit().await.map_err(Into::into)
    }

    /// # Errors
    /// Return error if db queries fail
    pub async fn get_max_queue_index(&self) -> Result<i32, Error> {
        let query = r#"SELECT max(idx) FROM movie_queue"#;
        if let Some(row) = self.pool.get().await?.query(query, &[]).await?.get(0) {
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
            write!(where_str, "WHERE {}", constraints)?;
        }
        let query = query_dyn!(&format_sstr!(
            r#"
                SELECT a.idx, b.path, c.link, c.istv
                FROM movie_queue a
                JOIN movie_collection b ON a.collection_idx = b.idx
                LEFT JOIN imdb_ratings c ON b.show_id = c.index
                {where_str}
                ORDER BY a.idx
            "#
        ),)?;
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
        let mut results = results?;

        results.sort_by_key(|r| -r.idx);
        Ok(results)
    }

    /// # Errors
    /// Return error if db queries fail
    pub async fn get_queue_after_timestamp(
        &self,
        timestamp: OffsetDateTime,
    ) -> Result<Vec<MovieQueueRow>, Error> {
        let query = query!(
            r#"
                SELECT a.idx, a.collection_idx, b.path, b.show, a.last_modified
                FROM movie_queue a
                JOIN movie_collection b ON a.collection_idx = b.idx
                WHERE a.last_modified >= $timestamp
            "#,
            timestamp = timestamp
        );
        let conn = self.pool.get().await?;
        query.fetch(&conn).await.map_err(Into::into)
    }
}

#[derive(Default, Debug, Serialize, Deserialize, FromSqlRow)]
pub struct MovieQueueRow {
    pub idx: i32,
    pub collection_idx: i32,
    pub path: StackString,
    pub show: StackString,
    pub last_modified: Option<DateTimeWrapper>,
}
