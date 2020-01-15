use anyhow::{format_err, Error};
use chrono::NaiveDate;
use log::debug;
use postgres_query::FromSqlRow;
use rayon::iter::{IntoParallelRefIterator, ParallelIterator};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::io;
use std::io::Write;

use crate::imdb_episodes::ImdbEpisodes;
use crate::imdb_ratings::ImdbRatings;
use crate::movie_collection::MovieCollection;
use crate::movie_queue::MovieQueueDB;
use crate::pgpool::PgPool;

use crate::trakt_instance;
use crate::tv_show_source::TvShowSource;
use crate::utils::option_string_wrapper;

#[derive(Clone, Copy)]
pub enum TraktActions {
    None,
    List,
    Add,
    Remove,
}

impl TraktActions {
    pub fn from_command(command: &str) -> Self {
        match command {
            "list" => Self::List,
            "add" => Self::Add,
            "rm" | "del" => Self::Remove,
            _ => Self::None,
        }
    }
}

pub enum TraktCommands {
    None,
    Calendar,
    WatchList,
    Watched,
}

impl TraktCommands {
    pub fn from_command(command: &str) -> Self {
        match command {
            "cal" | "calendar" => Self::Calendar,
            "watchlist" => Self::WatchList,
            "watched" => Self::Watched,
            _ => Self::None,
        }
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub struct TraktCalEntry {
    pub ep_link: Option<String>,
    pub episode: i32,
    pub link: String,
    pub season: i32,
    pub show: String,
    pub airdate: NaiveDate,
}

impl fmt::Display for TraktCalEntry {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "{} {} {} {} {} {}",
            self.show,
            self.link,
            self.season,
            self.episode,
            option_string_wrapper(&self.ep_link),
            self.airdate,
        )
    }
}

pub type TraktCalEntryList = Vec<TraktCalEntry>;

#[derive(Serialize, Deserialize, Debug, Default)]
pub struct TraktResult {
    pub status: String,
}

impl fmt::Display for TraktResult {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "status {}", self.status)
    }
}

#[derive(Serialize, Deserialize, Debug, Default, FromSqlRow)]
pub struct WatchListShow {
    pub link: String,
    pub title: String,
    pub year: i32,
}

impl fmt::Display for WatchListShow {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{} {} {}", self.link, self.title, self.year,)
    }
}

impl WatchListShow {
    pub fn get_show_by_link(link: &str, pool: &PgPool) -> Result<Option<Self>, Error> {
        let query = postgres_query::query!(
            "SELECT title, year FROM trakt_watchlist WHERE link = $link",
            link = link
        );
        if let Some(row) = pool.get()?.query(query.sql(), query.parameters())?.get(0) {
            let title: String = row.try_get("title")?;
            let year: i32 = row.try_get("year")?;
            Ok(Some(Self {
                link: link.to_string(),
                title,
                year,
            }))
        } else {
            Ok(None)
        }
    }

    pub fn get_index(&self, pool: &PgPool) -> Result<Option<i32>, Error> {
        let query = postgres_query::query!(
            "SELECT id FROM trakt_watchlist WHERE link = $link",
            link = self.link
        );
        if let Some(row) = pool.get()?.query(query.sql(), query.parameters())?.get(0) {
            let id: i32 = row.try_get("id")?;
            Ok(Some(id))
        } else {
            Ok(None)
        }
    }

    pub fn insert_show(&self, pool: &PgPool) -> Result<(), Error> {
        let query = postgres_query::query!(
            "INSERT INTO trakt_watchlist (link, title, year) VALUES ($link, $title, $year)",
            link = self.link,
            title = self.title,
            year = self.year
        );
        pool.get()?
            .execute(query.sql(), query.parameters())
            .map(|_| ())
            .map_err(Into::into)
    }

    pub fn delete_show(&self, pool: &PgPool) -> Result<(), Error> {
        let query = postgres_query::query!(
            "DELETE FROM trakt_watchlist WHERE link=$link",
            link = self.link
        );
        pool.get()?
            .execute(query.sql(), query.parameters())
            .map(|_| ())
            .map_err(Into::into)
    }
}

