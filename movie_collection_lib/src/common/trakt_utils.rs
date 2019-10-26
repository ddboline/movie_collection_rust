use chrono::NaiveDate;
use failure::{err_msg, Error};
use log::debug;
use rayon::iter::{IntoParallelRefIterator, ParallelIterator};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::io;
use std::io::Write;

use crate::common::imdb_episodes::ImdbEpisodes;
use crate::common::imdb_ratings::ImdbRatings;
use crate::common::movie_collection::MovieCollection;
use crate::common::movie_queue::MovieQueueDB;
use crate::common::pgpool::PgPool;
use crate::common::row_index_trait::RowIndexTrait;
use crate::common::trakt_instance::TraktInstance;
use crate::common::tv_show_source::TvShowSource;
use crate::common::utils::option_string_wrapper;

#[derive(Clone, Copy)]
pub enum TraktActions {
    None,
    List,
    Add,
    Remove,
}

impl TraktActions {
    pub fn from_command(command: &str) -> TraktActions {
        match command {
            "list" => TraktActions::List,
            "add" => TraktActions::Add,
            "rm" => TraktActions::Remove,
            "del" => TraktActions::Remove,
            _ => TraktActions::None,
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
    pub fn from_command(command: &str) -> TraktCommands {
        match command {
            "cal" | "calendar" => TraktCommands::Calendar,
            "watchlist" => TraktCommands::WatchList,
            "watched" => TraktCommands::Watched,
            _ => TraktCommands::None,
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

#[derive(Serialize, Deserialize, Debug, Default)]
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
    pub fn get_show_by_link(link: &str, pool: &PgPool) -> Result<Option<WatchListShow>, Error> {
        let query = "SELECT title, year FROM trakt_watchlist WHERE link = $1";
        if let Some(row) = pool.get()?.query(query, &[&link])?.iter().nth(0) {
            let title: String = row.get_idx(0)?;
            let year: i32 = row.get_idx(1)?;
            Ok(Some(WatchListShow {
                link: link.to_string(),
                title,
                year,
            }))
        } else {
            Ok(None)
        }
    }

    pub fn get_index(&self, pool: &PgPool) -> Result<Option<i32>, Error> {
        let query = "SELECT id FROM trakt_watchlist WHERE link = $1";
        if let Some(row) = pool.get()?.query(query, &[&self.link])?.iter().nth(0) {
            let id: i32 = row.get_idx(0)?;
            Ok(Some(id))
        } else {
            Ok(None)
        }
    }

    pub fn insert_show(&self, pool: &PgPool) -> Result<(), Error> {
        let query = "INSERT INTO trakt_watchlist (link, title, year) VALUES ($1, $2, $3)";
        pool.get()?
            .execute(query, &[&self.link, &self.title, &self.year])
            .map(|_| ())
            .map_err(err_msg)
    }

    pub fn delete_show(&self, pool: &PgPool) -> Result<(), Error> {
        let query = "DELETE FROM trakt_watchlist WHERE link=$1";
        pool.get()?
            .execute(query, &[&self.link])
            .map(|_| ())
            .map_err(err_msg)
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
            let link: String = row.get_idx(0)?;
            let title: String = row.get_idx(1)?;
            let year: i32 = row.get_idx(2)?;
            Ok((link.to_string(), WatchListShow { link, title, year }))
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
    pool.get()?
        .query(query, &[])?
        .iter()
        .map(|row| {
            let show: String = row.get_idx(0)?;
            let link: String = row.get_idx(1)?;
            let title: String = row.get_idx(2)?;
            let year: i32 = row.get_idx(3)?;
            let source: Option<String> = row.get_idx(4)?;

            let source: Option<TvShowSource> = match source {
                Some(s) => s.parse().ok(),
                None => None,
            };

            Ok((
                link.to_string(),
                (show, WatchListShow { link, title, year }, source),
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
        let query = r#"
            SELECT id
            FROM trakt_watched_episodes
            WHERE link=$1 AND season=$2 AND episode=$3
        "#;
        if let Some(row) = pool
            .get()?
            .query(query, &[&self.imdb_url, &self.season, &self.episode])?
            .iter()
            .nth(0)
        {
            let id: i32 = row.get_idx(0)?;
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
    ) -> Result<Option<WatchedEpisode>, Error> {
        let query = r#"
            SELECT a.link, b.title, a.season, a.episode
            FROM trakt_watched_episodes a
            JOIN imdb_ratings b ON a.link = b.link
            WHERE a.link = $1 AND a.season = $2 AND a.episode = $3
        "#;
        if let Some(row) = pool
            .get()?
            .query(query, &[&link, &season, &episode])?
            .iter()
            .nth(0)
        {
            let imdb_url: String = row.get_idx(0)?;
            let title: String = row.get_idx(1)?;
            let season: i32 = row.get_idx(2)?;
            let episode: i32 = row.get_idx(3)?;
            Ok(Some(WatchedEpisode {
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
        let query = r#"
            INSERT INTO trakt_watched_episodes (link, season, episode)
            VALUES ($1, $2, $3)
        "#;
        pool.get()?
            .execute(query, &[&self.imdb_url, &self.season, &self.episode])
            .map(|_| ())
            .map_err(err_msg)
    }

    pub fn delete_episode(&self, pool: &PgPool) -> Result<(), Error> {
        let query = r#"
            DELETE FROM trakt_watched_episodes
            WHERE link=$1 AND season=$2 AND episode=$3
        "#;
        pool.get()?
            .execute(query, &[&self.imdb_url, &self.season, &self.episode])
            .map(|_| ())
            .map_err(err_msg)
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

    let where_str = if !where_vec.is_empty() {
        format!("WHERE {}", where_vec.join(" AND "))
    } else {
        "".to_string()
    };

    let query = format!(
        r#"
        SELECT a.link, b.title, a.season, a.episode
        FROM trakt_watched_episodes a
        JOIN imdb_ratings b ON a.link = b.link
        {}
        ORDER BY 2,3,4
    "#,
        where_str
    );

    pool.get()?
        .query(&query, &[])?
        .iter()
        .map(|row| {
            let imdb_url: String = row.get_idx(0)?;
            let title: String = row.get_idx(1)?;
            let season: i32 = row.get_idx(2)?;
            let episode: i32 = row.get_idx(3)?;
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
        let query = r#"
            SELECT id
            FROM trakt_watched_movies
            WHERE link=$1
        "#;
        if let Some(row) = pool.get()?.query(query, &[&self.imdb_url])?.iter().nth(0) {
            let id: i32 = row.get_idx(0)?;
            Ok(Some(id))
        } else {
            Ok(None)
        }
    }

    pub fn get_watched_movie(pool: &PgPool, link: &str) -> Result<Option<WatchedMovie>, Error> {
        let query = r#"
            SELECT a.link, b.title
            FROM trakt_watched_movies a
            JOIN imdb_ratings b ON a.link = b.link
            WHERE a.link = $1
        "#;
        if let Some(row) = pool.get()?.query(query, &[&link])?.iter().nth(0) {
            let imdb_url: String = row.get_idx(0)?;
            let title: String = row.get_idx(1)?;
            Ok(Some(WatchedMovie { title, imdb_url }))
        } else {
            Ok(None)
        }
    }

    pub fn insert_movie(&self, pool: &PgPool) -> Result<(), Error> {
        let query = r#"
            INSERT INTO trakt_watched_movies (link)
            VALUES ($1)
        "#;
        pool.get()?
            .execute(query, &[&self.imdb_url])
            .map(|_| ())
            .map_err(err_msg)
    }

    pub fn delete_movie(&self, pool: &PgPool) -> Result<(), Error> {
        let query = r#"
            DELETE FROM trakt_watched_movies
            WHERE link=$1
        "#;
        pool.get()?
            .execute(query, &[&self.imdb_url])
            .map(|_| ())
            .map_err(err_msg)
    }
}

pub fn get_watched_movies_db(pool: &PgPool) -> Result<Vec<WatchedMovie>, Error> {
    let query = r#"
        SELECT a.link, b.title
        FROM trakt_watched_movies a
        JOIN imdb_ratings b ON a.link = b.link
        ORDER BY b.show
    "#;
    pool.get()?
        .query(query, &[])?
        .iter()
        .map(|row| {
            let imdb_url: String = row.get_idx(0)?;
            let title: String = row.get_idx(1)?;
            Ok(WatchedMovie { title, imdb_url })
        })
        .collect()
}

pub fn sync_trakt_with_db() -> Result<(), Error> {
    let mc = MovieCollection::new();
    let ti = TraktInstance::new();

    let watchlist_shows_db = get_watchlist_shows_db(&mc.pool)?;
    let watchlist_shows = ti.get_watchlist_shows()?;
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
    let watched_shows = ti.get_watched_shows()?;
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
    let watched_movies = ti.get_watched_movies()?;
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

fn trakt_cal_list(ti: &TraktInstance, mc: &MovieCollection) -> Result<(), Error> {
    let cal_entries = ti.get_calendar()?;
    for cal in cal_entries {
        let show = match ImdbRatings::get_show_by_link(&cal.link, &mc.pool)? {
            Some(s) => s.show,
            None => "".to_string(),
        };
        let exists = if !show.is_empty() {
            ImdbEpisodes {
                show: show.to_string(),
                season: cal.season,
                episode: cal.episode,
                ..Default::default()
            }
            .get_index(&mc.pool)?
            .is_some()
        } else {
            false
        };
        if !exists {
            writeln!(io::stdout().lock(), "{} {}", show, cal)?;
        }
    }
    Ok(())
}

fn watchlist_add(
    ti: &TraktInstance,
    mc: &MovieCollection,
    show: Option<&str>,
) -> Result<(), Error> {
    if let Some(imdb_url) = get_imdb_url_from_show(&mc, show)? {
        writeln!(
            io::stdout().lock(),
            "result: {}",
            ti.add_watchlist_show(&imdb_url)?
        )?;
        debug!("GOT HERE");
        if let Some(show) = ti.get_watchlist_shows()?.get(&imdb_url) {
            debug!("INSERT SHOW {}", show);
            show.insert_show(&mc.pool)?;
        }
    }
    Ok(())
}

fn watchlist_rm(ti: &TraktInstance, mc: &MovieCollection, show: Option<&str>) -> Result<(), Error> {
    if let Some(imdb_url) = get_imdb_url_from_show(&mc, show)? {
        writeln!(
            io::stdout().lock(),
            "result: {}",
            ti.remove_watchlist_show(&imdb_url)?
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
    ti: &TraktInstance,
    mc: &MovieCollection,
    show: Option<&str>,
    season: i32,
    episode: &[i32],
) -> Result<(), Error> {
    if let Some(imdb_url) = get_imdb_url_from_show(&mc, show)? {
        if season != -1 && !episode.is_empty() {
            for epi in episode {
                ti.add_episode_to_watched(&imdb_url, season, *epi)?;
                WatchedEpisode {
                    imdb_url: imdb_url.to_string(),
                    season,
                    episode: *epi,
                    ..Default::default()
                }
                .insert_episode(&mc.pool)?;
            }
        } else {
            ti.add_movie_to_watched(&imdb_url)?;
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
    ti: &TraktInstance,
    mc: &MovieCollection,
    show: Option<&str>,
    season: i32,
    episode: &[i32],
) -> Result<(), Error> {
    if let Some(imdb_url) = get_imdb_url_from_show(&mc, show)? {
        if season != -1 && !episode.is_empty() {
            for epi in episode {
                ti.remove_episode_to_watched(&imdb_url, season, *epi)?;
                if let Some(epi_) =
                    WatchedEpisode::get_watched_episode(&mc.pool, &imdb_url, season, *epi)?
                {
                    epi_.delete_episode(&mc.pool)?;
                }
            }
        } else {
            ti.remove_movie_to_watched(&imdb_url)?;
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
    let ti = TraktInstance::new();
    match trakt_command {
        TraktCommands::Calendar => trakt_cal_list(&ti, &mc)?,
        TraktCommands::WatchList => match trakt_action {
            TraktActions::Add => watchlist_add(&ti, &mc, show)?,
            TraktActions::Remove => watchlist_rm(&ti, &mc, show)?,
            TraktActions::List => watchlist_list(&mc)?,
            _ => {}
        },
        TraktCommands::Watched => match trakt_action {
            TraktActions::Add => watched_add(&ti, &mc, show, season, episode)?,
            TraktActions::Remove => watched_rm(&ti, &mc, show, season, episode)?,
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
        .ok_or_else(|| err_msg("Show Doesn't exist"))?;

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
    let ti = TraktInstance::new();
    let mc = MovieCollection::with_pool(&pool)?;

    let body = match action {
        TraktActions::Add => {
            let result = if season != -1 && episode != -1 {
                ti.add_episode_to_watched(&imdb_url, season, episode)?
            } else {
                ti.add_movie_to_watched(&imdb_url)?
            };
            if season != -1 && episode != -1 {
                WatchedEpisode {
                    imdb_url: imdb_url.to_string(),
                    season,
                    episode,
                    ..Default::default()
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
                ti.remove_episode_to_watched(&imdb_url, season, episode)?
            } else {
                ti.remove_movie_to_watched(&imdb_url)?
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
    let cal_list = TraktInstance::new().get_calendar()?;
    cal_list
        .into_iter()
        .map(|cal| {
            let show = match ImdbRatings::get_show_by_link(&cal.link, &pool)? {
                Some(s) => s.show,
                None => "".to_string(),
            };
            let exists = if !show.is_empty() {
                let idx_opt = ImdbEpisodes {
                    show: show.to_string(),
                    season: cal.season,
                    episode: cal.episode,
                    ..Default::default()
                }
                .get_index(&pool)?;

                match idx_opt {
                    Some(idx) => ImdbEpisodes::from_index(idx, &pool)?,
                    None => None,
                }
            } else {
                None
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
                if !exists.is_some() {
                    button_add
                        .replace("SHOW", &show)
                        .replace("LINK", &cal.link)
                        .replace("SEASON", &cal.season.to_string())
                } else {
                    "".to_string()
                },
            );
            Ok(entry)
        })
        .collect()
}
