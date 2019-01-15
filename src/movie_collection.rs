extern crate chrono;
extern crate failure;
extern crate r2d2;
extern crate r2d2_postgres;
extern crate rayon;
extern crate reqwest;
extern crate select;

use chrono::NaiveDate;
use failure::{err_msg, Error};
use r2d2::Pool;
use r2d2_postgres::{PostgresConnectionManager, TlsMode};
use rayon::prelude::*;
use reqwest::Url;
use reqwest::{Client, Response};
use select::document::Document;
use select::predicate::{Class, Name};
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::path::Path;
use std::thread::sleep;
use std::time::Duration;

use crate::config::Config;
use crate::imdb_episodes::ImdbEpisodes;
use crate::imdb_ratings::ImdbRatings;
use crate::utils::{map_result_vec, option_string_wrapper, parse_file_stem, walk_directory};

pub type PgPool = Pool<PostgresConnectionManager>;

#[derive(Debug)]
pub struct MovieCollectionDB {
    pub config: Config,
    pub pool: PgPool,
}

impl Default for MovieCollectionDB {
    fn default() -> MovieCollectionDB {
        MovieCollectionDB::new()
    }
}

impl MovieCollectionDB {
    pub fn new() -> MovieCollectionDB {
        let config = Config::with_config();
        let manager = PostgresConnectionManager::new(config.pgurl.clone(), TlsMode::None)
            .expect("Failed to open DB connection");
        MovieCollectionDB {
            config,
            pool: Pool::new(manager).expect("Failed to open DB connection"),
        }
    }

    pub fn print_imdb_shows(&self, show: &str, istv: bool) -> Result<Vec<ImdbRatings>, Error> {
        let conn = self.pool.get()?;

        let query = format!("SELECT show FROM imdb_ratings WHERE show like '%{}%'", show);
        let query = if istv {
            format!("{} AND istv", query)
        } else {
            query
        };
        let shows: Vec<String> = conn.query(&query, &[])?.iter().map(|r| r.get(0)).collect();

        let shows = if shows.contains(&show.to_string()) {
            vec![show.to_string()]
        } else {
            shows
        };

        let shows: Vec<Result<_, Error>> = shows
            .iter()
            .map(|show| {
                let query = r#"
                    SELECT index, show, title, link, rating
                    FROM imdb_ratings
                    WHERE link is not null AND rating is not null"#;
                let query = format!("{} AND show = '{}'", query, show);
                let query = if istv {
                    format!("{} AND istv", query)
                } else {
                    query
                };

                let results: Vec<_> = conn
                    .query(&query, &[])?
                    .iter()
                    .map(|row| {
                        let index: i32 = row.get(0);
                        let show: String = row.get(1);
                        let title: String = row.get(2);
                        let link: String = row.get(3);
                        let rating: f64 = row.get(4);

                        println!("{} {} {} {}", show, title, link, rating);
                        ImdbRatings {
                            index,
                            show,
                            title: Some(title),
                            link: Some(link),
                            rating: Some(rating),
                            ..Default::default()
                        }
                    })
                    .collect();
                Ok(results)
            })
            .collect();
        let results: Vec<_> = map_result_vec(shows)?.into_iter().flatten().collect();
        Ok(results)
    }

