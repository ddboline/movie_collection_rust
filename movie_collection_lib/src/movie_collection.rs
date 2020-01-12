use anyhow::{format_err, Error};
use chrono::{DateTime, Duration, Local, NaiveDate, Utc};
use postgres_query::FromSqlRow;
use rayon::iter::{IntoParallelIterator, IntoParallelRefIterator, ParallelIterator};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::ffi::OsStr;
use std::fmt;
use std::io;
use std::io::Write;
use std::path::Path;

use crate::config::Config;
use crate::imdb_episodes::ImdbEpisodes;
use crate::imdb_ratings::ImdbRatings;
use crate::movie_queue::MovieQueueDB;
use crate::pgpool::PgPool;

use crate::tv_show_source::TvShowSource;
use crate::utils::{option_string_wrapper, parse_file_stem, walk_directory};

#[derive(FromSqlRow)]
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

#[derive(Default, FromSqlRow)]
pub struct TvShowsResult {
    pub show: String,
    pub link: String,
    pub count: i64,
    pub title: String,
    pub source: Option<TvShowSource>,
}

impl fmt::Display for TvShowsResult {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "{} {} {} {}",
            self.show,
            self.link,
            self.count,
            self.source
                .as_ref()
                .map_or_else(|| "".to_string(), ToString::to_string),
        )
    }
}

#[derive(Default, Serialize, Deserialize, FromSqlRow)]
pub struct MovieCollectionRow {
    pub idx: i32,
    pub path: String,
    pub show: String,
}

#[derive(Default, FromSqlRow)]
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
        if self.istv {
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
        } else {
            write!(
                f,
                "{} {} {:.1} {}",
                self.path, self.show, self.rating, self.title
            )
        }
    }
}

#[derive(Debug, Default, FromSqlRow)]
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
            self.show.to_string(),
            self.title.to_string(),
            self.season.to_string(),
            self.nepisodes.to_string(),
        ]
    }
}

#[derive(Debug)]
pub struct MovieCollection {
    pub config: Config,
    pub pool: PgPool,
}

impl Default for MovieCollection {
    fn default() -> Self {
        Self::new()
    }
}

impl MovieCollection {
    pub fn new() -> Self {
        let config = Config::with_config().expect("Init config failed");
        let pgurl = &config.pgurl;
        Self {
            pool: PgPool::new(&pgurl),
            config,
        }
    }

    pub fn with_pool(pool: &PgPool) -> Result<Self, Error> {
        let config = Config::with_config()?;
        let mc = Self {
            config,
            pool: pool.clone(),
        };
        Ok(mc)
    }

    pub fn get_pool(&self) -> &PgPool {
        &self.pool
    }

    pub fn get_config(&self) -> &Config {
        &self.config
    }

    pub fn print_imdb_shows(&self, show: &str, istv: bool) -> Result<Vec<ImdbRatings>, Error> {
        let query = format!("SELECT show FROM imdb_ratings WHERE show like '%{}%'", show);
        let query = if istv {
            format!("{} AND istv", query)
        } else {
            query
        };
        let shows: Vec<String> = self
            .get_pool()
            .get()?
            .query(query.as_str(), &[])?
            .iter()
            .map(|r| r.get(0))
            .collect();

        let shows = if shows.contains(&show.to_string()) {
            vec![show.to_string()]
        } else {
            shows
        };

        let shows: Result<Vec<_>, Error> = shows
            .par_iter()
            .map(|show| {
                let query = postgres_query::query_dyn!(
                    &format!(
                        r#"
                            SELECT index, show, title, link, rating
                            FROM imdb_ratings
                            WHERE link is not null AND
                                  rating is not null AND
                                  show = $show {}
                        "#,
                        if istv { "AND istv" } else { "" },
                    ),
                    show = show
                )?;

                let results: Result<Vec<_>, Error> = self
                    .get_pool()
                    .get()?
                    .query(query.sql(), query.parameters())?
                    .iter()
                    .map(|row| {
                        let index: i32 = row.try_get("index")?;
                        let show: String = row.try_get("show")?;
                        let title: String = row.try_get("title")?;
                        let link: String = row.try_get("link")?;
                        let rating: f64 = row.try_get("rating")?;

                        Ok(ImdbRatings {
                            index,
                            show,
                            title: Some(title),
                            link,
                            rating: Some(rating),
                            ..ImdbRatings::default()
                        })
                    })
                    .collect();
                results
            })
            .collect();
        let results: Vec<_> = shows?.into_iter().flatten().collect();
        Ok(results)
    }

