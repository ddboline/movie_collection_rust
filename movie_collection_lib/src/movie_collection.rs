use anyhow::{format_err, Error};
use chrono::{DateTime, Duration, Local, NaiveDate, Utc};
use derive_more::Into;
use futures::future::try_join_all;
use itertools::Itertools;
use postgres_query::{query, query_dyn, FromSqlRow};
use rayon::iter::{IntoParallelIterator, IntoParallelRefIterator, ParallelIterator};
use serde::{Deserialize, Serialize};
use stack_string::{format_sstr, StackString};
use std::{
    collections::{HashMap, HashSet},
    ffi::OsStr,
    fmt,
    fmt::Write,
    path::Path,
    sync::Arc,
};
use stdout_channel::{rate_limiter::RateLimiter, StdoutChannel};

use crate::{
    config::Config,
    imdb_episodes::{ImdbEpisodes, ImdbSeason},
    imdb_ratings::ImdbRatings,
    movie_queue::MovieQueueDB,
    pgpool::PgPool,
    tv_show_source::TvShowSource,
    utils::{option_string_wrapper, parse_file_stem, walk_directory},
};

#[derive(FromSqlRow)]
pub struct NewEpisodesResult {
    pub show: StackString,
    pub link: StackString,
    pub title: StackString,
    pub season: i32,
    pub episode: i32,
    pub epurl: StackString,
    pub airdate: NaiveDate,
    pub rating: f64,
    pub eprating: f64,
    pub eptitle: StackString,
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
    pub show: StackString,
    pub link: StackString,
    pub count: i64,
    pub title: StackString,
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
                .map_or(StackString::new(), StackString::from_display),
        )
    }
}

#[derive(Default, Serialize, Deserialize, FromSqlRow)]
pub struct MovieCollectionRow {
    pub idx: i32,
    pub path: StackString,
    pub show: StackString,
}

#[derive(Default, FromSqlRow)]
pub struct MovieCollectionResult {
    pub path: StackString,
    pub show: StackString,
    pub rating: f64,
    pub title: StackString,
    pub istv: bool,
    pub eprating: Option<f64>,
    pub season: Option<i32>,
    pub episode: Option<i32>,
    pub eptitle: Option<StackString>,
    pub epurl: Option<StackString>,
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
                option_string_wrapper(self.eptitle.as_ref()),
                option_string_wrapper(self.epurl.as_ref()),
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

#[derive(Debug, Clone)]
pub struct MovieCollection {
    pub config: Config,
    pub pool: PgPool,
    pub stdout: StdoutChannel<StackString>,
}

impl Default for MovieCollection {
    fn default() -> Self {
        Self::new(
            &Config::default(),
            &PgPool::default(),
            &StdoutChannel::default(),
        )
    }
}

impl MovieCollection {
    pub fn new(config: &Config, pool: &PgPool, stdout: &StdoutChannel<StackString>) -> Self {
        let config = config.clone();
        let pool = pool.clone();
        let stdout = stdout.clone();
        Self {
            pool,
            config,
            stdout,
        }
    }

