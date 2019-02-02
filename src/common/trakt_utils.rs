extern crate chrono;
extern crate failure;
extern crate r2d2;
extern crate r2d2_postgres;
extern crate rayon;
extern crate reqwest;
extern crate select;

use chrono::NaiveDate;
use failure::Error;
use rayon::prelude::*;
use reqwest::{Client, Url};
use std::collections::HashMap;
use std::fmt;
use std::io;
use std::io::Write;

use crate::common::config::Config;
use crate::common::imdb_episodes::ImdbEpisodes;
use crate::common::imdb_ratings::ImdbRatings;
use crate::common::movie_collection::{MovieCollection, MovieCollectionDB};
use crate::common::pgpool::PgPool;
use crate::common::utils::{map_result_vec, option_string_wrapper, ExponentialRetry};

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

pub struct TraktConnection {
    client: Client,
    config: Config,
}

impl Default for TraktConnection {
    fn default() -> TraktConnection {
        TraktConnection::new()
    }
}

impl ExponentialRetry for TraktConnection {
    fn get_client(&self) -> &Client {
        &self.client
    }
}

impl TraktConnection {
    pub fn new() -> TraktConnection {
        TraktConnection {
            client: Client::new(),
            config: Config::with_config(),
        }
    }

    pub fn get_watchlist_shows(&self) -> Result<HashMap<String, WatchListShow>, Error> {
        let url = format!("https://{}/trakt/watchlist", &self.config.domain);
        let url = Url::parse(&url)?;
        let watchlist_shows: Vec<WatchListShow> = self.get(&url)?.json()?;
        let watchlist_shows = watchlist_shows
            .into_iter()
            .map(|s| (s.link.clone(), s))
            .collect();
        Ok(watchlist_shows)
    }

    pub fn add_watchlist_show(&self, imdb_id: &str) -> Result<TraktResult, Error> {
        let url = format!(
            "https://{}/trakt/add_to_watchlist/{}",
            &self.config.domain, imdb_id
        );
        let url = Url::parse(&url)?;
        let result: TraktResult = self.get(&url)?.json()?;
        Ok(result)
    }

    pub fn remove_watchlist_show(&self, imdb_id: &str) -> Result<TraktResult, Error> {
        let url = format!(
            "https://{}/trakt/delete_show/{}",
            &self.config.domain, imdb_id
        );
        let url = Url::parse(&url)?;
        let result: TraktResult = self.get(&url)?.json()?;
        Ok(result)
    }

    pub fn get_watched_shows(&self) -> Result<HashMap<(String, i32, i32), WatchedEpisode>, Error> {
        let url = format!("https://{}/trakt/watched_shows", &self.config.domain);
        let url = Url::parse(&url)?;
        let watched_shows: Vec<WatchedEpisode> = self.get(&url)?.json()?;
        let watched_shows: HashMap<(String, i32, i32), WatchedEpisode> = watched_shows
            .into_iter()
            .map(|s| ((s.imdb_url.clone(), s.season, s.episode), s))
            .collect();
        Ok(watched_shows)
    }

    pub fn get_watched_movies(&self) -> Result<HashMap<String, WatchedMovie>, Error> {
        let url = format!("https://{}/trakt/watched_movies", &self.config.domain);
        let url = Url::parse(&url)?;
        let watched_movies: Vec<WatchedMovie> = self.get(&url)?.json()?;
        let watched_movies: HashMap<String, WatchedMovie> = watched_movies
            .into_iter()
            .map(|s| (s.imdb_url.clone(), s))
            .collect();
        Ok(watched_movies)
    }

    pub fn get_calendar(&self) -> Result<TraktCalEntryList, Error> {
        let url = format!("https://{}/trakt/cal", &self.config.domain);
        let url = Url::parse(&url)?;
        let calendar = self.get(&url)?.json()?;
        Ok(calendar)
    }

    pub fn add_episode_to_watched(
        &self,
        imdb_id: &str,
        season: i32,
        episode: i32,
    ) -> Result<TraktResult, Error> {
        let url = format!(
            "https://{}/trakt/add_episode_to_watched/{}/{}/{}",
            &self.config.domain, imdb_id, season, episode
        );
        let url = Url::parse(&url)?;
        let result: TraktResult = self.get(&url)?.json()?;
        Ok(result)
    }