pub fn get_watchlist_shows_db(pool: &PgPool) -> Result<HashMap<String, WatchListShow>, Error> {
    let query = r#"
        SELECT a.link, a.title, a.year
        FROM trakt_watchlist a
    "#;
    pool.get()?
        .query(query, &[])?
        .iter()
        .map(|row| {
            let val = WatchListShow::from_row(row)?;
            Ok((val.link.to_string(), val))
        })
        .collect()
}

pub type WatchListMap = HashMap<String, (String, WatchListShow, Option<TvShowSource>)>;

pub fn get_watchlist_shows_db_map(pool: &PgPool) -> Result<WatchListMap, Error> {
    let query = r#"
        SELECT b.show, a.link, a.title, a.year, b.source
        FROM trakt_watchlist a
        JOIN imdb_ratings b ON a.link=b.link
    "#;

    #[derive(FromSqlRow)]
    struct WatchlistShowDbMap {
        show: String,
        link: String,
        title: String,
        year: i32,
        source: Option<String>,
    }

    pool.get()?
        .query(query, &[])?
        .iter()
        .map(|row| {
            let row = WatchlistShowDbMap::from_row(row)?;

            let source: Option<TvShowSource> = match row.source {
                Some(s) => s.parse().ok(),
                None => None,
            };

            Ok((
                row.link.to_string(),
                (
                    row.show,
                    WatchListShow {
                        link: row.link,
                        title: row.title,
                        year: row.year,
                    },
                    source,
                ),
            ))
        })
        .collect()
}

#[derive(Serialize, Deserialize, Debug, Default, PartialEq, Eq, Hash)]
pub struct WatchedEpisode {
    pub title: String,
    pub imdb_url: String,
    pub episode: i32,
    pub season: i32,
}

impl fmt::Display for WatchedEpisode {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "{} {} {} {}",
            self.title, self.imdb_url, self.season, self.episode
        )
    }
}

impl WatchedEpisode {
    pub fn get_index(&self, pool: &PgPool) -> Result<Option<i32>, Error> {
        let query = postgres_query::query!(
            r#"
                SELECT id
                FROM trakt_watched_episodes
                WHERE link=$link AND season=$season AND episode=$episode
            "#,
            link = self.imdb_url,
            season = self.season,
            episode = self.episode
        );
        if let Some(row) = pool.get()?.query(query.sql(), query.parameters())?.get(0) {
            let id: i32 = row.try_get("id")?;
            Ok(Some(id))
        } else {
            Ok(None)
        }
    }

    pub fn get_watched_episode(
        pool: &PgPool,
        link: &str,
        season: i32,
        episode: i32,
    ) -> Result<Option<Self>, Error> {
        let query = postgres_query::query!(
            r#"
                SELECT a.link, b.title
                FROM trakt_watched_episodes a
                JOIN imdb_ratings b ON a.link = b.link
                WHERE a.link = $link AND a.season = $season AND a.episode = $episode
            "#,
            link = link,
            season = season,
            episode = episode
        );
        if let Some(row) = pool.get()?.query(query.sql(), query.parameters())?.get(0) {
            let imdb_url: String = row.try_get("link")?;
            let title: String = row.try_get("title")?;
            Ok(Some(Self {
                title,
                imdb_url,
                season,
                episode,
            }))
        } else {
            Ok(None)
        }
    }

    pub fn insert_episode(&self, pool: &PgPool) -> Result<(), Error> {
        let query = postgres_query::query!(
            r#"
                INSERT INTO trakt_watched_episodes (link, season, episode)
                VALUES ($link, $season, $episode)
            "#,
            link = self.imdb_url,
            season = self.season,
            episode = self.episode
        );
        pool.get()?
            .execute(query.sql(), query.parameters())
            .map(|_| ())
            .map_err(Into::into)
    }

    pub fn delete_episode(&self, pool: &PgPool) -> Result<(), Error> {
        let query = postgres_query::query!(
            r#"
            DELETE FROM trakt_watched_episodes
            WHERE link=$link AND season=$season AND episode=$episode
        "#,
            link = self.imdb_url,
            season = self.season,
            episode = self.episode
        );
        pool.get()?
            .execute(query.sql(), query.parameters())
            .map(|_| ())
            .map_err(Into::into)
    }
}

