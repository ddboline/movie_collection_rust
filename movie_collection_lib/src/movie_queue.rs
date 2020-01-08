use anyhow::{format_err, Error};
use chrono::{DateTime, Utc};
use log::debug;
use postgres_query::FromSqlRow;
use rayon::iter::{IntoParallelIterator, ParallelIterator};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::Path;

use crate::config::Config;
use crate::movie_collection::MovieCollection;
use crate::pgpool::PgPool;

use crate::utils::{option_string_wrapper, parse_file_stem};

#[derive(Default, Serialize)]
pub struct MovieQueueResult {
    pub idx: i32,
    pub path: String,
    pub link: Option<String>,
    pub istv: bool,
    pub show: Option<String>,
    pub eplink: Option<String>,
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

    pub fn remove_from_queue_by_idx(&self, idx: i32) -> Result<(), Error> {
        let mut conn = self.pool.get()?;
        let mut tran = conn.transaction()?;

        let query = r#"SELECT max(idx) FROM movie_queue"#;
        let max_idx: i32 = tran.query(query, &[])?.get(0).map_or(-1, |r| r.get(0));
        if idx > max_idx || idx < 0 {
            return Ok(());
        }
        let diff = max_idx - idx;

        let query = r#"DELETE FROM movie_queue WHERE idx = $1"#;
        tran.execute(query, &[&idx])?;

        let query = r#"
            UPDATE movie_queue
            SET idx = idx + $1, last_modified = now()
            WHERE idx > $2
        "#;
        tran.execute(query, &[&diff, &idx])?;

        let query = r#"
            UPDATE movie_queue
            SET idx = idx - $1 - 1, last_modified = now()
            WHERE idx > $2
        "#;
        tran.execute(query, &[&diff, &idx])?;

        tran.commit().map_err(Into::into)
    }

    pub fn remove_from_queue_by_collection_idx(&self, collection_idx: i32) -> Result<(), Error> {
        let query = r#"SELECT idx FROM movie_queue WHERE collection_idx=$1"#;
        self.pool
            .get()?
            .query(query, &[&collection_idx])?
            .iter()
            .map(|row| {
                let idx = row.try_get(0)?;
                self.remove_from_queue_by_idx(idx)
            })
            .collect()
    }

    pub fn remove_from_queue_by_path(&self, path: &str) -> Result<(), Error> {
        let mc = MovieCollection::with_pool(&self.pool)?;
        if let Some(collection_idx) = mc.get_collection_index(&path)? {
            self.remove_from_queue_by_collection_idx(collection_idx)
        } else {
            Ok(())
        }
    }

    pub fn insert_into_queue(&self, idx: i32, path: &str) -> Result<(), Error> {
        if !Path::new(&path).exists() {
            return Err(format_err!("File doesn't exist"));
        }
        let mc = MovieCollection::with_pool(&self.pool)?;
        let collection_idx = match mc.get_collection_index(&path)? {
            Some(i) => i,
            None => {
                mc.insert_into_collection(&path)?;
                mc.get_collection_index(&path)?
                    .ok_or_else(|| format_err!("Path not found"))?
            }
        };

        self.insert_into_queue_by_collection_idx(idx, collection_idx)
    }

    pub fn insert_into_queue_by_collection_idx(
        &self,
        idx: i32,
        collection_idx: i32,
    ) -> Result<(), Error> {
        let query = r#"SELECT idx FROM movie_queue WHERE collection_idx = $1"#;
        let current_idx: i32 = self
            .pool
            .get()?
            .query(query, &[&collection_idx])?
            .iter()
            .last()
            .map_or(Ok(-1), |row| row.try_get(0))?;

        if current_idx != -1 {
            self.remove_from_queue_by_idx(current_idx)?;
        }

        let mut conn = self.pool.get()?;
        let mut tran = conn.transaction()?;

        let query = r#"SELECT max(idx) FROM movie_queue"#;
        let max_idx: i32 = tran.query(query, &[])?.get(0).map_or(-1, |r| r.get(0));
        let diff = max_idx - idx + 2;
        debug!("{} {} {}", max_idx, idx, diff);

        let query = r#"
            UPDATE movie_queue
            SET idx = idx + $1, last_modified = now()
            WHERE idx >= $2
        "#;
        tran.execute(query, &[&diff, &idx])?;

        let query = r#"
            INSERT INTO movie_queue (idx, collection_idx, last_modified)
            VALUES ($1, $2, now())
        "#;
        tran.execute(query, &[&idx, &collection_idx])?;

        let query = r#"
            UPDATE movie_queue
            SET idx = idx - $1 + 1, last_modified = now()
            WHERE idx > $2
        "#;
        tran.execute(query, &[&diff, &idx])?;

        tran.commit().map_err(Into::into)
    }

