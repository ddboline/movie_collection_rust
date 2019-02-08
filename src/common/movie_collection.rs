extern crate chrono;
extern crate failure;
extern crate r2d2;
extern crate r2d2_postgres;
extern crate rayon;
extern crate reqwest;
extern crate select;

use chrono::NaiveDate;
use failure::{err_msg, Error};
use rayon::prelude::*;
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::io;
use std::io::Write;
use std::path::Path;

use crate::common::config::Config;
use crate::common::imdb_episodes::ImdbEpisodes;
use crate::common::imdb_ratings::ImdbRatings;
use crate::common::movie_queue::MovieQueueDB;
use crate::common::pgpool::PgPool;
use crate::common::utils::{
    map_result_vec, option_string_wrapper, parse_file_stem, walk_directory,
};

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
    pub source: Option<String>,
}

impl fmt::Display for TvShowsResult {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "{} {} {} {}",
            self.show,
            self.link,
            self.count,
            option_string_wrapper(&self.source),
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

#[derive(Debug, Default)]
pub struct ImdbSeason {
    pub show: String,
    pub title: String,
    pub season: i32,
    pub nepisodes: i64,
}

impl fmt::Display for ImdbSeason {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "{} {} {} {}",
            self.show, self.title, self.season, self.nepisodes
        )
    }
}

impl ImdbSeason {
    pub fn get_string_vec(&self) -> Vec<String> {
        vec![
            self.show.clone(),
            self.title.clone(),
            self.season.to_string(),
            self.nepisodes.to_string(),
        ]
    }
}

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
        let pgurl = &config.pgurl;
        MovieCollectionDB {
            pool: PgPool::new(&pgurl),
            config,
        }
    }

    pub fn with_pool(pool: &PgPool) -> MovieCollectionDB {
        let config = Config::with_config();
        MovieCollectionDB {
            config,
            pool: pool.clone(),
        }
    }
}

impl MovieCollection for MovieCollectionDB {
    fn get_pool(&self) -> &PgPool {
        &self.pool
    }

    fn get_config(&self) -> &Config {
        &self.config
    }
}

pub trait MovieCollection: Send + Sync {
    fn get_pool(&self) -> &PgPool;

    fn get_config(&self) -> &Config;