    pub fn print_imdb_episodes(
        &self,
        show: &str,
        season: Option<i32>,
    ) -> Result<Vec<ImdbEpisodes>, Error> {
        let conn = self.pool.get()?;
        let query = r#"
            SELECT a.show, b.title, a.season, a.episode,
                   a.airdate,
                   cast(a.rating as double precision),
                   a.eptitle, a.epurl
            FROM imdb_episodes a
            JOIN imdb_ratings b ON a.show=b.show"#;
        let query = format!("{} WHERE a.show = '{}'", query, show);
        let query = if let Some(season) = season {
            format!("{} AND a.season = {}", query, season)
        } else {
            query
        };
        let query = format!("{} ORDER BY a.season, a.episode", query);

        let result: Vec<_> = conn
            .query(&query, &[])?
            .iter()
            .map(|row| {
                let show: String = row.get(0);
                let title: String = row.get(1);
                let season: i32 = row.get(2);
                let episode: i32 = row.get(3);
                let airdate: NaiveDate = row.get(4);
                let rating: f64 = row.get(5);
                let eptitle: String = row.get(6);
                let epurl: String = row.get(7);

                println!(
                    "{} {} {} {} {} {} {} {}",
                    show, title, season, episode, airdate, rating, eptitle, epurl
                );
                ImdbEpisodes {
                    show,
                    title,
                    season,
                    episode,
                    airdate,
                    rating,
                    eptitle,
                    epurl,
                }
            })
            .collect();
        Ok(result)
    }

    pub fn print_imdb_all_seasons(&self, show: &str) -> Result<(), Error> {
        let conn = self.pool.get()?;
        let query = r#"
            SELECT a.show, b.title, a.season, count(distinct a.episode)
            FROM imdb_episodes a
            JOIN imdb_ratings b ON a.show=b.show"#;
        let query = format!("{} WHERE a.show = '{}'", query, show);
        let query = format!("{} GROUP BY a.show, b.title, a.season", query);
        let query = format!("{} ORDER BY a.season", query);

        for row in conn.query(&query, &[])?.iter() {
            let show: String = row.get(0);
            let title: String = row.get(1);
            let season: i32 = row.get(2);
            let nepisodes: i64 = row.get(3);

            println!("{} {} {} {}", show, title, season, nepisodes);
        }
        Ok(())
    }

    pub fn search_movie_collection(
        &self,
        search_strs: &[String],
    ) -> Result<Vec<MovieCollectionResult>, Error> {
        let query = r#"
            SELECT a.path, a.show,
            COALESCE(b.rating, -1),
            COALESCE(b.title, ''),
            COALESCE(b.istv, FALSE)
            FROM movie_collection a
            LEFT JOIN imdb_ratings b ON a.show_id = b.index
        "#;
        let query = if !search_strs.is_empty() {
            let search_strs: Vec<_> = search_strs
                .iter()
                .map(|s| format!("a.path like '%{}%'", s))
                .collect();
            format!("{} WHERE {}", query, search_strs.join(" OR "))
        } else {
            query.to_string()
        };

        let results: Vec<_> = self
            .pool
            .get()?
            .query(&query, &[])?
            .iter()
            .map(|row| {
                let mut result = MovieCollectionResult::default();
                result.path = row.get(0);
                result.show = row.get(1);
                result.rating = row.get(2);
                result.title = row.get(3);
                let istv: Option<bool> = row.get(4);
                result.istv = istv.unwrap_or(false);
                result
            })
            .collect();

        let results: Vec<Result<_, Error>> = results
            .into_par_iter()
            .map(|mut result| {
                let file_stem = Path::new(&result.path)
                    .file_stem()
                    .unwrap()
                    .to_str()
                    .unwrap();
                let (show, season, episode) = parse_file_stem(&file_stem);

                if season != -1 && episode != -1 && show == result.show {
                    let query = r#"
                        SELECT cast(rating as double precision), eptitle, epurl
                        FROM imdb_episodes
                        WHERE show = $1 AND season = $2 AND episode = $3
                    "#;
                    for row in self
                        .pool
                        .get()?
                        .query(query, &[&show, &season, &episode])?
                        .iter()
                    {
                        result.season = Some(season);
                        result.episode = Some(episode);
                        result.eprating = row.get(0);
                        result.eptitle = row.get(1);
                        result.epurl = row.get(2);
                    }
                }
                Ok(result)
            })
            .collect();
        let mut results = map_result_vec(results)?;
        results.sort_by_key(|r| (r.season, r.episode));
        Ok(results)
    }