pub fn get_watched_shows_db(
    pool: &PgPool,
    show: &str,
    season: Option<i32>,
) -> Result<Vec<WatchedEpisode>, Error> {
    let mut where_vec = Vec::new();
    if !show.is_empty() {
        where_vec.push(format!("show='{}'", show));
    }
    if let Some(season) = season {
        where_vec.push(format!("season={}", season));
    }

    let where_str = if where_vec.is_empty() {
        "".to_string()
    } else {
        format!("WHERE {}", where_vec.join(" AND "))
    };

    let query = postgres_query::query_dyn!(&format!(
        r#"
            SELECT a.link, b.title, a.season, a.episode
            FROM trakt_watched_episodes a
            JOIN imdb_ratings b ON a.link = b.link
            {}
            ORDER BY 2,3,4
        "#,
        where_str
    ))?;

    pool.get()?
        .query(query.sql(), &[])?
        .iter()
        .map(|row| {
            let imdb_url: String = row.try_get("link")?;
            let title: String = row.try_get("title")?;
            let season: i32 = row.try_get("season")?;
            let episode: i32 = row.try_get("episode")?;
            Ok(WatchedEpisode {
                title,
                imdb_url,
                season,
                episode,
            })
        })
        .collect()
}

#[derive(Serialize, Deserialize, Debug, Default, PartialEq, Eq, Hash)]
pub struct WatchedMovie {
    pub title: String,
    pub imdb_url: String,
}

impl fmt::Display for WatchedMovie {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{} {}", self.title, self.imdb_url)
    }
}

impl WatchedMovie {
    pub fn get_index(&self, pool: &PgPool) -> Result<Option<i32>, Error> {
        let query = postgres_query::query!(
            r#"
                SELECT id
                FROM trakt_watched_movies
                WHERE link=$link
            "#,
            link = self.imdb_url
        );
        if let Some(row) = pool.get()?.query(query.sql(), query.parameters())?.get(0) {
            let id: i32 = row.try_get("id")?;
            Ok(Some(id))
        } else {
            Ok(None)
        }
    }

    pub fn get_watched_movie(pool: &PgPool, link: &str) -> Result<Option<Self>, Error> {
        let query = postgres_query::query!(
            r#"
                SELECT a.link, b.title
                FROM trakt_watched_movies a
                JOIN imdb_ratings b ON a.link = b.link
                WHERE a.link = $link
            "#,
            link = link
        );
        if let Some(row) = pool.get()?.query(query.sql(), query.parameters())?.get(0) {
            let imdb_url: String = row.try_get("link")?;
            let title: String = row.try_get("title")?;
            Ok(Some(Self { title, imdb_url }))
        } else {
            Ok(None)
        }
    }

    pub fn insert_movie(&self, pool: &PgPool) -> Result<(), Error> {
        let query = postgres_query::query!(
            r#"
                INSERT INTO trakt_watched_movies (link)
                VALUES ($link)
            "#,
            link = self.imdb_url
        );
        pool.get()?
            .execute(query.sql(), query.parameters())
            .map(|_| ())
            .map_err(Into::into)
    }

    pub fn delete_movie(&self, pool: &PgPool) -> Result<(), Error> {
        let query = postgres_query::query!(
            r#"
                DELETE FROM trakt_watched_movies
                WHERE link=$link
            "#,
            link = self.imdb_url
        );
        pool.get()?
            .execute(query.sql(), query.parameters())
            .map(|_| ())
            .map_err(Into::into)
    }
}

pub fn get_watched_movies_db(pool: &PgPool) -> Result<Vec<WatchedMovie>, Error> {
    let query = postgres_query::query!(
        r#"
            SELECT a.link, b.title
            FROM trakt_watched_movies a
            JOIN imdb_ratings b ON a.link = b.link
            ORDER BY b.show
        "#
    );
    pool.get()?
        .query(query.sql(), &[])?
        .iter()
        .map(|row| {
            let imdb_url: String = row.try_get("link")?;
            let title: String = row.try_get("title")?;
            Ok(WatchedMovie { title, imdb_url })
        })
        .collect()
}