    pub fn print_imdb_episodes(
        &self,
        show: &str,
        season: Option<i32>,
    ) -> Result<Vec<ImdbEpisodes>, Error> {
        let query = postgres_query::query_dyn!(
            &format!(
                r#"
                    SELECT a.show, b.title, a.season, a.episode,
                        a.airdate,
                        cast(a.rating as double precision),
                        a.eptitle, a.epurl
                    FROM imdb_episodes a
                    JOIN imdb_ratings b ON a.show=b.show
                    WHERE a.show = $show {}
                    ORDER BY a.season, a.episode
                "#,
                if let Some(season) = season {
                    format!("AND a.season = {}", season)
                } else {
                    "".to_string()
                }
            ),
            show = show,
        )?;

        self.get_pool()
            .get()?
            .query(query.sql(), query.parameters())?
            .iter()
            .map(|row| {
                let val = ImdbEpisodes::from_row(row)?;

                Ok(val)
            })
            .collect()
    }

    pub fn print_imdb_all_seasons(&self, show: &str) -> Result<Vec<ImdbSeason>, Error> {
        let query = postgres_query::query!(
            r#"
                SELECT a.show, b.title, a.season, count(distinct a.episode) as nepisodes
                FROM imdb_episodes a
                JOIN imdb_ratings b ON a.show=b.show
                WHERE a.show = $show
                GROUP BY a.show, b.title, a.season
                ORDER BY a.season
            "#,
            show = show
        );

        self.get_pool()
            .get()?
            .query(query.sql(), query.parameters())?
            .iter()
            .map(|row| {
                let val = ImdbSeason::from_row(row)?;

                Ok(val)
            })
            .collect()
    }

    pub fn search_movie_collection(
        &self,
        search_strs: &[String],
    ) -> Result<Vec<MovieCollectionResult>, Error> {
        let query = postgres_query::query_dyn!(&format!(
            r#"
                SELECT a.path, a.show,
                COALESCE(b.rating, -1),
                COALESCE(b.title, ''),
                COALESCE(b.istv, FALSE)
                FROM movie_collection a
                LEFT JOIN imdb_ratings b ON a.show_id = b.index
                {}
            "#,
            if search_strs.is_empty() {
                "".to_string()
            } else {
                let search_strs: Vec<_> = search_strs
                    .iter()
                    .map(|s| format!("a.path like '%{}%'", s))
                    .collect();
                format!("WHERE {}", search_strs.join(" OR "))
            },
        ),)?;

        let results: Result<Vec<_>, Error> = self
            .get_pool()
            .get()?
            .query(query.sql(), &[])?
            .iter()
            .map(|row| {
                let path: String = row.try_get(0)?;
                let show: String = row.try_get(1)?;
                let rating: f64 = row.try_get(2)?;
                let title: String = row.try_get(3)?;
                let istv: Option<bool> = row.try_get(4)?;

                Ok(MovieCollectionResult {
                    path,
                    show,
                    rating,
                    title,
                    istv: istv.unwrap_or(false),
                    ..MovieCollectionResult::default()
                })
            })
            .collect();

        let results: Result<Vec<_>, Error> = results?
            .into_par_iter()
            .map(|mut result| {
                let file_stem = Path::new(&result.path)
                    .file_stem()
                    .unwrap()
                    .to_string_lossy();
                let (show, season, episode) = parse_file_stem(&file_stem);

                if season != -1 && episode != -1 && show == result.show {
                    let query = postgres_query::query!(
                        r#"
                            SELECT cast(rating as double precision), eptitle, epurl
                            FROM imdb_episodes
                            WHERE show = $show AND season = $season AND episode = $episode
                        "#,
                        show = show,
                        season = season,
                        episode = episode
                    );
                    for row in &self
                        .get_pool()
                        .get()?
                        .query(query.sql(), query.parameters())?
                    {
                        result.season = Some(season);
                        result.episode = Some(episode);
                        result.eprating = row.try_get(0)?;
                        result.eptitle = row.try_get(1)?;
                        result.epurl = row.try_get(2)?;
                    }
                }
                Ok(result)
            })
            .collect();
        let mut results = results?;
        results.sort_by_key(|r| (r.season, r.episode));
        Ok(results)
    }