    pub fn remove_from_queue_by_idx(&self, idx: i32) -> Result<(), Error> {
        let max_idx = self.get_max_queue_index()?;
        if idx > max_idx || idx < 0 {
            return Ok(());
        }
        let diff = max_idx - idx;

        let conn = self.pool.get()?;
        let tran = conn.transaction()?;
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
        let collection_idx = self.get_collection_index(&path)?;
        self.remove_from_queue_by_collection_idx(collection_idx)
    }

    pub fn insert_into_queue(&self, idx: i32, path: &str) -> Result<(), Error> {
        if !Path::new(&path).exists() {
            return Err(err_msg("File doesn't exist"));
        }
        let collection_idx = self.get_collection_index(&path)?;

        let query = r#"SELECT idx FROM movie_queue WHERE collection_idx = $1"#;
        let mut current_idx: i32 = -1;
        for row in self.pool.get()?.query(query, &[&collection_idx])?.iter() {
            current_idx = row.get(0);
        }

        if current_idx != -1 {
            self.remove_from_queue_by_idx(current_idx)?;
        }

        let max_idx = self.get_max_queue_index()?;
        let diff = max_idx - idx + 1;

        let conn = self.pool.get()?;
        let tran = conn.transaction()?;
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

    pub fn remove_from_collection(&self, path: &str) -> Result<(), Error> {
        let query = r#"
            DELETE FROM movie_collection
            WHERE path = $1
        "#;
        self.pool.get()?.execute(query, &[&path.to_string()])?;
        Ok(())
    }

    pub fn get_collection_index(&self, path: &str) -> Result<i32, Error> {
        let query = r#"SELECT idx FROM movie_collection WHERE path = $1"#;
        let result = match self
            .pool
            .get()?
            .query(query, &[&path])?
            .iter()
            .map(|row| row.get(0))
            .nth(0)
        {
            Some(r) => r,
            None => {
                if Path::new(path).exists() {
                    let max_idx = self.get_max_queue_index()?;
                    self.insert_into_queue(max_idx, &path)?;
                    Some(self.get_collection_index(&path)?)
                } else {
                    None
                }
            }
        }
        .ok_or_else(|| err_msg("No path found"))?;
        Ok(result)
    }

    pub fn get_collection_path(&self, idx: i32) -> Result<String, Error> {
        let query = "SELECT path FROM movie_collection WHERE idx = $1";
        let path: String = self
            .pool
            .get()?
            .query(query, &[&idx])?
            .iter()
            .nth(0)
            .ok_or_else(|| err_msg("Index not found"))?
            .get(0);
        Ok(path)
    }

    pub fn get_collection_index_match(&self, path: &str) -> Result<i32, Error> {
        let query = format!(
            r#"SELECT idx FROM movie_collection WHERE path like '%{}%'"#,
            path
        );
        let result = self
            .pool
            .get()?
            .query(&query, &[])?
            .iter()
            .map(|row| row.get(0))
            .nth(0)
            .ok_or_else(|| err_msg("No path found"))?;
        Ok(result)
    }

    pub fn insert_into_collection(&self, path: &str) -> Result<(), Error> {
        if !Path::new(&path).exists() {
            return Err(err_msg("No such file"));
        }
        let file_stem = Path::new(&path).file_stem().unwrap().to_str().unwrap();
        let (show, _, _) = parse_file_stem(&file_stem);
        let query = r#"
            INSERT INTO movie_collection (idx, path, show)
            VALUES ((SELECT max(idx)+1 FROM movie_collection), $1, $2)
        "#;
        self.pool
            .get()?
            .execute(query, &[&path.to_string(), &show])?;
        Ok(())
    }

    pub fn fix_collection_show_id(&self) -> Result<u64, Error> {
        let query = r#"
            WITH a AS (
                SELECT a.idx, a.show, b.index
                FROM movie_collection a
                JOIN imdb_ratings b ON a.show = b.show
                WHERE a.show_id is null
            )
            UPDATE movie_collection b set show_id=(SELECT c.index FROM imdb_ratings c WHERE b.show=c.show)
            WHERE idx in (SELECT a.idx FROM a)
        "#;
        let rows = self.pool.get()?.execute(query, &[])?;
        Ok(rows)
    }

    pub fn make_collection(&self) -> Result<(), Error> {
        let file_list: Vec<_> = self
            .config
            .movie_dirs
            .par_iter()
            .map(|d| walk_directory(&d, &self.config.suffixes))
            .collect();

        let file_list: HashSet<_> = map_result_vec(file_list)?.into_iter().flatten().collect();

        let episode_list: HashSet<_> = file_list
            .par_iter()
            .filter_map(|f| {
                let file_stem = Path::new(f).file_stem().unwrap().to_str().unwrap();
                let (show, season, episode) = parse_file_stem(&file_stem);
                if season == -1 || episode == -1 {
                    None
                } else {
                    Some((show, season, episode, f))
                }
            })
            .collect();

        let query = r#"
            SELECT b.path, a.idx
            FROM movie_queue a
            JOIN movie_collection b ON a.collection_idx=b.idx
        "#;
        let movie_queue: HashMap<String, i32> = self
            .pool
            .get()?
            .query(query, &[])?
            .iter()
            .map(|row| {
                let path: String = row.get(0);
                let idx: i32 = row.get(1);
                (path, idx)
            })
            .collect();

        let query = "SELECT path, show FROM movie_collection";
        let collection_map: HashMap<String, String> = self
            .pool
            .get()?
            .query(query, &[])?
            .iter()
            .map(|row| {
                let path: String = row.get(0);
                let show: String = row.get(1);
                (path, show)
            })
            .collect();
        let query = "SELECT show, season, episode from imdb_episodes";
        let episodes_set: HashSet<(String, i32, i32)> = self
            .pool
            .get()?
            .query(query, &[])?
            .iter()
            .map(|row| {
                let show: String = row.get(0);
                let season: i32 = row.get(1);
                let episode: i32 = row.get(2);
                (show, season, episode)
            })
            .collect();

        for e in &file_list {
            if collection_map.get(e).is_none() {
                let ext = Path::new(&e)
                    .extension()
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .to_string();
                if !self.config.suffixes.contains(&ext) {
                    continue;
                }
                println!("not in collection {}", e);
                self.insert_into_collection(e)?;
            }
        }
        let results = collection_map
            .par_iter()
            .map(|(key, val)| {
                if !file_list.contains(key) {
                    match movie_queue.get(key) {
                        Some(v) => {
                            println!("in queue but not disk {} {}", key, v);
                            self.remove_from_queue_by_path(key)?;
                            self.remove_from_collection(key)
                        }
                        None => {
                            println!("not on disk {} {}", key, val);
                            self.remove_from_collection(key)
                        }
                    }
                } else {
                    Ok(())
                }
            })
            .collect();

        map_result_vec(results)?;

        for (key, val) in collection_map {
            if !file_list.contains(&key) {
                if movie_queue.contains_key(&key) {
                    println!("in queue but not disk {}", key);
                } else {
                    println!("not on disk {} {}", key, val);
                }
            }
        }
        let mut shows_not_in_db: HashSet<String> = HashSet::new();
        for (show, season, episode, _) in &episode_list {
            let key = (show.clone(), *season, *episode);
            if !episodes_set.contains(&key) {
                shows_not_in_db.insert(show.clone());
            }
        }
        for show in shows_not_in_db {
            println!("show has episode not in db {} ", show);
        }
        Ok(())
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

    pub fn get_imdb_show_map(&self) -> Result<HashMap<String, ImdbRatings>, Error> {
        let query = r#"
            SELECT link, show, title, rating, istv, source
            FROM imdb_ratings
            WHERE link IS NOT null AND rating IS NOT null
        "#;
        let results: HashMap<String, ImdbRatings> = self
            .pool
            .get()?
            .query(query, &[])?
            .iter()
            .map(|row| {
                let link: String = row.get(0);
                let show: String = row.get(1);
                let title: String = row.get(2);
                let rating: f64 = row.get(3);
                let istv: bool = row.get(4);
                let source: Option<String> = row.get(5);

                (
                    link.clone(),
                    ImdbRatings {
                        show,
                        title: Some(title),
                        link: Some(link),
                        rating: Some(rating),
                        istv: Some(istv),
                        source,
                        ..Default::default()
                    },
                )
            })
            .collect();
        Ok(results)
    }

    pub fn print_tv_shows(&self) -> Result<Vec<TvShowsResult>, Error> {
        let query = r#"
            SELECT b.show, c.link, c.title, count(*)
            FROM movie_queue a
            JOIN movie_collection b ON a.collection_idx=b.idx
            JOIN imdb_ratings c ON b.show_id=c.index
            WHERE c.istv
            GROUP BY 1,2,3
            ORDER BY 1,2,3
        "#;
        let results: Vec<_> = self
            .pool
            .get()?
            .query(query, &[])?
            .iter()
            .map(|row| {
                let show: String = row.get(0);
                let link: String = row.get(1);
                let title: String = row.get(2);
                let count: i64 = row.get(3);
                TvShowsResult {
                    show,
                    link,
                    title,
                    count,
                }
            })
            .collect();
        Ok(results)
    }

    pub fn get_new_episodes(
        &self,
        mindate: NaiveDate,
        maxdate: NaiveDate,
        source: Option<String>,
    ) -> Result<Vec<NewEpisodesResult>, Error> {
        let query = r#"
            SELECT b.show,
                   c.link,
                   c.title,
                   d.season,
                   d.episode,
                   d.epurl,
                   d.airdate,
                   c.rating,
                   cast(d.rating as double precision),
                   d.eptitle
            FROM movie_queue a
            JOIN movie_collection b ON a.collection_idx=b.idx
            JOIN imdb_ratings c ON b.show_id=c.index
            JOIN imdb_episodes d ON c.show = d.show
            LEFT JOIN trakt_watched_episodes e ON c.link=e.link AND d.season=e.season AND d.episode=e.episode
            WHERE e.episode is null AND c.istv AND d.airdate >= $1 AND d.airdate <= $2
        "#;
        let groupby = r#"
            GROUP BY 1,2,3,4,5,6,7,8,9,10
            ORDER BY d.airdate, b.show, d.season, d.episode
        "#;
        let query: String = match source {
            Some(source) => {
                if source.as_str() == "all" {
                    query.to_string()
                } else {
                    format!("{} AND c.source = '{}'", query, source)
                }
            }
            None => format!("{} AND c.source is null", query),
        };
        let query = format!("{} {}", query, groupby);

        let results = self
            .pool
            .get()?
            .query(&query, &[&mindate, &maxdate])?
            .iter()
            .map(|row| {
                let show: String = row.get(0);
                let link: String = row.get(1);
                let title: String = row.get(2);
                let season: i32 = row.get(3);
                let episode: i32 = row.get(4);
                let epurl: String = row.get(5);
                let airdate: NaiveDate = row.get(6);
                let rating: f64 = row.get(7);
                let eprating: f64 = row.get(8);
                let eptitle: String = row.get(9);
                NewEpisodesResult {
                    show,
                    link,
                    title,
                    season,
                    episode,
                    epurl,
                    airdate,
                    rating,
                    eprating,
                    eptitle,
                }
            })
            .collect();
        Ok(results)
    }
}

pub struct NewEpisodesResult {
    pub show: String,
    pub link: String,
    pub title: String,
    pub season: i32,
    pub episode: i32,
    pub epurl: String,
    pub airdate: NaiveDate,
    pub rating: f64,
    pub eprating: f64,
    pub eptitle: String,
}

impl fmt::Display for NewEpisodesResult {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "{} {} {} {} {} {} {} {} {} {}",
            self.show,
            self.link,
            self.title,
            self.season,
            self.episode,
            self.epurl,
            self.airdate,
            self.rating,
            self.eprating,
            self.eptitle,
        )
    }
}