    pub async fn print_imdb_shows(
        &self,
        show: &str,
        istv: bool,
    ) -> Result<Vec<ImdbRatings>, Error> {
        let query = format_sstr!("SELECT show FROM imdb_ratings WHERE show like '%{show}%'");
        let query = if istv {
            format_sstr!("{query} AND istv")
        } else {
            query
        };
        let shows: HashSet<StackString> = self
            .pool
            .get()
            .await?
            .query(query.as_str(), &[])
            .await?
            .iter()
            .map(|r| r.get(0))
            .collect();

        let shows = if shows.contains(show) {
            vec![show.into()]
        } else {
            shows.into_iter().sorted().collect()
        };

        let futures = shows.into_iter().map(|show| async move {
            #[derive(FromSqlRow)]
            struct TempImdbRating {
                index: i32,
                show: StackString,
                title: StackString,
                link: StackString,
                rating: f64,
            }

            let query = query_dyn!(
                &format_sstr!(
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
            let conn = self.pool.get().await?;
            let results: Vec<TempImdbRating> = query.fetch(&conn).await?;
            let results: Vec<_> = results
                .into_iter()
                .map(|row| ImdbRatings {
                    index: row.index,
                    show: row.show,
                    title: Some(row.title),
                    link: row.link,
                    rating: Some(row.rating),
                    ..ImdbRatings::default()
                })
                .collect();
            Ok(results)
        });
        let results: Result<Vec<_>, Error> = try_join_all(futures).await;
        Ok(results?.into_iter().flatten().collect())
    }

    pub async fn print_imdb_episodes(
        &self,
        show: &str,
        season: Option<i32>,
    ) -> Result<Vec<ImdbEpisodes>, Error> {
        ImdbEpisodes::get_episodes_by_show_season_episode(show, season, None, &self.pool).await
    }

    pub async fn print_imdb_all_seasons(&self, show: &str) -> Result<Vec<ImdbSeason>, Error> {
        ImdbSeason::get_seasons(show, &self.pool).await
    }

    pub async fn search_movie_collection(
        &self,
        search_strs: &[impl AsRef<str>],
    ) -> Result<Vec<MovieCollectionResult>, Error> {
        #[derive(FromSqlRow)]
        struct SearchMovieCollection {
            path: StackString,
            show: StackString,
            rating: f64,
            title: StackString,
            istv: Option<bool>,
        }
        let mut search_constr = StackString::new();
        if !search_strs.is_empty() {
            let search_strs = search_strs
                .iter()
                .map(|s| format_sstr!("a.path like '%{}%'", s.as_ref()))
                .join(" OR ");
            write!(search_constr, "AND ({search_strs})")?;
        }

        let query = query_dyn!(&format_sstr!(
            r#"
                SELECT a.path, a.show,
                COALESCE(b.rating, -1) as rating,
                COALESCE(b.title, '') as title,
                COALESCE(b.istv, FALSE) as istv
                FROM movie_collection a
                LEFT JOIN imdb_ratings b ON a.show_id = b.index
                WHERE a.is_deleted = false {search_constr}
            "#
        ),)?;
        let conn = self.pool.get().await?;
        let results: Vec<SearchMovieCollection> = query.fetch(&conn).await?;

        let futures = results.into_iter().map(|row| async {
            let mut result = MovieCollectionResult {
                path: row.path,
                show: row.show,
                rating: row.rating,
                title: row.title,
                istv: row.istv.unwrap_or(false),
                ..MovieCollectionResult::default()
            };
            let file_stem = Path::new(result.path.as_str())
                .file_stem()
                .ok_or_else(|| format_err!("No file stem"))?
                .to_string_lossy();
            let (show, season, episode) = parse_file_stem(&file_stem);

            if season != -1 && episode != -1 && show.as_str() == result.show.as_str() {
                #[derive(FromSqlRow)]
                struct TempImdbEpisodes {
                    eprating: Option<f64>,
                    eptitle: Option<StackString>,
                    epurl: Option<StackString>,
                }

                let query = query!(
                    r#"
                            SELECT cast(rating as double precision) as eprating, eptitle, epurl
                            FROM imdb_episodes
                            WHERE show = $show AND season = $season AND episode = $episode
                        "#,
                    show = show,
                    season = season,
                    episode = episode
                );
                let conn = self.pool.get().await?;
                let results: Vec<TempImdbEpisodes> = query.fetch(&conn).await?;

                for row in results {
                    result.season = Some(season);
                    result.episode = Some(episode);
                    result.eprating = row.eprating;
                    result.eptitle = row.eptitle;
                    result.epurl = row.epurl;
                }
            }
            Ok(result)
        });
        let results: Result<Vec<_>, Error> = try_join_all(futures).await;
        let mut results = results?;
        results.sort_by_key(|r| (r.season, r.episode));
        Ok(results)
    }

    pub async fn remove_from_collection(&self, path: &str) -> Result<(), Error> {
        let query = query!(
            r#"UPDATE movie_collection SET is_deleted=true WHERE path = $path"#,
            path = path
        );
        let conn = self.pool.get().await?;
        query.execute(&conn).await.map(|_| ()).map_err(Into::into)
    }

    pub async fn get_collection_index(&self, path: &str) -> Result<Option<i32>, Error> {
        let query = query!(
            r#"SELECT idx FROM movie_collection WHERE path = $path"#,
            path = path
        );
        let conn = self.pool.get().await?;
        let id = query.fetch_opt(&conn).await?;
        Ok(id.map(|(x,)| x))
    }

    pub async fn get_collection_path(&self, idx: i32) -> Result<StackString, Error> {
        let query = query!(
            "SELECT path FROM movie_collection WHERE idx = $idx",
            idx = idx
        );
        let conn = self.pool.get().await?;
        let (path,) = query.fetch_one(&conn).await?;
        Ok(path)
    }

    pub async fn insert_into_collection(&self, path: &str, check_path: bool) -> Result<(), Error> {
        if check_path && !Path::new(&path).exists() {
            return Err(format_err!("No such file"));
        }
        let conn = self.pool.get().await?;
        if let Some(idx) = self.get_collection_index(path).await? {
            let query = query!(
                "UPDATE movie_collection SET is_deleted=false WHERE idx=$idx",
                idx = idx
            );
            query.execute(&conn).await?;
        } else {
            let file_stem = Path::new(&path)
                .file_stem()
                .ok_or_else(|| format_err!("No file stem"))?
                .to_string_lossy();
            let (show, _, _) = parse_file_stem(&file_stem);
            let query = query!(
                r#"
                    INSERT INTO movie_collection (path, show, last_modified)
                    VALUES ($path, $show, now())
                "#,
                path = path,
                show = show
            );
            query.execute(&conn).await?;
        }
        Ok(())
    }

    pub async fn fix_collection_show_id(&self) -> Result<u64, Error> {
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
        let rows = self.pool.get().await?.execute(query, &[]).await?;
        Ok(rows)
    }

    pub async fn make_collection(&self) -> Result<(), Error> {
        let file_list: Result<Vec<_>, Error> = self
            .config
            .movie_dirs
            .par_iter()
            .filter(|d| d.exists())
            .map(|d| walk_directory(d, &self.config.suffixes))
            .collect();
        let file_list = file_list?;

        if file_list.is_empty() {
            return Ok(());
        }

        let file_list: HashSet<_> = file_list
            .into_iter()
            .flatten()
            .map(|f| f.to_string_lossy().into_owned())
            .collect();
        let file_list = Arc::new(file_list);

        let episode_list: Result<HashSet<(StackString, _, _, _)>, Error> = file_list
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
        let movie_queue: Result<HashMap<StackString, i32>, Error> = self
            .pool
            .get()
            .await?
            .query(query, &[])
            .await?
            .iter()
            .map(|row| {
                let path: StackString = row.try_get("path")?;
                let idx: i32 = row.try_get("idx")?;
                Ok((path, idx))
            })
            .collect();
        let movie_queue = Arc::new(movie_queue?);

        let query = "SELECT path, show FROM movie_collection";
        let collection_map: Result<HashMap<StackString, StackString>, Error> = self
            .pool
            .get()
            .await?
            .query(query, &[])
            .await?
            .iter()
            .map(|row| {
                let path: StackString = row.try_get("path")?;
                let show: StackString = row.try_get("show")?;
                Ok((path, show))
            })
            .collect();
        let collection_map = Arc::new(collection_map?);

        let query = "SELECT show, season, episode from imdb_episodes";
        let episodes_set: Result<HashSet<(StackString, i32, i32)>, Error> = self
            .pool
            .get()
            .await?
            .query(query, &[])
            .await?
            .iter()
            .map(|row| {
                let show: StackString = row.try_get("show")?;
                let season: i32 = row.try_get("season")?;
                let episode: i32 = row.try_get("episode")?;
                Ok((show, season, episode))
            })
            .collect();
        let episodes_set = episodes_set?;
        let rate_limiter = RateLimiter::new(10, 100);

        let futures = file_list.iter().map(|f| {
            let collection_map = collection_map.clone();
            let rate_limiter = rate_limiter.clone();
            async move {
                if collection_map.get(f.as_str()).is_none() {
                    let ext = Path::new(f)
                        .extension()
                        .map(OsStr::to_string_lossy)
                        .ok_or_else(|| format_err!("extension fail"))?
                        .as_ref()
                        .into();
                    if self.config.suffixes.contains(&ext) {
                        self.stdout.send(format_sstr!("not in collection {f}"));
                        rate_limiter.acquire().await;
                        self.insert_into_collection(f, true).await?;
                    }
                }
                Ok(())
            }
        });
        let results: Result<Vec<_>, Error> = try_join_all(futures).await;
        results?;

        let futures = collection_map.iter().map(|(key, val)| {
            let file_list = file_list.clone();
            let movie_queue = movie_queue.clone();
            let rate_limiter = rate_limiter.clone();
            async move {
                if !file_list.contains(key.as_str()) {
                    if let Some(v) = movie_queue.get(key) {
                        self.stdout
                            .send(format_sstr!("in queue but not disk {key} {v}"));
                        let mq = MovieQueueDB::new(&self.config, &self.pool, &self.stdout);
                        mq.remove_from_queue_by_path(key).await?;
                    } else {
                        self.stdout.send(format_sstr!("not on disk {key} {val}"));
                    }
                    rate_limiter.acquire().await;
                    self.remove_from_collection(key).await?;
                }
                Ok(())
            }
        });
        let results: Result<Vec<_>, Error> = try_join_all(futures).await;
        results?;

        for (key, val) in collection_map.iter() {
            if !file_list.contains(key.as_str()) {
                if movie_queue.contains_key(key.as_str()) {
                    self.stdout
                        .send(format_sstr!("in queue but not disk {key}"));
                } else {
                    self.stdout.send(format_sstr!("not on disk {key} {val}"));
                }
            }
        }

        let shows_not_in_db: HashSet<StackString> = episode_list
            .into_par_iter()
            .filter_map(|(show, season, episode, _)| {
                let key = (show.clone(), season, episode);
                if episodes_set.contains(&key) {
                    None
                } else {
                    Some(show)
                }
            })
            .collect();

        for show in shows_not_in_db {
            self.stdout
                .send(format_sstr!("show has episode not in db {show} "));
        }
        Ok(())
    }

    pub async fn get_imdb_show_map(&self) -> Result<HashMap<StackString, ImdbRatings>, Error> {
        #[derive(FromSqlRow)]
        struct ImdbShowMap {
            link: StackString,
            show: StackString,
            title: StackString,
            rating: f64,
            istv: bool,
            source: Option<StackString>,
        }

        let query = r#"
            SELECT link, show, title, rating, istv, source
            FROM imdb_ratings
            WHERE link IS NOT null AND rating IS NOT null
        "#;

        self.pool
            .get()
            .await?
            .query(query, &[])
            .await?
            .iter()
            .map(|row| {
                let row = ImdbShowMap::from_row(row)?;

                let source: Option<TvShowSource> = match row.source {
                    Some(s) => s.parse().ok(),
                    None => None,
                };

                Ok((
                    row.link.clone(),
                    ImdbRatings {
                        show: row.show,
                        title: Some(row.title),
                        link: row.link,
                        rating: Some(row.rating),
                        istv: Some(row.istv),
                        source,
                        ..ImdbRatings::default()
                    },
                ))
            })
            .collect()
    }

    pub async fn print_tv_shows(&self) -> Result<Vec<TvShowsResult>, Error> {
        let query = query!(
            r#"
            SELECT b.show, c.link, c.title, c.source, count(*) as count
            FROM movie_queue a
            JOIN movie_collection b ON a.collection_idx=b.idx
            JOIN imdb_ratings c ON b.show_id=c.index
            WHERE c.istv
            GROUP BY 1,2,3,4
            ORDER BY 1,2,3,4
        "#
        );
        let conn = self.pool.get().await?;
        query.fetch(&conn).await.map_err(Into::into)
    }

    pub async fn get_new_episodes(
        &self,
        mindate: NaiveDate,
        maxdate: NaiveDate,
        source: Option<TvShowSource>,
    ) -> Result<Vec<NewEpisodesResult>, Error> {
        let mut source_str = StackString::new();
        match source {
            Some(TvShowSource::All) => (),
            Some(s) => write!(source_str, "AND c.source = '{}'", s)?,
            None => write!(source_str, "AND c.source is null")?,
        }
        let query = query_dyn!(
            &format_sstr!(
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
                        d.airdate <= $maxdate {source_str}
                    GROUP BY 1,2,3,4,5,6,7,8,9,10
                    ORDER BY d.airdate, c.show, d.season, d.episode
                "#
            ),
            mindate = mindate,
            maxdate = maxdate
        )?;
        let conn = self.pool.get().await?;
        query.fetch(&conn).await.map_err(Into::into)
    }

    pub async fn find_new_episodes(
        &self,
        source: Option<TvShowSource>,
        shows: &[impl AsRef<str>],
    ) -> Result<Vec<NewEpisodesResult>, Error> {
        let mindate = Local::today() + Duration::days(-14);
        let maxdate = Local::today() + Duration::days(7);

        let mq = MovieQueueDB::new(&self.config, &self.pool, &self.stdout);

        let mut output = Vec::new();

        let episodes = self
            .get_new_episodes(mindate.naive_local(), maxdate.naive_local(), source)
            .await?;
        'outer: for epi in episodes {
            let movie_queue = mq.print_movie_queue(&[epi.show.as_str()]).await?;
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
            if !shows.is_empty() && shows.iter().any(|s| epi.show.as_str() != s.as_ref()) {
                continue;
            }
            output.push(epi);
        }
        Ok(output)
    }

    pub async fn get_collection_after_timestamp(
        &self,
        timestamp: DateTime<Utc>,
    ) -> Result<Vec<MovieCollectionRow>, Error> {
        let query = query!(
            r#"
                SELECT idx, path, show
                FROM movie_collection
                WHERE last_modified >= $timestamp
            "#,
            timestamp = timestamp
        );
        let conn = self.pool.get().await?;
        query.fetch(&conn).await.map_err(Into::into)
    }

    pub async fn match_file_pattern(
        &self,
        patterns: &[impl AsRef<str>],
    ) -> Result<Vec<StackString>, Error> {
        #[derive(FromSqlRow, Into)]
        struct Wrap(StackString);

        let constr = patterns
            .iter()
            .map(|p| {
                let p = p.as_ref();
                format_sstr!("path like '%{p}%'")
            })
            .join(" OR ");
        let where_str = if constr.is_empty() {
            StackString::new()
        } else {
            format_sstr!("WHERE {constr}")
        };
        let query = query_dyn!(&format_sstr!(
            "SELECT path FROM movie_collection {where_str}"
        ))?;
        let conn = self.pool.get().await?;
        let wrap: Vec<Wrap> = query.fetch(&conn).await?;
        Ok(wrap.into_iter().map(Into::into).collect())
    }
}

pub async fn find_new_episodes_http_worker(
    config: &Config,
    pool: &PgPool,
    stdout: &StdoutChannel<StackString>,
    shows: Option<impl AsRef<str>>,
    source: Option<TvShowSource>,
) -> Result<Vec<StackString>, Error> {
    let mut source_str = StackString::new();
    if let Some(s) = source.as_ref() {
        write!(source_str, "?source={}", s)?;
    }
    let mut button_add = StackString::new();
    write!(
        button_add,
        "{}{}",
        r#"<td><button type="submit" id="ID" "#,
        format_sstr!(
            r#"onclick="imdb_update('SHOW', 'LINK', SEASON,
            '/list/cal{source_str}');"
            >update database</button></td>"#
        ),
    )?;