    fn print_imdb_shows(&self, show: &str, istv: bool) -> Result<Vec<ImdbRatings>, Error> {
        let query = format!("SELECT show FROM imdb_ratings WHERE show like '%{}%'", show);
        let query = if istv {
            format!("{} AND istv", query)
        } else {
            query
        };
        let shows: Vec<String> = self
            .get_pool()
            .get()?
            .query(&query, &[])?
            .iter()
            .map(|r| r.get(0))
            .collect();

        let shows = if shows.contains(&show.to_string()) {
            vec![show.to_string()]
        } else {
            shows
        };

        let shows: Vec<Result<_, Error>> = shows
            .par_iter()
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

                let results: Vec<_> = self
                    .get_pool()
                    .get()?
                    .query(&query, &[])?
                    .iter()
                    .map(|row| {
                        let index: i32 = row.get(0);
                        let show: String = row.get(1);
                        let title: String = row.get(2);
                        let link: String = row.get(3);
                        let rating: f64 = row.get(4);

                        ImdbRatings {
                            index,
                            show,
                            title: Some(title),
                            link,
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

    fn print_imdb_episodes(
        &self,
        show: &str,
        season: Option<i32>,
    ) -> Result<Vec<ImdbEpisodes>, Error> {
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

        let result: Vec<_> = self
            .get_pool()
            .get()?
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

    fn print_imdb_all_seasons(&self, show: &str) -> Result<Vec<ImdbSeason>, Error> {
        let query = r#"
            SELECT a.show, b.title, a.season, count(distinct a.episode)
            FROM imdb_episodes a
            JOIN imdb_ratings b ON a.show=b.show"#;
        let query = format!("{} WHERE a.show = '{}'", query, show);
        let query = format!("{} GROUP BY a.show, b.title, a.season", query);
        let query = format!("{} ORDER BY a.season", query);

        let result = self
            .get_pool()
            .get()?
            .query(&query, &[])?
            .iter()
            .map(|row| {
                let show: String = row.get(0);
                let title: String = row.get(1);
                let season: i32 = row.get(2);
                let nepisodes: i64 = row.get(3);

                ImdbSeason {
                    show,
                    title,
                    season,
                    nepisodes,
                }
            })
            .collect();
        Ok(result)
    }

    fn search_movie_collection(
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
            .get_pool()
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
                        .get_pool()
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

    fn remove_from_collection(&self, path: &str) -> Result<(), Error> {
        let query = r#"
            DELETE FROM movie_collection
            WHERE path = $1
        "#;
        self.get_pool()
            .get()?
            .execute(query, &[&path.to_string()])?;
        Ok(())
    }

    fn get_collection_index(&self, path: &str) -> Result<Option<i32>, Error> {
        let query = r#"SELECT idx FROM movie_collection WHERE path = $1"#;
        let result = self
            .get_pool()
            .get()?
            .query(query, &[&path])?
            .iter()
            .map(|row| row.get(0))
            .nth(0);
        Ok(result)
    }

    fn get_collection_path(&self, idx: i32) -> Result<String, Error> {
        let query = "SELECT path FROM movie_collection WHERE idx = $1";
        let path: String = self
            .get_pool()
            .get()?
            .query(query, &[&idx])?
            .iter()
            .nth(0)
            .ok_or_else(|| err_msg("Index not found"))?
            .get(0);
        Ok(path)
    }

    fn get_collection_index_match(&self, path: &str) -> Result<Option<i32>, Error> {
        let query = format!(
            r#"SELECT idx FROM movie_collection WHERE path like '%{}%'"#,
            path
        );
        let result = self
            .get_pool()
            .get()?
            .query(&query, &[])?
            .iter()
            .map(|row| row.get(0))
            .nth(0);
        Ok(result)
    }

    fn insert_into_collection(&self, path: &str) -> Result<(), Error> {
        if !Path::new(&path).exists() {
            return Err(err_msg("No such file"));
        }
        let file_stem = Path::new(&path).file_stem().unwrap().to_str().unwrap();
        let (show, _, _) = parse_file_stem(&file_stem);
        let query = r#"
            INSERT INTO movie_collection (idx, path, show)
            VALUES ((SELECT max(idx)+1 FROM movie_collection), $1, $2)
        "#;
        self.get_pool()
            .get()?
            .execute(query, &[&path.to_string(), &show])?;
        Ok(())
    }

    fn fix_collection_show_id(&self) -> Result<u64, Error> {
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
        let rows = self.get_pool().get()?.execute(query, &[])?;
        Ok(rows)
    }

    fn make_collection(&self) -> Result<(), Error> {
        let file_list: Vec<_> = self
            .get_config()
            .movie_dirs
            .par_iter()
            .map(|d| walk_directory(&d, &self.get_config().suffixes))
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
            .get_pool()
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
            .get_pool()
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
            .get_pool()
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

        let stdout = io::stdout();

        for f in &file_list {
            if collection_map.get(f).is_none() {
                let ext = Path::new(&f)
                    .extension()
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .to_string();
                if !self.get_config().suffixes.contains(&ext) {
                    continue;
                }
                writeln!(stdout.lock(), "not in collection {}", f)?;
                self.insert_into_collection(f)?;
            }
        }
        let results = collection_map
            .par_iter()
            .map(|(key, val)| {
                if !file_list.contains(key) {
                    match movie_queue.get(key) {
                        Some(v) => {
                            writeln!(stdout.lock(), "in queue but not disk {} {}", key, v)?;
                            let mq = MovieQueueDB::with_pool(self.get_pool());
                            mq.remove_from_queue_by_path(key)?;
                            self.remove_from_collection(key)
                        }
                        None => {
                            writeln!(stdout.lock(), "not on disk {} {}", key, val)?;
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
                    writeln!(stdout.lock(), "in queue but not disk {}", key)?;
                } else {
                    writeln!(stdout.lock(), "not on disk {} {}", key, val)?;
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
            writeln!(stdout.lock(), "show has episode not in db {} ", show)?;
        }
        Ok(())
    }

    fn get_imdb_show_map(&self) -> Result<HashMap<String, ImdbRatings>, Error> {
        let query = r#"
            SELECT link, show, title, rating, istv, source
            FROM imdb_ratings
            WHERE link IS NOT null AND rating IS NOT null
        "#;
        let results: HashMap<String, ImdbRatings> = self
            .get_pool()
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
                        link,
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

    fn print_tv_shows(&self) -> Result<Vec<TvShowsResult>, Error> {
        let query = r#"
            SELECT b.show, c.link, c.title, c.source, count(*) as count
            FROM movie_queue a
            JOIN movie_collection b ON a.collection_idx=b.idx
            JOIN imdb_ratings c ON b.show_id=c.index
            WHERE c.istv
            GROUP BY 1,2,3,4
            ORDER BY 1,2,3,4
        "#;
        let results: Vec<_> = self
            .get_pool()
            .get()?
            .query(query, &[])?
            .iter()
            .map(|row| {
                let show: String = row.get(0);
                let link: String = row.get(1);
                let title: String = row.get(2);
                let source: Option<String> = row.get(3);
                let count: i64 = row.get(4);
                TvShowsResult {
                    show,
                    link,
                    title,
                    count,
                    source,
                }
            })
            .collect();
        Ok(results)
    }

    fn get_new_episodes(
        &self,
        mindate: NaiveDate,
        maxdate: NaiveDate,
        source: Option<String>,
    ) -> Result<Vec<NewEpisodesResult>, Error> {
        let query = r#"
            WITH active_links AS (
                SELECT c.link
                FROM movie_queue a
                JOIN movie_collection b ON a.collection_idx=b.idx
                JOIN imdb_ratings c ON b.show_id=c.index
                JOIN imdb_episodes d ON c.show = d.show
                UNION
                SELECT link
                FROM trakt_watchlist
            )
            SELECT c.show,
                    c.link,
                    c.title,
                    d.season,
                    d.episode,
                    d.epurl,
                    d.airdate,
                    c.rating,
                    cast(d.rating as double precision),
                    d.eptitle
            FROM imdb_ratings c
            JOIN imdb_episodes d ON c.show = d.show
            LEFT JOIN trakt_watched_episodes e ON c.link=e.link AND d.season=e.season AND d.episode=e.episode
            WHERE c.link in (SELECT link FROM active_links GROUP BY link) AND
                e.episode is null AND
                c.istv AND d.airdate >= $1 AND
                d.airdate <= $2
        "#;
        let groupby = r#"
            GROUP BY 1,2,3,4,5,6,7,8,9,10
            ORDER BY d.airdate, c.show, d.season, d.episode
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
            .get_pool()
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