#[derive(Default)]
pub struct TvShowsResult {
    pub show: String,
    pub link: String,
    pub count: i64,
    pub title: String,
}

impl fmt::Display for TvShowsResult {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{} {} {}", self.show, self.link, self.count,)
    }
}

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

#[derive(Default)]
pub struct MovieCollectionResult {
    pub path: String,
    pub show: String,
    pub rating: f64,
    pub title: String,
    pub istv: bool,
    pub eprating: Option<f64>,
    pub season: Option<i32>,
    pub episode: Option<i32>,
    pub eptitle: Option<String>,
    pub epurl: Option<String>,
}

impl fmt::Display for MovieCollectionResult {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        if !self.istv {
            write!(
                f,
                "{} {} {:.1} {}",
                self.path, self.show, self.rating, self.title
            )
        } else {
            write!(
                f,
                "{} {} {:.1}/{:.1} s{:02} ep{:02} {} {} {}",
                self.path,
                self.show,
                self.rating,
                self.eprating.unwrap_or(-1.0),
                self.season.unwrap_or(-1),
                self.episode.unwrap_or(-1),
                self.title,
                option_string_wrapper(&self.eptitle),
                option_string_wrapper(&self.epurl),
            )
        }
    }
}

#[derive(Default)]
pub struct ImdbTuple {
    pub title: String,
    pub link: String,
    pub rating: f64,
}