    pub fn remove_from_collection(&self, path: &str) -> Result<(), Error> {
        let query = postgres_query::query!(
            r#"DELETE FROM movie_collection WHERE path = $path"#,
            path = path
        );
        self.get_pool()
            .get()?
            .execute(query.sql(), query.parameters())
            .map(|_| ())
            .map_err(Into::into)
    }

    pub fn get_collection_index(&self, path: &str) -> Result<Option<i32>, Error> {
        let query = postgres_query::query!(
            r#"SELECT idx FROM movie_collection WHERE path = $path"#,
            path = path
        );
        self.get_pool()
            .get()?
            .query(query.sql(), query.parameters())?
            .iter()
            .map(|row| row.try_get(0))
            .nth(0)
            .transpose()
            .map_err(Into::into)
    }

    pub fn get_collection_path(&self, idx: i32) -> Result<String, Error> {
        let query = postgres_query::query!(
            "SELECT path FROM movie_collection WHERE idx = $idx",
            idx = idx
        );
        let path: String = self
            .get_pool()
            .get()?
            .query(query.sql(), query.parameters())?
            .get(0)
            .ok_or_else(|| format_err!("Index not found"))?
            .get(0);
        Ok(path)
    }

    pub fn get_collection_index_match(&self, path: &str) -> Result<Option<i32>, Error> {
        let query = postgres_query::query_dyn!(&format!(
            r#"SELECT idx FROM movie_collection WHERE path like '%{}%'"#,
            path
        ))?;
        self.get_pool()
            .get()?
            .query(query.sql(), &[])?
            .iter()
            .map(|row| row.try_get(0))
            .nth(0)
            .transpose()
            .map_err(Into::into)
    }

    pub fn insert_into_collection(&self, path: &str) -> Result<(), Error> {
        if !Path::new(&path).exists() {
            return Err(format_err!("No such file"));
        }
        let file_stem = Path::new(&path).file_stem().unwrap().to_string_lossy();
        let (show, _, _) = parse_file_stem(&file_stem);
        let query = postgres_query::query!(
            r#"
                INSERT INTO movie_collection (idx, path, show, last_modified)
                VALUES ((SELECT max(idx)+1 FROM movie_collection), $path, $show, now())
            "#,
            path = path,
            show = show
        );
        self.get_pool()
            .get()?
            .execute(query.sql(), query.parameters())
            .map(|_| ())
            .map_err(Into::into)
    }

    pub fn insert_into_collection_by_idx(&self, idx: i32, path: &str) -> Result<(), Error> {
        let file_stem = Path::new(&path).file_stem().unwrap().to_string_lossy();
        let (show, _, _) = parse_file_stem(&file_stem);
        let query = postgres_query::query!(
            r#"
                INSERT INTO movie_collection (idx, path, show, last_modified)
                VALUES ($idx, $path, $show, now())
            "#,
            idx = idx,
            path = path,
            show = show
        );
        self.get_pool()
            .get()?
            .execute(query.sql(), query.parameters())
            .map(|_| ())
            .map_err(Into::into)
    }

    pub fn fix_collection_show_id(&self) -> Result<u64, Error> {
        let query = r#"
            WITH a AS (
                SELECT a.idx, a.show, b.index
                FROM movie_collection a
                JOIN imdb_ratings b ON a.show = b.show
                WHERE a.show_id is null
            )
            UPDATE movie_collection b
            SET show_id=(SELECT c.index FROM imdb_ratings c WHERE b.show=c.show),
                last_modified=now()
            WHERE idx in (SELECT a.idx FROM a)
        "#;
        let rows = self.get_pool().get()?.execute(query, &[])?;
        Ok(rows)
    }

