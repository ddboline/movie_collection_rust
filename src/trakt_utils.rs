use chrono::NaiveDate;
use failure::Error;
use std::collections::{HashMap, HashSet};
use std::fmt;

use crate::config::Config;
use crate::movie_collection::PgPool;
use crate::utils::option_string_wrapper;

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
    pub fn get_index(&self, pool: &PgPool) -> Result<Option<i32>, Error> {
        let query = "SELECT id FROM trakt_watchlist WHERE link = $1";
        for row in pool.get()?.query(query, &[&self.link])?.iter() {
            let id: i32 = row.get(0);
            return Ok(Some(id));
        }
        Ok(None)
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

pub fn get_watchlist_shows() -> Result<HashMap<String, WatchListShow>, Error> {
    let config = Config::with_config();
    let url = format!("https://{}/trakt/watchlist", &config.domain);
    let watchlist_shows: Vec<WatchListShow> = reqwest::get(&url)?.json()?;
    let watchlist_shows = watchlist_shows
        .into_iter()
        .map(|s| (s.link.clone(), s))
        .collect();
    Ok(watchlist_shows)
}

pub fn get_watchlist_shows_db(pool: &PgPool) -> Result<HashMap<String, WatchListShow>, Error> {
    let query = "SELECT link, title, year FROM trakt_watchlist";
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

#[derive(Serialize, Deserialize, Debug, Default, PartialEq, Eq, Hash)]
pub struct WatchedShows {
    pub title: String,
    pub imdb_url: String,
    pub episode: i32,
    pub season: i32,
}

impl fmt::Display for WatchedShows {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "{} {} {} {}",
            self.title, self.imdb_url, self.episode, self.season
        )
    }
}

impl WatchedShows {
    pub fn get_index(&self, pool: &PgPool) -> Result<Option<i32>, Error> {
        let query = r#"
            SELECT id
            FROM trakt_watched_episodes
            WHERE link=$1 AND season=$2 AND episode=$3
        "#;
        for row in pool
            .get()?
            .query(query, &[&self.imdb_url, &self.season, &self.episode])?
            .iter()
        {
            let id: i32 = row.get(0);
            return Ok(Some(id));
        }
        Ok(None)
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

pub fn get_watched_shows() -> Result<HashMap<(String, i32, i32), WatchedShows>, Error> {
    let config = Config::with_config();
    let url = format!("https://{}/trakt/watched_shows", &config.domain);
    let watched_shows: Vec<WatchedShows> = reqwest::get(&url)?.json()?;
    let watched_shows: HashMap<(String, i32, i32), WatchedShows> = watched_shows
        .into_iter()
        .map(|s| ((s.imdb_url.clone(), s.season, s.episode), s))
        .collect();
    Ok(watched_shows)
}

pub fn get_watched_shows_db(
    pool: &PgPool,
) -> Result<HashMap<(String, i32, i32), WatchedShows>, Error> {
    let query = r#"
        SELECT a.link, b.title, a.season, a.episode
        FROM trakt_watched_episodes a
        JOIN imdb_ratings b ON a.link = b.link
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
            (
                (imdb_url.clone(), season, episode),
                WatchedShows {
                    title,
                    imdb_url,
                    season,
                    episode,
                },
            )
        })
        .collect();
    Ok(watched_shows)
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

impl TraktCalEntry {}

pub type TraktCalEntryList = Vec<TraktCalEntry>;

pub fn get_calendar() -> Result<TraktCalEntryList, Error> {
    let config = Config::with_config();
    let url = format!("https://{}/trakt/cal", &config.domain);
    let calendar = reqwest::get(&url)?.json()?;
    Ok(calendar)
}