pub fn sync_trakt_with_db() -> Result<(), Error> {
    let mc = MovieCollection::new();

    let watchlist_shows_db = get_watchlist_shows_db(&mc.pool)?;
    let watchlist_shows = trakt_instance::get_watchlist_shows()?;
    if watchlist_shows.is_empty() {
        return Ok(());
    }

    let stdout = io::stdout();

    let results: Result<Vec<_>, Error> = watchlist_shows
        .par_iter()
        .map(|(link, show)| {
            if !watchlist_shows_db.contains_key(link) {
                show.insert_show(&mc.pool)?;
                writeln!(stdout.lock(), "insert watchlist {}", show)?;
            }
            Ok(())
        })
        .collect();
    results?;

    let watched_shows_db: HashMap<(String, i32, i32), _> =
        get_watched_shows_db(&mc.pool, "", None)?
            .into_iter()
            .map(|s| ((s.imdb_url.to_string(), s.season, s.episode), s))
            .collect();
    let watched_shows = trakt_instance::get_watched_shows()?;
    if watched_shows.is_empty() {
        return Ok(());
    }
    let results: Result<Vec<_>, Error> = watched_shows
        .par_iter()
        .map(|(key, episode)| {
            if !watched_shows_db.contains_key(&key) {
                episode.insert_episode(&mc.pool)?;
                writeln!(stdout.lock(), "insert watched {}", episode)?;
            }
            Ok(())
        })
        .collect();
    results?;

    let watched_movies_db: HashMap<String, _> = get_watched_movies_db(&mc.pool)?
        .into_iter()
        .map(|s| (s.imdb_url.to_string(), s))
        .collect();
    let watched_movies = trakt_instance::get_watched_movies()?;
    if watched_movies.is_empty() {
        return Ok(());
    }
    let results: Result<Vec<_>, Error> = watched_movies
        .par_iter()
        .map(|(key, movie)| {
            if !watched_movies_db.contains_key(key) {
                movie.insert_movie(&mc.pool)?;
                writeln!(stdout.lock(), "insert watched {}", movie)?;
            }
            Ok(())
        })
        .collect();
    results?;

    watched_movies_db
        .par_iter()
        .map(|(key, movie)| {
            if !watched_movies.contains_key(key) {
                movie.delete_movie(&mc.pool)?;
                writeln!(stdout.lock(), "delete watched {}", movie)?;
            }
            Ok(())
        })
        .collect()
}

fn get_imdb_url_from_show(
    mc: &MovieCollection,
    show: Option<&str>,
) -> Result<Option<String>, Error> {
    let stdout = io::stdout();

    let result = if let Some(show) = show {
        let imdb_shows = mc.print_imdb_shows(show, false)?;
        if imdb_shows.len() > 1 {
            for show in imdb_shows {
                writeln!(stdout.lock(), "{}", show)?;
            }
            None
        } else {
            Some(imdb_shows[0].link.to_string())
        }
    } else {
        None
    };
    Ok(result)
}

fn trakt_cal_list(mc: &MovieCollection) -> Result<(), Error> {
    let cal_entries = trakt_instance::get_calendar()?;
    for cal in cal_entries {
        let show = match ImdbRatings::get_show_by_link(&cal.link, &mc.pool)? {
            Some(s) => s.show,
            None => "".to_string(),
        };
        let exists = if show.is_empty() {
            false
        } else {
            ImdbEpisodes {
                show: show.to_string(),
                season: cal.season,
                episode: cal.episode,
                ..ImdbEpisodes::default()
            }
            .get_index(&mc.pool)?
            .is_some()
        };
        if !exists {
            writeln!(io::stdout().lock(), "{} {}", show, cal)?;
        }
    }
    Ok(())
}

fn watchlist_add(mc: &MovieCollection, show: Option<&str>) -> Result<(), Error> {
    if let Some(imdb_url) = get_imdb_url_from_show(&mc, show)? {
        writeln!(
            io::stdout().lock(),
            "result: {}",
            trakt_instance::add_watchlist_show(&imdb_url)?
        )?;
        debug!("GOT HERE");
        if let Some(show) = trakt_instance::get_watchlist_shows()?.get(&imdb_url) {
            debug!("INSERT SHOW {}", show);
            show.insert_show(&mc.pool)?;
        }
    }
    Ok(())
}