    pub fn make_collection(&self) -> Result<(), Error> {
        let file_list: Result<Vec<_>, Error> = self
            .get_config()
            .movie_dirs
            .par_iter()
            .map(|d| walk_directory(&d, &self.get_config().suffixes))
            .collect();
        let file_list = file_list?;

        if file_list.is_empty() {
            return Ok(());
        }

        let file_list: HashSet<_> = file_list.into_iter().flatten().collect();

        let episode_list: Result<HashSet<_>, Error> = file_list
            .par_iter()
            .filter_map(|f| {
                let res = || {
                    let file_stem = Path::new(f)
                        .file_stem()
                        .map(OsStr::to_string_lossy)
                        .ok_or_else(|| format_err!("file_stem failed"))?;
                    let (show, season, episode) = parse_file_stem(&file_stem);
                    if season == -1 || episode == -1 {
                        Ok(None)
                    } else {
                        Ok(Some((show, season, episode, f)))
                    }
                };
                res().transpose()
            })
            .collect();
        let episode_list = episode_list?;

        let query = r#"
            SELECT b.path, a.idx
            FROM movie_queue a
            JOIN movie_collection b ON a.collection_idx=b.idx
        "#;
        let movie_queue: Result<HashMap<String, i32>, Error> = self
            .get_pool()
            .get()?
            .query(query, &[])?
            .iter()
            .map(|row| {
                let path: String = row.try_get(0)?;
                let idx: i32 = row.try_get(1)?;
                Ok((path, idx))
            })
            .collect();
        let movie_queue = movie_queue?;

        let query = "SELECT path, show FROM movie_collection";
        let collection_map: Result<HashMap<String, String>, Error> = self
            .get_pool()
            .get()?
            .query(query, &[])?
            .iter()
            .map(|row| {
                let path: String = row.try_get(0)?;
                let show: String = row.try_get(1)?;
                Ok((path, show))
            })
            .collect();
        let collection_map = collection_map?;

        let query = "SELECT show, season, episode from imdb_episodes";
        let episodes_set: Result<HashSet<(String, i32, i32)>, Error> = self
            .get_pool()
            .get()?
            .query(query, &[])?
            .iter()
            .map(|row| {
                let show: String = row.try_get(0)?;
                let season: i32 = row.try_get(1)?;
                let episode: i32 = row.try_get(2)?;
                Ok((show, season, episode))
            })
            .collect();
        let episodes_set = episodes_set?;

        let stdout = io::stdout();

        let results: Result<(), Error> = file_list
            .iter()
            .map(|f| {
                if collection_map.get(f).is_none() {
                    let ext = Path::new(&f)
                        .extension()
                        .map(OsStr::to_string_lossy)
                        .ok_or_else(|| format_err!("extension fail"))?
                        .to_string();
                    if self.get_config().suffixes.contains(&ext) {
                        writeln!(stdout.lock(), "not in collection {}", f)?;
                        self.insert_into_collection(f)?;
                    }
                }
                Ok(())
            })
            .collect();
        results?;

        let results: Result<(), Error> = collection_map
            .par_iter()
            .map(|(key, val)| {
                if file_list.contains(key) {
                    Ok(())
                } else {
                    match movie_queue.get(key) {
                        Some(v) => {
                            writeln!(stdout.lock(), "in queue but not disk {} {}", key, v)?;
                            let mq = MovieQueueDB::with_pool(self.get_pool());
                            mq.remove_from_queue_by_path(key)?;
                        }
                        None => {
                            writeln!(stdout.lock(), "not on disk {} {}", key, val)?;
                        }
                    };
                    self.remove_from_collection(key)
                }
            })
            .collect();
        results?;

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
        for (show, season, episode, _) in episode_list {
            let key = (show.to_string(), season, episode);
            if !episodes_set.contains(&key) {
                shows_not_in_db.insert(show);
            }
        }
        for show in shows_not_in_db {
            writeln!(stdout.lock(), "show has episode not in db {} ", show)?;
        }
        Ok(())
    }