impl fmt::Display for ImdbTuple {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{} {} {}", self.title, self.link, self.rating)
    }
}

#[derive(Debug)]
pub struct RatingOutput {
    pub rating: Option<f64>,
    pub count: Option<u64>,
}

#[derive(Default, Clone)]
pub struct ImdbEpisodeResult {
    pub season: i32,
    pub episode: i32,
    pub epurl: Option<String>,
    pub eptitle: Option<String>,
    pub airdate: Option<NaiveDate>,
    pub rating: Option<f64>,
    pub nrating: Option<u64>,
}

impl fmt::Display for ImdbEpisodeResult {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "{} {} {} {} {} {} {} ",
            self.season,
            self.episode,
            option_string_wrapper(&self.epurl),
            option_string_wrapper(&self.eptitle),
            self.airdate
                .unwrap_or_else(|| NaiveDate::from_ymd(1970, 1, 1)),
            self.rating.unwrap_or(-1.0),
            self.nrating.unwrap_or(0),
        )
    }
}

pub struct ImdbConnection {
    client: Client,
}

impl Default for ImdbConnection {
    fn default() -> ImdbConnection {
        ImdbConnection::new()
    }
}

impl ImdbConnection {
    pub fn new() -> ImdbConnection {
        ImdbConnection {
            client: Client::new(),
        }
    }