fn watchlist_rm(mc: &MovieCollection, show: Option<&str>) -> Result<(), Error> {
    if let Some(imdb_url) = get_imdb_url_from_show(&mc, show)? {
        writeln!(
            io::stdout().lock(),
            "result: {}",
            trakt_instance::remove_watchlist_show(&imdb_url)?
        )?;
        if let Some(show) = WatchListShow::get_show_by_link(&imdb_url, &mc.pool)? {
            show.delete_show(&mc.pool)?;
        }
    }
    Ok(())
}

fn watchlist_list(mc: &MovieCollection) -> Result<(), Error> {
    let show_map = get_watchlist_shows_db(&mc.pool)?;
    for (_, show) in show_map {
        writeln!(io::stdout().lock(), "{}", show)?;
    }
    Ok(())
}

fn watched_add(
    mc: &MovieCollection,
    show: Option<&str>,
    season: i32,
    episode: &[i32],
) -> Result<(), Error> {
    if let Some(imdb_url) = get_imdb_url_from_show(&mc, show)? {
        if season != -1 && !episode.is_empty() {
            for epi in episode {
                trakt_instance::add_episode_to_watched(&imdb_url, season, *epi)?;
                WatchedEpisode {
                    imdb_url: imdb_url.to_string(),
                    season,
                    episode: *epi,
                    ..WatchedEpisode::default()
                }
                .insert_episode(&mc.pool)?;
            }
        } else {
            trakt_instance::add_movie_to_watched(&imdb_url)?;
            WatchedMovie {
                imdb_url,
                title: "".to_string(),
            }
            .insert_movie(&mc.pool)?;
        }
    }
    Ok(())
}

fn watched_rm(
    mc: &MovieCollection,
    show: Option<&str>,
    season: i32,
    episode: &[i32],
) -> Result<(), Error> {
    if let Some(imdb_url) = get_imdb_url_from_show(&mc, show)? {
        if season != -1 && !episode.is_empty() {
            for epi in episode {
                trakt_instance::remove_episode_to_watched(&imdb_url, season, *epi)?;
                if let Some(epi_) =
                    WatchedEpisode::get_watched_episode(&mc.pool, &imdb_url, season, *epi)?
                {
                    epi_.delete_episode(&mc.pool)?;
                }
            }
        } else {
            trakt_instance::remove_movie_to_watched(&imdb_url)?;
            if let Some(movie) = WatchedMovie::get_watched_movie(&mc.pool, &imdb_url)? {
                movie.delete_movie(&mc.pool)?;
            }
        }
    }
    Ok(())
}

fn watched_list(mc: &MovieCollection, show: Option<&str>, season: i32) -> Result<(), Error> {
    let watched_shows = get_watched_shows_db(&mc.pool, "", None)?;
    let watched_movies = get_watched_movies_db(&mc.pool)?;

    if let Some(imdb_url) = get_imdb_url_from_show(&mc, show)? {
        for show in &watched_shows {
            if season != -1 && show.season != season {
                continue;
            }
            if show.imdb_url == imdb_url {
                writeln!(io::stdout().lock(), "{}", show)?;
            }
        }
        for show in &watched_movies {
            if show.imdb_url == imdb_url {
                writeln!(io::stdout().lock(), "{}", show)?;
            }
        }
    } else {
        for show in &watched_shows {
            writeln!(io::stdout().lock(), "{}", show)?;
        }
        for show in &watched_movies {
            writeln!(io::stdout().lock(), "{}", show)?;
        }
    }
    Ok(())
}

pub fn trakt_app_parse(
    trakt_command: &TraktCommands,
    trakt_action: TraktActions,
    show: Option<&str>,
    season: i32,
    episode: &[i32],
) -> Result<(), Error> {
    let mc = MovieCollection::new();
    match trakt_command {
        TraktCommands::Calendar => trakt_cal_list(&mc)?,
        TraktCommands::WatchList => match trakt_action {
            TraktActions::Add => watchlist_add(&mc, show)?,
            TraktActions::Remove => watchlist_rm(&mc, show)?,
            TraktActions::List => watchlist_list(&mc)?,
            _ => {}
        },
        TraktCommands::Watched => match trakt_action {
            TraktActions::Add => watched_add(&mc, show, season, episode)?,
            TraktActions::Remove => watched_rm(&mc, show, season, episode)?,
            TraktActions::List => watched_list(&mc, show, season)?,
            _ => {}
        },
        _ => {}
    }
    Ok(())
}