    pub fn get_imdb_show_map(&self) -> Result<HashMap<String, ImdbRatings>, Error> {
        let query = r#"
            SELECT link, show, title, rating, istv, source
            FROM imdb_ratings
            WHERE link IS NOT null AND rating IS NOT null
        "#;
        self.get_pool()
            .get()?
            .query(query, &[])?
            .iter()
            .map(|row| {
                let link: String = row.try_get(0)?;
                let show: String = row.try_get(1)?;
                let title: String = row.try_get(2)?;
                let rating: f64 = row.try_get(3)?;
                let istv: bool = row.try_get(4)?;
                let source: Option<String> = row.try_get(5)?;

                let source: Option<TvShowSource> = match source {
                    Some(s) => s.parse().ok(),
                    None => None,
                };

                Ok((
                    link.to_string(),
                    ImdbRatings {
                        show,
                        title: Some(title),
                        link,
                        rating: Some(rating),
                        istv: Some(istv),
                        source,
                        ..ImdbRatings::default()
                    },
                ))
            })
            .collect()
    }

    pub fn print_tv_shows(&self) -> Result<Vec<TvShowsResult>, Error> {
        let query = r#"
            SELECT b.show, c.link, c.title, c.source, count(*) as count
            FROM movie_queue a
            JOIN movie_collection b ON a.collection_idx=b.idx
            JOIN imdb_ratings c ON b.show_id=c.index
            WHERE c.istv
            GROUP BY 1,2,3,4
            ORDER BY 1,2,3,4
        "#;
        self.get_pool()
            .get()?
            .query(query, &[])?
            .iter()
            .map(|row| TvShowsResult::from_row(row).map_err(Into::into))
            .collect()
    }

    pub fn get_new_episodes(
        &self,
        mindate: NaiveDate,
        maxdate: NaiveDate,
        source: &Option<TvShowSource>,
    ) -> Result<Vec<NewEpisodesResult>, Error> {
        let query = postgres_query::query_dyn!(
            &format!(
                r#"
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
                            cast(d.rating as double precision) as eprating,
                            d.eptitle
                    FROM imdb_ratings c
                    JOIN imdb_episodes d ON c.show = d.show
                    LEFT JOIN trakt_watched_episodes e
                        ON c.link=e.link AND d.season=e.season AND d.episode=e.episode
                    WHERE c.link in (SELECT link FROM active_links GROUP BY link) AND
                        e.episode is null AND
                        c.istv AND d.airdate >= $mindate AND
                        d.airdate <= $maxdate {}
                    GROUP BY 1,2,3,4,5,6,7,8,9,10
                    ORDER BY d.airdate, c.show, d.season, d.episode
                "#,
                match source {
                    Some(TvShowSource::All) => "",
                    Some(s) => "AND c.source = $source",
                    None => "AND c.source is null",
                }
            ),
            mindate = mindate,
            maxdate = maxdate
        )?;
        self.get_pool()
            .get()?
            .query(query.sql(), query.parameters())?
            .iter()
            .map(|row| NewEpisodesResult::from_row(row).map_err(Into::into))
            .collect()
    }