    pub fn get_max_queue_index(&self) -> Result<i32, Error> {
        let query = r#"SELECT max(idx) FROM movie_queue"#;
        if let Some(row) = self.pool.get()?.query(query, &[])?.get(0) {
            let max_idx: i32 = row.try_get(0)?;
            Ok(max_idx)
        } else {
            Ok(-1)
        }
    }

    pub fn print_movie_queue(&self, patterns: &[&str]) -> Result<Vec<MovieQueueResult>, Error> {
        let query = r#"
            SELECT a.idx, b.path, c.link, c.istv
            FROM movie_queue a
            JOIN movie_collection b ON a.collection_idx = b.idx
            LEFT JOIN imdb_ratings c ON b.show_id = c.index
        "#;
        let query = if patterns.is_empty() {
            query.to_string()
        } else {
            let constraints: Vec<_> = patterns
                .iter()
                .map(|p| format!("b.path like '%{}%'", p))
                .collect();
            format!("{} WHERE {}", query, constraints.join(" OR "))
        };
        let results: Result<Vec<_>, Error> = self
            .pool
            .get()?
            .query(query.as_str(), &[])?
            .iter()
            .map(|row| {
                let idx: i32 = row.try_get(0)?;
                let path: String = row.try_get(1)?;
                let link: Option<String> = row.try_get(2)?;
                let istv: Option<bool> = row.try_get(3)?;
                Ok(MovieQueueResult {
                    idx,
                    path,
                    link,
                    istv: istv.unwrap_or(false),
                    ..MovieQueueResult::default()
                })
            })
            .collect();

        let results: Result<Vec<_>, Error> = results?
            .into_par_iter()
            .map(|mut result| {
                if result.istv {
                    let file_stem = Path::new(&result.path)
                        .file_stem()
                        .unwrap()
                        .to_string_lossy();
                    let (show, season, episode) = parse_file_stem(&file_stem);
                    let query = r#"
                        SELECT epurl
                        FROM imdb_episodes
                        WHERE show = $1 AND season = $2 AND episode = $3
                    "#;
                    for row in &self.pool.get()?.query(query, &[&show, &season, &episode])? {
                        let epurl: String = row.try_get(0)?;
                        result.eplink = Some(epurl);
                        result.show = Some(show.to_string());
                        result.season = Some(season);
                        result.episode = Some(episode);
                    }
                }
                Ok(result)
            })
            .collect();
        let mut results = results?;
        results.sort_by_key(|r| r.idx);
        Ok(results)
    }

    pub fn get_queue_after_timestamp(
        &self,
        timestamp: DateTime<Utc>,
    ) -> Result<Vec<MovieQueueRow>, Error> {
        let query = r#"
            SELECT a.idx, a.collection_idx, b.path, b.show
            FROM movie_queue a
            JOIN movie_collection b ON a.collection_idx = b.idx
            WHERE a.last_modified >= $1
        "#;
        self.pool
            .get()?
            .query(query, &[&timestamp])?
            .iter()
            .map(|row| MovieQueueRow::from_row(row).map_err(Into::into))
            .collect()
    }
}

#[derive(Default, Debug, Serialize, Deserialize, FromSqlRow)]
pub struct MovieQueueRow {
    pub idx: i32,
    pub collection_idx: i32,
    pub path: String,
    pub show: String,
}