pub fn watch_list_http_worker(pool: &PgPool, imdb_url: &str, season: i32) -> Result<String, Error> {
    let button_add = format!(
        "{}{}",
        r#"<button type="submit" id="ID" "#,
        r#"onclick="watched_add('SHOW', SEASON, EPISODE);">add to watched</button>"#
    );
    let button_rm = format!(
        "{}{}",
        r#"<button type="submit" id="ID" "#,
        r#"onclick="watched_rm('SHOW', SEASON, EPISODE);">remove from watched</button>"#
    );

    let mc = MovieCollection::with_pool(&pool)?;
    let mq = MovieQueueDB::with_pool(&pool);

    let show = ImdbRatings::get_show_by_link(imdb_url, &pool)?
        .ok_or_else(|| format_err!("Show Doesn't exist"))?;

    let watched_episodes_db: HashSet<i32> = get_watched_shows_db(&pool, &show.show, Some(season))?
        .into_iter()
        .map(|s| s.episode)
        .collect();

    let queue: HashMap<(String, i32, i32), _> = mq
        .print_movie_queue(&[&show.show])?
        .into_iter()
        .filter_map(|s| match &s.show {
            Some(show) => match s.season {
                Some(season) => match s.episode {
                    Some(episode) => Some(((show.to_string(), season, episode), s)),
                    None => None,
                },
                None => None,
            },
            None => None,
        })
        .collect();

    let entries: Vec<_> = mc.print_imdb_episodes(&show.show, Some(season))?;

    let collection_idx_map: Result<HashMap<i32, i32>, Error> = entries
        .iter()
        .filter_map(
            |r| match queue.get(&(show.show.to_string(), season, r.episode)) {
                Some(row) => match mc.get_collection_index(&row.path) {
                    Ok(i) => match i {
                        Some(index) => Some(Ok((r.episode, index))),
                        None => None,
                    },
                    Err(e) => Some(Err(e)),
                },
                None => None,
            },
        )
        .collect();

    let collection_idx_map = collection_idx_map?;

    let entries: Vec<_> = entries
        .iter()
        .map(|s| {
            let entry = if let Some(collection_idx) = collection_idx_map.get(&s.episode) {
                format!(
                    r#"<a href="javascript:updateMainArticle('{}');">{}</a>"#,
                    &format!("{}/{}", "/list/play", collection_idx),
                    s.eptitle
                )
            } else {
                s.eptitle.to_string()
            };

            format!(
                "<tr><td>{}</td><td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
                show.show,
                entry,
                format!(
                    r#"<a href="https://www.imdb.com/title/{}">s{} ep{}</a>"#,
                    s.epurl, season, s.episode,
                ),
                format!(
                    "rating: {:0.1} / {:0.1}",
                    s.rating,
                    show.rating.as_ref().unwrap_or(&-1.0)
                ),
                s.airdate,
                if watched_episodes_db.contains(&s.episode) {
                    button_rm
                        .replace("SHOW", &show.link)
                        .replace("SEASON", &season.to_string())
                        .replace("EPISODE", &s.episode.to_string())
                } else {
                    button_add
                        .replace("SHOW", &show.link)
                        .replace("SEASON", &season.to_string())
                        .replace("EPISODE", &s.episode.to_string())
                }
            )
        })
        .collect();

    let previous = format!(
        r#"<a href="javascript:updateMainArticle('/list/trakt/watched/list/{}')">Go Back</a><br>"#,
        imdb_url
    );
    let buttons = format!(
        r#"
        <button name="remcomout" id="remcomoutput"> &nbsp; </button>
        <button type="submit" id="ID"
            onclick="imdb_update('{show}', '{link}', {season},
            '/list/trakt/watched/list/{link}/{season}');"
            >update database</button><br>
    "#,
        show = show.show,
        link = show.link,
        season = season
    );

    let entries = format!(
        r#"{}{}<table border="0">{}</table>"#,
        previous,
        buttons,
        entries.join("\n")
    );
    Ok(entries)
}