    pub fn find_new_episodes(
        &self,
        source: &Option<TvShowSource>,
        shows: &[String],
    ) -> Result<Vec<NewEpisodesResult>, Error> {
        let mindate = Local::today() + Duration::days(-14);
        let maxdate = Local::today() + Duration::days(7);

        let mq = MovieQueueDB::with_pool(&self.get_pool());

        let mut output = Vec::new();

        let episodes =
            self.get_new_episodes(mindate.naive_local(), maxdate.naive_local(), source)?;
        'outer: for epi in episodes {
            let movie_queue = mq.print_movie_queue(&[&epi.show])?;
            for s in movie_queue {
                if let Some(show) = &s.show {
                    if let Some(season) = &s.season {
                        if let Some(episode) = &s.episode {
                            if (show == &epi.show)
                                && (season == &epi.season)
                                && (episode == &epi.episode)
                            {
                                continue 'outer;
                            }
                        }
                    }
                }
            }
            if !shows.is_empty() && shows.iter().any(|s| &epi.show != s) {
                continue;
            }
            output.push(epi);
        }
        Ok(output)
    }

    pub fn get_collection_after_timestamp(
        &self,
        timestamp: DateTime<Utc>,
    ) -> Result<Vec<MovieCollectionRow>, Error> {
        let query = postgres_query::query!(
            r#"
                SELECT idx, path, show
                FROM movie_collection
                WHERE last_modified >= $timestamp
            "#,
            timestamp = timestamp
        );
        self.get_pool()
            .get()?
            .query(query.sql(), query.parameters())?
            .iter()
            .map(|row| MovieCollectionRow::from_row(row).map_err(Into::into))
            .collect()
    }
}

pub fn find_new_episodes_http_worker(
    pool: &PgPool,
    shows: Option<String>,
    source: &Option<TvShowSource>,
) -> Result<Vec<String>, Error> {
    let button_add = format!(
        "{}{}",
        r#"<td><button type="submit" id="ID" "#,
        format!(
            r#"onclick="imdb_update('SHOW', 'LINK', SEASON,
            '/list/cal{}');"
            >update database</button></td>"#,
            match source.as_ref() {
                Some(s) => format!("?source={}", s.to_string()),
                None => "".to_string(),
            }
        ),
    );

    let mc = MovieCollection::with_pool(&pool)?;
    let shows_filter: Option<HashSet<String>> =
        shows.map(|s| s.split(',').map(ToString::to_string).collect());

    let mindate = (Local::today() + Duration::days(-14)).naive_local();
    let maxdate = (Local::today() + Duration::days(7)).naive_local();

    let mq = MovieQueueDB::with_pool(&pool);

    let episodes = mc.get_new_episodes(mindate, maxdate, &source)?;

    let shows: HashSet<String> = episodes
        .iter()
        .filter_map(|s| {
            let show = s.show.to_string();
            match shows_filter.as_ref() {
                Some(f) => {
                    if f.contains(&show) {
                        Some(show)
                    } else {
                        None
                    }
                }
                None => Some(show),
            }
        })
        .collect();

    let mut queue = Vec::new();

    for show in shows {
        let movie_queue = mq.print_movie_queue(&[&show])?;
        for s in movie_queue {
            if let Some(u) = mc.get_collection_index(&s.path)? {
                queue.push((
                    (
                        s.show
                            .as_ref()
                            .map_or_else(|| "".to_string(), ToString::to_string),
                        s.season.unwrap_or(-1),
                        s.episode.unwrap_or(-1),
                    ),
                    u,
                ));
            }
        }
    }

    let queue: HashMap<(String, i32, i32), i32> = queue.into_iter().collect();

    let output = episodes
        .into_iter()
        .map(|epi| {
            let key = (epi.show.to_string(), epi.season, epi.episode);
            format!(
                "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td>{}</tr>",
                format!(
                    r#"<a href="javascript:updateMainArticle('/list/trakt/watched/list/{}/{}')">{}</a>"#,
                    epi.link, epi.season, epi.title
                ),
                match queue.get(&key) {
                    Some(idx) => format!(
                        r#"<a href="javascript:updateMainArticle('{}');">{}</a>"#,
                        &format!(r#"{}/{}"#, "/list/play", idx),
                        epi.eptitle
                    ),
                    None => epi.eptitle.to_string(),
                },
                format!(
                    r#"<a href="https://www.imdb.com/title/{}">s{} ep{}</a>"#,
                    epi.epurl, epi.season, epi.episode
                ),
                format!("rating: {:0.1} / {:0.1}", epi.eprating, epi.rating,),
                epi.airdate,
                button_add
                    .replace("SHOW", &epi.show)
                    .replace("LINK", &epi.link)
                    .replace("SEASON", &epi.season.to_string()),
            )
        })
        .collect();

    Ok(output)
}