    pub fn get(&self, url: &Url) -> Result<Response, Error> {
        let mut timeout: u64 = 1;
        loop {
            match self.client.get(url.clone()).send() {
                Ok(x) => return Ok(x),
                Err(e) => {
                    sleep(Duration::from_secs(timeout));
                    timeout *= 2;
                    if timeout >= 64 {
                        return Err(err_msg(e));
                    }
                }
            }
        }
    }

    pub fn parse_imdb(&self, title: &str) -> Result<Vec<ImdbTuple>, Error> {
        let endpoint = "http://www.imdb.com/find?";
        let url = Url::parse_with_params(endpoint, &[("s", "all"), ("q", title)])?;
        let body = self.get(&url)?.text()?;

        let document = Document::from(body.as_str());

        let mut shows = Vec::new();

        for tr in document.find(Class("result_text")) {
            let title = tr.text().trim().to_string();
            for a in tr.find(Name("a")) {
                if let Some(link) = a.attr("href") {
                    if let Some(link) = link.split('/').nth(2) {
                        if link.starts_with("tt") {
                            shows.push((title.clone(), link));
                        }
                    }
                } else {
                };
            }
        }

        let results: Vec<Result<_, Error>> = shows
            .into_par_iter()
            .map(|(t, l)| {
                let r = self.parse_imdb_rating(l)?;
                Ok(ImdbTuple {
                    title: t.to_string(),
                    link: l.to_string(),
                    rating: r.rating.unwrap_or(-1.0),
                })
            })
            .collect();

        let shows: Vec<_> = map_result_vec(results)?;

        Ok(shows)
    }