pub fn watched_action_http_worker(
    pool: &PgPool,
    action: TraktActions,
    imdb_url: &str,
    season: i32,
    episode: i32,
) -> Result<String, Error> {
    let mc = MovieCollection::with_pool(&pool)?;

    let body = match action {
        TraktActions::Add => {
            let result = if season != -1 && episode != -1 {
                trakt_instance::add_episode_to_watched(&imdb_url, season, episode)?
            } else {
                trakt_instance::add_movie_to_watched(&imdb_url)?
            };
            if season != -1 && episode != -1 {
                WatchedEpisode {
                    imdb_url: imdb_url.to_string(),
                    season,
                    episode,
                    ..WatchedEpisode::default()
                }
                .insert_episode(&mc.pool)?;
            } else {
                WatchedMovie {
                    imdb_url: imdb_url.to_string(),
                    title: "".to_string(),
                }
                .insert_movie(&mc.pool)?;
            }

            format!("{}", result)
        }
        TraktActions::Remove => {
            let result = if season != -1 && episode != -1 {
                trakt_instance::remove_episode_to_watched(&imdb_url, season, episode)?
            } else {
                trakt_instance::remove_movie_to_watched(&imdb_url)?
            };

            if season != -1 && episode != -1 {
                if let Some(epi_) =
                    WatchedEpisode::get_watched_episode(&mc.pool, &imdb_url, season, episode)?
                {
                    epi_.delete_episode(&mc.pool)?;
                }
            } else if let Some(movie) = WatchedMovie::get_watched_movie(&mc.pool, &imdb_url)? {
                movie.delete_movie(&mc.pool)?;
            };

            format!("{}", result)
        }
        _ => "".to_string(),
    };
    Ok(body)
}

pub fn trakt_cal_http_worker(pool: &PgPool) -> Result<Vec<String>, Error> {
    let button_add = format!(
        "{}{}",
        r#"<td><button type="submit" id="ID" "#,
        r#"onclick="imdb_update('SHOW', 'LINK', SEASON, '/list/trakt/cal');"
            >update database</button></td>"#
    );
    let cal_list = trakt_instance::get_calendar()?;
    cal_list
        .into_iter()
        .map(|cal| {
            let show = match ImdbRatings::get_show_by_link(&cal.link, &pool)? {
                Some(s) => s.show,
                None => "".to_string(),
            };
            let exists = if show.is_empty() {None} else {
                let idx_opt = ImdbEpisodes {
                    show: show.to_string(),
                    season: cal.season,
                    episode: cal.episode,
                    ..ImdbEpisodes::default()
                }
                .get_index(&pool)?;

                match idx_opt {
                    Some(idx) => ImdbEpisodes::from_index(idx, &pool)?,
                    None => None,
                }
            };

            let entry = format!(
                "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td>{}</tr>",
                format!(
                    r#"<a href="javascript:updateMainArticle('/list/trakt/watched/list/{}/{}')">{}</a>"#,
                    cal.link, cal.season, cal.show,
                ),
                format!(
                    r#"<a href="https://www.imdb.com/title/{}">imdb</a>"#,
                    cal.link
                ),
                match cal.ep_link {
                    Some(link) => format!(
                        r#"<a href="https://www.imdb.com/title/{}">{} {}</a>"#,
                        link, cal.season, cal.episode,
                    ),
                    None => match exists.as_ref() {
                        Some(link) => format!(
                            r#"<a href="https://www.imdb.com/title/{}">{} {}</a>"#,
                            link, cal.season, cal.episode,
                        ),
                        None => format!("{} {}", cal.season, cal.episode,),
                    },
                },
                cal.airdate,
                if exists.is_some() {"".to_string()} else {
                    button_add
                        .replace("SHOW", &show)
                        .replace("LINK", &cal.link)
                        .replace("SEASON", &cal.season.to_string())
                },
            );
            Ok(entry)
        })
        .collect()
}