    pub fn add_movie_to_watched(&self, imdb_id: &str) -> Result<TraktResult, Error> {
        let url = format!(
            "https://{}/trakt/add_to_watched/{}",
            &self.config.domain, imdb_id
        );
        let url = Url::parse(&url)?;
        let result: TraktResult = self.get(&url)?.json()?;
        Ok(result)
    }

    pub fn remove_episode_to_watched(
        &self,
        imdb_id: &str,
        season: i32,
        episode: i32,
    ) -> Result<TraktResult, Error> {
        let url = format!(
            "https://{}/trakt/delete_watched/{}/{}/{}",
            &self.config.domain, imdb_id, season, episode
        );
        let url = Url::parse(&url)?;
        let result: TraktResult = self.get(&url)?.json()?;
        Ok(result)
    }

    pub fn remove_movie_to_watched(&self, imdb_id: &str) -> Result<TraktResult, Error> {
        let url = format!(
            "https://{}/trakt/delete_watched_movie/{}",
            &self.config.domain, imdb_id
        );
        let url = Url::parse(&url)?;
        let result: TraktResult = self.get(&url)?.json()?;
        Ok(result)
    }
}

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
            let title: String = row.get(0);
            let year: i32 = row.get(1);
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
            let id: i32 = row.get(0);
            Ok(Some(id))
        } else {
            Ok(None)
        }
    }

    pub fn insert_show(&self, pool: &PgPool) -> Result<(), Error> {
        let query = "INSERT INTO trakt_watchlist (link, title, year) VALUES ($1, $2, $3)";
        pool.get()?
            .execute(query, &[&self.link, &self.title, &self.year])?;
        Ok(())
    }

    pub fn delete_show(&self, pool: &PgPool) -> Result<(), Error> {
        let query = "DELETE FROM trakt_watchlist WHERE link=$1";
        pool.get()?.execute(query, &[&self.link])?;
        Ok(())
    }
}

pub fn get_watchlist_shows_db(pool: &PgPool) -> Result<HashMap<String, WatchListShow>, Error> {
    let query = r#"
        SELECT a.link, a.title, a.year
        FROM trakt_watchlist a
    "#;
    let watchlist = pool
        .get()?
        .query(query, &[])?
        .iter()
        .map(|row| {
            let link: String = row.get(0);
            let title: String = row.get(1);
            let year: i32 = row.get(2);
            (link.clone(), WatchListShow { link, title, year })
        })
        .collect();
    Ok(watchlist)
}

pub type WatchListMap = HashMap<String, (String, WatchListShow, Option<String>)>;

pub fn get_watchlist_shows_db_map(pool: &PgPool) -> Result<WatchListMap, Error> {
    let query = r#"
        SELECT b.show, a.link, a.title, a.year, b.source
        FROM trakt_watchlist a
        JOIN imdb_ratings b ON a.link=b.link
    "#;
    let watchlist = pool
        .get()?
        .query(query, &[])?
        .iter()
        .map(|row| {
            let show: String = row.get(0);
            let link: String = row.get(1);
            let title: String = row.get(2);
            let year: i32 = row.get(3);
            let source: Option<String> = row.get(4);
            (
                link.clone(),
                (show, WatchListShow { link, title, year }, source),
            )
        })
        .collect();
    Ok(watchlist)
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
            let id: i32 = row.get(0);
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
        let watched_episode = if let Some(row) = pool
            .get()?
            .query(query, &[&link, &season, &episode])?
            .iter()
            .nth(0)
        {
            let imdb_url: String = row.get(0);
            let title: String = row.get(1);
            let season: i32 = row.get(2);
            let episode: i32 = row.get(3);
            Some(WatchedEpisode {
                title,
                imdb_url,
                season,
                episode,
            })
        } else {
            None
        };
        Ok(watched_episode)
    }

    pub fn insert_episode(&self, pool: &PgPool) -> Result<(), Error> {
        let query = r#"
            INSERT INTO trakt_watched_episodes (link, season, episode)
            VALUES ($1, $2, $3)
        "#;
        pool.get()?
            .execute(query, &[&self.imdb_url, &self.season, &self.episode])?;
        Ok(())
    }

    pub fn delete_episode(&self, pool: &PgPool) -> Result<(), Error> {
        let query = r#"
            DELETE FROM trakt_watched_episodes
            WHERE link=$1 AND season=$2 AND episode=$3
        "#;
        pool.get()?
            .execute(query, &[&self.imdb_url, &self.season, &self.episode])?;
        Ok(())
    }
}