    pub fn parse_imdb_rating(&self, title: &str) -> Result<RatingOutput, Error> {
        let mut output = RatingOutput {
            rating: None,
            count: None,
        };

        if !title.starts_with("tt") {
            return Ok(output);
        };

        let url = Url::parse("http://www.imdb.com/title/")?.join(title)?;
        let body = self.get(&url)?.text()?;
        let document = Document::from(body.as_str());

        for span in document.find(Name("span")) {
            if let Some("ratingValue") = span.attr("itemprop") {
                output.rating = Some(span.text().parse()?);
            }
            if let Some("ratingCount") = span.attr("itemprop") {
                output.count = Some(span.text().replace(",", "").parse()?);
            }
        }
        Ok(output)
    }

    pub fn parse_imdb_episode_list(
        &self,
        imdb_id: &str,
        season: Option<i32>,
    ) -> Result<Vec<ImdbEpisodeResult>, Error> {
        let endpoint: String = format!("http://m.imdb.com/title/{}/episodes", imdb_id);
        let url = Url::parse(&endpoint)?;
        let body = self.get(&url)?.text()?;
        let document = Document::from(body.as_str());

        let mut results = Vec::new();

        for a in document.find(Name("a")) {
            if let Some("season") = a.attr("class") {
                let season_: i32 = a.attr("season_number").unwrap_or("-1").parse()?;
                if let Some(link) = a.attr("href") {
                    if let Some(s) = season {
                        if s != season_ {
                            continue;
                        }
                    }
                    let episode_url = "http://www.imdb.com/title";
                    let episodes_url = format!("{}/{}/episodes/{}", episode_url, imdb_id, link);
                    results
                        .extend_from_slice(&self.parse_episodes_url(&episodes_url, Some(season_))?);
                }
            }
        }
        Ok(results)
    }

    fn parse_episodes_url(
        &self,
        episodes_url: &str,
        season: Option<i32>,
    ) -> Result<Vec<ImdbEpisodeResult>, Error> {
        let episodes_url = Url::parse(&episodes_url)?;
        let body = self.get(&episodes_url)?.text()?;
        let document = Document::from(body.as_str());

        let mut results = Vec::new();

        for div in document.find(Name("div")) {
            if let Some("info") = div.attr("class") {
                if let Some("episodes") = div.attr("itemprop") {
                    let mut result = ImdbEpisodeResult::default();
                    if let Some(s) = season {
                        result.season = s;
                    }
                    for meta in div.find(Name("meta")) {
                        if let Some("episodeNumber") = meta.attr("itemprop") {
                            if let Some(episode) = meta.attr("content") {
                                result.episode = episode.parse()?;
                            }
                        }
                    }
                    for div_ in div.find(Name("div")) {
                        if let Some("airdate") = div_.attr("class") {
                            if let Ok(date) =
                                NaiveDate::parse_from_str(div_.text().trim(), "%d %b. %Y")
                            {
                                result.airdate = Some(date);
                            }
                        }
                    }
                    for a_ in div.find(Name("a")) {
                        if result.epurl.is_some() {
                            continue;
                        };
                        if let Some(epi_url) = a_.attr("href") {
                            if let Some(link) = epi_url.split('/').nth(2) {
                                result.epurl = Some(link.to_string());
                                result.eptitle = Some(a_.text().trim().to_string());
                                let r = self.parse_imdb_rating(link)?;
                                result.rating = r.rating;
                                result.nrating = r.count;
                            }
                        }
                    }
                    results.push(result);
                }
            }
        }
        Ok(results)
    }
}
