use failure::{err_msg, Error};
use rayon::prelude::*;
use std::fmt;
use std::path::Path;

use crate::common::config::Config;
use crate::common::movie_collection::{MovieCollection, MovieCollectionDB};
use crate::common::pgpool::PgPool;
use crate::common::utils::{map_result_vec, option_string_wrapper, parse_file_stem};

#[derive(Default)]
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
    fn default() -> MovieQueueDB {
        MovieQueueDB::new()
    }
}

impl MovieQueueDB {
    pub fn new() -> MovieQueueDB {
        let config = Config::with_config();
        let pgurl = config.pgurl;
        MovieQueueDB {
            pool: PgPool::new(&pgurl),
        }
    }

    pub fn with_pool(pool: &PgPool) -> MovieQueueDB {
        MovieQueueDB { pool: pool.clone() }
    }

    pub fn remove_from_queue_by_idx(&self, idx: i32) -> Result<(), Error> {
        let conn = self.pool.get()?;
        let tran = conn.transaction()?;

        let query = r#"SELECT max(idx) FROM movie_queue"#;
        let max_idx: i32 = tran
            .query(query, &[])?
            .iter()
            .nth(0)
            .map(|r| r.get(0))
            .unwrap_or(-1);
        if idx > max_idx || idx < 0 {
            return Ok(());
        }
        let diff = max_idx - idx;

        let query = r#"DELETE FROM movie_queue WHERE idx = $1"#;
        tran.execute(query, &[&idx])?;

        let query = r#"UPDATE movie_queue SET idx = idx + $1 WHERE idx > $2"#;
        tran.execute(query, &[&diff, &idx])?;

        let query = r#"UPDATE movie_queue SET idx = idx - $1 - 1 WHERE idx > $2"#;
        tran.execute(query, &[&diff, &idx])?;

        tran.commit().map_err(err_msg)
    }

    pub fn remove_from_queue_by_collection_idx(&self, collection_idx: i32) -> Result<(), Error> {
        let query = r#"SELECT idx FROM movie_queue WHERE collection_idx=$1"#;
        for row in self.pool.get()?.query(query, &[&collection_idx])?.iter() {
            let idx = row.get(0);
            self.remove_from_queue_by_idx(idx)?;
        }
        Ok(())
    }

    pub fn remove_from_queue_by_path(&self, path: &str) -> Result<(), Error> {
        let mc = MovieCollectionDB::with_pool(&self.pool);
        if let Some(collection_idx) = mc.get_collection_index(&path)? {
            self.remove_from_queue_by_collection_idx(collection_idx)
        } else {
            Ok(())
        }
    }

    pub fn insert_into_queue(&self, idx: i32, path: &str) -> Result<(), Error> {
        if !Path::new(&path).exists() {
            return Err(err_msg("File doesn't exist"));
        }
        let mc = MovieCollectionDB::with_pool(&self.pool);
        let collection_idx = match mc.get_collection_index(&path)? {
            Some(i) => i,
            None => {
                mc.insert_into_collection(&path)?;
                mc.get_collection_index(&path)?
                    .ok_or_else(|| err_msg("Path not found"))?
            }
        };

        let query = r#"SELECT idx FROM movie_queue WHERE collection_idx = $1"#;
        let current_idx: i32 = self
            .pool
            .get()?
            .query(query, &[&collection_idx])?
            .iter()
            .last()
            .map(|row| row.get(0))
            .unwrap_or(-1);

        if current_idx != -1 {
            self.remove_from_queue_by_idx(current_idx)?;
        }

        let conn = self.pool.get()?;
        let tran = conn.transaction()?;

        let query = r#"SELECT max(idx) FROM movie_queue"#;
        let max_idx: i32 = tran
            .query(query, &[])?
            .iter()
            .nth(0)
            .map(|r| r.get(0))
            .unwrap_or(-1);
        let diff = max_idx - idx + 2;
        println!("{} {} {}", max_idx, idx, diff);

        let query = r#"UPDATE movie_queue SET idx = idx + $1 WHERE idx >= $2"#;
        tran.execute(query, &[&diff, &idx])?;

        let query = r#"INSERT INTO movie_queue (idx, collection_idx) VALUES ($1, $2)"#;
        tran.execute(query, &[&idx, &collection_idx])?;

        let query = r#"UPDATE movie_queue SET idx = idx - $1 + 1 WHERE idx > $2"#;
        tran.execute(query, &[&diff, &idx])?;

        tran.commit().map_err(err_msg)
    }

    pub fn get_max_queue_index(&self) -> Result<i32, Error> {
        let query = r#"SELECT max(idx) FROM movie_queue"#;
        if let Some(row) = self.pool.get()?.query(query, &[])?.iter().nth(0) {
            let max_idx: i32 = row.get(0);
            Ok(max_idx)
        } else {
            Ok(-1)
        }
    }

    pub fn print_movie_queue(&self, patterns: &[String]) -> Result<Vec<MovieQueueResult>, Error> {
        let query = r#"
            SELECT a.idx, b.path, c.link, c.istv
            FROM movie_queue a
            JOIN movie_collection b ON a.collection_idx = b.idx
            LEFT JOIN imdb_ratings c ON b.show_id = c.index
        "#;
        let query = if !patterns.is_empty() {
            let constraints: Vec<_> = patterns
                .iter()
                .map(|p| format!("b.path like '%{}%'", p))
                .collect();
            format!("{} WHERE {}", query, constraints.join(" OR "))
        } else {
            query.to_string()
        };
        let results: Vec<_> = self
            .pool
            .get()?
            .query(&query, &[])?
            .iter()
            .map(|row| {
                let idx: i32 = row.get(0);
                let path: String = row.get(1);
                let link: Option<String> = row.get(2);
                let istv: Option<bool> = row.get(3);
                MovieQueueResult {
                    idx,
                    path,
                    link,
                    istv: istv.unwrap_or(false),
                    ..Default::default()
                }
            })
            .collect();

        let results: Vec<Result<_, Error>> = results
            .into_par_iter()
            .map(|mut result| {
                if result.istv {
                    let file_stem = Path::new(&result.path)
                        .file_stem()
                        .unwrap()
                        .to_str()
                        .unwrap();
                    let (show, season, episode) = parse_file_stem(&file_stem);
                    let query = r#"
                        SELECT epurl
                        FROM imdb_episodes
                        WHERE show = $1 AND season = $2 AND episode = $3
                    "#;
                    for row in self
                        .pool
                        .get()?
                        .query(query, &[&show, &season, &episode])?
                        .iter()
                    {
                        let epurl: String = row.get(0);
                        result.eplink = Some(epurl);
                        result.show = Some(show.clone());
                        result.season = Some(season);
                        result.episode = Some(episode);
                    }
                }
                Ok(result)
            })
            .collect();
        let mut results = map_result_vec(results)?;
        results.sort_by_key(|r| r.idx);
        Ok(results)
    }
}