pub fn get_watched_shows_db(pool: &PgPool) -> Result<Vec<WatchedEpisode>, Error> {
    let query = r#"
        SELECT a.link, b.title, a.season, a.episode
        FROM trakt_watched_episodes a
        JOIN imdb_ratings b ON a.link = b.link
        ORDER BY 2,3,4
    "#;
    let watched_shows = pool
        .get()?
        .query(query, &[])?
        .iter()
        .map(|row| {
            let imdb_url: String = row.get(0);
            let title: String = row.get(1);
            let season: i32 = row.get(2);
            let episode: i32 = row.get(3);
            WatchedEpisode {
                title,
                imdb_url,
                season,
                episode,
            }
        })
        .collect();
    Ok(watched_shows)
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
            let id: i32 = row.get(0);
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
        let watched_movie = if let Some(row) = pool.get()?.query(query, &[&link])?.iter().nth(0) {
            let imdb_url: String = row.get(0);
            let title: String = row.get(1);
            Some(WatchedMovie { title, imdb_url })
        } else {
            None
        };
        Ok(watched_movie)
    }

    pub fn insert_movie(&self, pool: &PgPool) -> Result<(), Error> {
        let query = r#"
            INSERT INTO trakt_watched_movies (link)
            VALUES ($1)
        "#;
        pool.get()?.execute(query, &[&self.imdb_url])?;
        Ok(())
    }

    pub fn delete_movie(&self, pool: &PgPool) -> Result<(), Error> {
        let query = r#"
            DELETE FROM trakt_watched_movies
            WHERE link=$1
        "#;
        pool.get()?.execute(query, &[&self.imdb_url])?;
        Ok(())
    }
}

pub fn get_watched_movies_db(pool: &PgPool) -> Result<Vec<WatchedMovie>, Error> {
    let query = r#"
        SELECT a.link, b.title
        FROM trakt_watched_movies a
        JOIN imdb_ratings b ON a.link = b.link
        ORDER BY b.show
    "#;
    let watched_shows = pool
        .get()?
        .query(query, &[])?
        .iter()
        .map(|row| {
            let imdb_url: String = row.get(0);
            let title: String = row.get(1);
            WatchedMovie { title, imdb_url }
        })
        .collect();
    Ok(watched_shows)
}

pub fn sync_trakt_with_db() -> Result<(), Error> {
    let mc = MovieCollectionDB::new();
    let ti = TraktConnection::new();

    let watchlist_shows_db = get_watchlist_shows_db(&mc.pool)?;
    let watchlist_shows = ti.get_watchlist_shows()?;
    if watchlist_shows.is_empty() {
        return Ok(());
    }

    let stdout = io::stdout();

    let results: Vec<Result<_, Error>> = watchlist_shows
        .par_iter()
        .map(|(link, show)| {
            if !watchlist_shows_db.contains_key(link) {
                show.insert_show(&mc.pool)?;
                writeln!(stdout.lock(), "insert watchlist {}", show)?;
            }
            Ok(())
        })
        .collect();
    map_result_vec(results)?;

    /*
    let results: Vec<Result<_, Error>> = watchlist_shows_db
        .par_iter()
        .map(|(link, show)| {
            if !watchlist_shows.contains_key(link) {
                show.delete_show(&mc.pool)?;
                writeln!(stdout.lock(), "delete watchlist {}", show)?;
            }
            Ok(())
        })
        .collect();
    map_result_vec(results)?;
    */

    let watched_shows_db: HashMap<(String, i32, i32), _> = get_watched_shows_db(&mc.pool)?
        .into_iter()
        .map(|s| ((s.imdb_url.clone(), s.season, s.episode), s))
        .collect();
    let watched_shows = ti.get_watched_shows()?;
    if watched_shows.is_empty() {
        return Ok(());
    }
    let results: Vec<Result<_, Error>> = watched_shows
        .par_iter()
        .map(|(key, episode)| {
            if !watched_shows_db.contains_key(&key) {
                episode.insert_episode(&mc.pool)?;
                writeln!(stdout.lock(), "insert watched {}", episode)?;
            }
            Ok(())
        })
        .collect();
    map_result_vec(results)?;

    /*
    let results: Vec<Result<_, Error>> = watched_shows_db
        .par_iter()
        .map(|(key, episode)| {
            if !watched_shows.contains_key(&key) {
                episode.delete_episode(&mc.pool)?;
                writeln!(stdout.lock(), "delete watched {}", episode)?;
            }
            Ok(())
        })
        .collect();
    map_result_vec(results)?;
    */

    let watched_movies_db: HashMap<String, _> = get_watched_movies_db(&mc.pool)?
        .into_iter()
        .map(|s| (s.imdb_url.clone(), s))
        .collect();
    let watched_movies = ti.get_watched_movies()?;
    if watched_movies.is_empty() {
        return Ok(());
    }
    let results: Vec<Result<_, Error>> = watched_movies
        .par_iter()
        .map(|(key, movie)| {
            if !watched_movies_db.contains_key(key) {
                movie.insert_movie(&mc.pool)?;
                writeln!(stdout.lock(), "insert watched {}", movie)?;
            }
            Ok(())
        })
        .collect();
    map_result_vec(results)?;

    let results: Vec<Result<_, Error>> = watched_movies_db
        .par_iter()
        .map(|(key, movie)| {
            if !watched_movies.contains_key(key) {
                movie.delete_movie(&mc.pool)?;
                writeln!(stdout.lock(), "delete watched {}", movie)?;
            }
            Ok(())
        })
        .collect();
    map_result_vec(results)?;
    Ok(())
}