    let mc = MovieCollection::new(config, pool, stdout);
    let shows_filter: Option<HashSet<StackString>> =
        shows.map(|s| s.as_ref().split(',').map(Into::into).collect());

    let mindate = (Local::today() + Duration::days(-14)).naive_local();
    let maxdate = (Local::today() + Duration::days(7)).naive_local();

    let mq = MovieQueueDB::new(config, pool, stdout);

    let episodes = mc.get_new_episodes(mindate, maxdate, source).await?;

    let shows: HashSet<StackString> = episodes
        .iter()
        .filter_map(|s| {
            let show = s.show.clone();
            match shows_filter.as_ref() {
                Some(f) => {
                    if f.contains(show.as_str()) {
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
        let movie_queue = mq.print_movie_queue(&[&show]).await?;
        for s in movie_queue {
            if let Some(u) = mc.get_collection_index(&s.path).await? {
                queue.push((
                    (
                        s.show.clone().unwrap_or_else(|| "".into()),
                        s.season.unwrap_or(-1),
                        s.episode.unwrap_or(-1),
                    ),
                    u,
                ));
            }
        }
    }

    let queue: HashMap<(StackString, i32, i32), i32> = queue.into_iter().collect();

    let output = episodes
        .into_iter()
        .map(|epi| {
            let season_str = StackString::from_display(epi.season);
            let link_str = &epi.link;
            let title = &epi.title;
            let epurl = &epi.epurl;
            let episode = epi.episode;
            let key = (epi.show.clone(), epi.season, episode);
            let airdate = epi.airdate;
            format_sstr!(
                "<tr><td>{a}</td><td>{b}</td><td>{c}</td><td>{d}</td><td>{airdate}</td>{e}</tr>",
                a=format_sstr!(
                    r#"<a href="javascript:updateMainArticle('/trakt/watched/list/{link_str}/{season_str}')">{title}</a>"#
                ),
                b=match queue.get(&key) {
                    Some(idx) => format_sstr!(
                        r#"<a href="javascript:updateMainArticle('{}');">{}</a>"#,
                        &format_sstr!(r#"/list/play/{idx}"#),
                        epi.eptitle
                    ),
                    None => epi.eptitle.clone(),
                },
                c=format_sstr!(
                    r#"<a href="https://www.imdb.com/title/{epurl}" target="_blank">s{season_str:02} ep{episode:02}</a>"#
                ),
                d=format_sstr!("rating: {:0.1} / {:0.1}", epi.eprating, epi.rating,),
                e=button_add
                    .replace("SHOW", &epi.show)
                    .replace("LINK", link_str)
                    .replace("SEASON", &season_str),
            )
        })
        .collect();

    Ok(output)
}

#[derive(Serialize, Deserialize)]
pub struct LastModifiedResponse {
    pub table: StackString,
    pub last_modified: DateTime<Utc>,
}

impl LastModifiedResponse {
    pub async fn get_last_modified(pool: &PgPool) -> Result<Vec<Self>, Error> {
        let tables = vec![
            "imdb_episodes",
            "imdb_ratings",
            "movie_collection",
            "movie_queue",
            "plex_event",
            "plex_filename",
        ];

        let futures = tables.into_iter().map(|table| async move {
            let query = query_dyn!(&format_sstr!("SELECT max(last_modified) FROM {table}"))?;
            let conn = pool.get().await?;
            if let Some((last_modified,)) = query.fetch_opt(&conn).await? {
                let last_modified: DateTime<Utc> = last_modified;
                Ok(Some(LastModifiedResponse {
                    table: (*table).into(),
                    last_modified,
                }))
            } else {
                Ok(None)
            }
        });
        let results: Result<Vec<_>, Error> = try_join_all(futures).await;
        let results: Vec<_> = results?.into_iter().flatten().collect();
        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use anyhow::Error;
    use stdout_channel::{MockStdout, StdoutChannel};

    use crate::{config::Config, movie_collection::MovieCollection, pgpool::PgPool};

    #[tokio::test]
    #[ignore]
    async fn test_match_file_pattern() -> Result<(), Error> {
        let config = Config::with_config()?;
        let pool = PgPool::new(&config.pgurl);
        let mock_stdout = MockStdout::new();
        let stdout = StdoutChannel::with_mock_stdout(mock_stdout.clone(), mock_stdout.clone());

        let mc = MovieCollection::new(&config, &pool, &stdout);
        let files = mc
            .match_file_pattern(&[
                "/media/seagate4000/Documents/movies/scifi/star_trek_2_the_wrath_of_khan.mp4",
                "/media/western2000/Documents/movies/scifi/star_trek_2_the_wrath_of_khan.mp4",
            ])
            .await?;
        assert_eq!(files.len(), 2);
        Ok(())
    }
}