fn get_imdb_url_from_show(
    mc: &MovieCollectionDB,
    show: Option<&String>,
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
            imdb_shows[0].link.clone()
        }
    } else {
        None
    };
    Ok(result)
}

fn trakt_cal_list(ti: &TraktConnection, mc: &MovieCollectionDB) -> Result<(), Error> {
    for cal in ti.get_calendar()? {
        let show = match ImdbRatings::get_show_by_link(&cal.link, &mc.pool)? {
            Some(s) => s.show,
            None => "".to_string(),
        };
        let exists = if !show.is_empty() {
            ImdbEpisodes {
                show: show.clone(),
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
    ti: &TraktConnection,
    mc: &MovieCollectionDB,
    show: Option<&String>,
) -> Result<(), Error> {
    if let Some(imdb_url) = get_imdb_url_from_show(&mc, show)? {
        writeln!(
            io::stdout().lock(),
            "result: {}",
            ti.add_watchlist_show(&imdb_url)?
        )?;
        if let Some(show) = ti.get_watchlist_shows()?.get(&imdb_url) {
            show.insert_show(&mc.pool)?;
        }
    }
    Ok(())
}

fn watchlist_rm(
    ti: &TraktConnection,
    mc: &MovieCollectionDB,
    show: Option<&String>,
) -> Result<(), Error> {
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

fn watchlist_list(mc: &MovieCollectionDB) -> Result<(), Error> {
    for (_, show) in get_watchlist_shows_db(&mc.pool)? {
        writeln!(io::stdout().lock(), "{}", show)?;
    }
    Ok(())
}

fn watched_add(
    ti: &TraktConnection,
    mc: &MovieCollectionDB,
    show: Option<&String>,
    season: i32,
    episode: &[i32],
) -> Result<(), Error> {
    if let Some(imdb_url) = get_imdb_url_from_show(&mc, show)? {
        if season != -1 && !episode.is_empty() {
            for epi in episode {
                ti.add_episode_to_watched(&imdb_url, season, *epi)?;
                WatchedEpisode {
                    imdb_url: imdb_url.clone(),
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
    ti: &TraktConnection,
    mc: &MovieCollectionDB,
    show: Option<&String>,
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

fn watched_list(mc: &MovieCollectionDB, show: Option<&String>, season: i32) -> Result<(), Error> {
    let watched_shows = get_watched_shows_db(&mc.pool)?;
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
    trakt_action: &TraktActions,
    show: Option<&String>,
    season: i32,
    episode: &[i32],
) -> Result<(), Error> {
    let mc = MovieCollectionDB::new();
    let ti = TraktConnection::new();
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
