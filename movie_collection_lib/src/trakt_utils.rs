use anyhow::{format_err, Error};
use chrono::NaiveDate;
use futures::future::{join_all, try_join_all};
use lazy_static::lazy_static;
use log::debug;
use postgres_query::FromSqlRow;
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    fmt,
    str::FromStr,
    sync::Arc,
};

use crate::{
    config::Config, imdb_episodes::ImdbEpisodes, imdb_ratings::ImdbRatings,
    movie_collection::MovieCollection, movie_queue::MovieQueueDB, pgpool::PgPool,
    stack_string::StackString, trakt_connection::TraktConnection,
};

use crate::{tv_show_source::TvShowSource, utils::option_string_wrapper};

lazy_static! {
    static ref CONFIG: Config = Config::with_config().unwrap();
    pub static ref TRAKT_CONN: TraktConnection = TraktConnection::new(CONFIG.clone());
}

#[derive(Clone, Copy)]
pub enum TraktActions {
    None,
    List,
    Add,
    Remove,
}

impl From<&str> for TraktActions {
    fn from(s: &str) -> Self {
        match s {
            "list" => Self::List,
            "add" => Self::Add,
            "rm" | "del" => Self::Remove,
            _ => Self::None,
        }
    }
}

impl FromStr for TraktActions {
    type Err = ();
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self::from(s))
    }
}

pub enum TraktCommands {
    None,
    Calendar,
    WatchList,
    Watched,
}

impl From<&str> for TraktCommands {
    fn from(s: &str) -> Self {
        match s {
            "cal" | "calendar" => Self::Calendar,
            "watchlist" => Self::WatchList,
            "watched" => Self::Watched,
            _ => Self::None,
        }
    }
}

impl FromStr for TraktCommands {
    type Err = ();
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self::from(s))
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub struct TraktCalEntry {
    pub ep_link: Option<StackString>,
    pub episode: i32,
    pub link: StackString,
    pub season: i32,
    pub show: StackString,
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
    pub status: StackString,
}

impl fmt::Display for TraktResult {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "status {}", self.status)
    }
}

#[derive(Serialize, Deserialize, Debug, Default, FromSqlRow)]
pub struct WatchListShow {
    pub link: StackString,
    pub title: StackString,
    pub year: i32,
}

impl fmt::Display for WatchListShow {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{} {} {}", self.link, self.title, self.year,)
    }
}

impl WatchListShow {
    pub async fn get_show_by_link(link: &str, pool: &PgPool) -> Result<Option<Self>, Error> {
        let query = postgres_query::query!(
            "SELECT title, year FROM trakt_watchlist WHERE link = $link",
            link = link
        );
        if let Some(row) = pool
            .get()
            .await?
            .query(query.sql(), query.parameters())
            .await?
            .get(0)
        {
            let title: String = row.try_get("title")?;
            let year: i32 = row.try_get("year")?;
            Ok(Some(Self {
                link: link.into(),
                title: title.into(),
                year,
            }))
        } else {
            Ok(None)
        }
    }

    pub async fn get_index(&self, pool: &PgPool) -> Result<Option<i32>, Error> {
        let query = postgres_query::query!(
            "SELECT id FROM trakt_watchlist WHERE link = $link",
            link = self.link
        );
        if let Some(row) = pool
            .get()
            .await?
            .query(query.sql(), query.parameters())
            .await?
            .get(0)
        {
            let id: i32 = row.try_get("id")?;
            Ok(Some(id))
        } else {
            Ok(None)
        }
    }

    pub async fn insert_show(&self, pool: &PgPool) -> Result<(), Error> {
        let query = postgres_query::query!(
            "INSERT INTO trakt_watchlist (link, title, year) VALUES ($link, $title, $year)",
            link = self.link,
            title = self.title,
            year = self.year
        );
        pool.get()
            .await?
            .execute(query.sql(), query.parameters())
            .await
            .map(|_| ())
            .map_err(Into::into)
    }

    pub async fn delete_show(&self, pool: &PgPool) -> Result<(), Error> {
        let query = postgres_query::query!(
            "DELETE FROM trakt_watchlist WHERE link=$link",
            link = self.link
        );
        pool.get()
            .await?
            .execute(query.sql(), query.parameters())
            .await
            .map(|_| ())
            .map_err(Into::into)
    }
}

pub async fn get_watchlist_shows_db(
    pool: &PgPool,
) -> Result<HashMap<StackString, WatchListShow>, Error> {
    let query = r#"
        SELECT a.link, a.title, a.year
        FROM trakt_watchlist a
    "#;
    pool.get()
        .await?
        .query(query, &[])
        .await?
        .iter()
        .map(|row| {
            let val = WatchListShow::from_row(row)?;
            Ok((val.link.clone(), val))
        })
        .collect()
}

pub type WatchListMap = HashMap<StackString, (StackString, WatchListShow, Option<TvShowSource>)>;

pub async fn get_watchlist_shows_db_map(pool: &PgPool) -> Result<WatchListMap, Error> {
    #[derive(FromSqlRow)]
    struct WatchlistShowDbMap {
        show: StackString,
        link: StackString,
        title: StackString,
        year: i32,
        source: Option<StackString>,
    }

    let query = r#"
        SELECT b.show, a.link, a.title, a.year, b.source
        FROM trakt_watchlist a
        JOIN imdb_ratings b ON a.link=b.link
    "#;

    pool.get()
        .await?
        .query(query, &[])
        .await?
        .iter()
        .map(|row| {
            let row = WatchlistShowDbMap::from_row(row)?;

            let source: Option<TvShowSource> = match row.source {
                Some(s) => s.parse().ok(),
                None => None,
            };

            Ok((
                row.link.clone(),
                (
                    row.show.clone(),
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
    pub title: StackString,
    pub imdb_url: StackString,
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
    pub async fn get_index(&self, pool: &PgPool) -> Result<Option<i32>, Error> {
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
        if let Some(row) = pool
            .get()
            .await?
            .query(query.sql(), query.parameters())
            .await?
            .get(0)
        {
            let id: i32 = row.try_get("id")?;
            Ok(Some(id))
        } else {
            Ok(None)
        }
    }

    pub async fn get_watched_episode(
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
        if let Some(row) = pool
            .get()
            .await?
            .query(query.sql(), query.parameters())
            .await?
            .get(0)
        {
            let imdb_url: StackString = row.try_get("link")?;
            let title: StackString = row.try_get("title")?;
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

    pub async fn insert_episode(&self, pool: &PgPool) -> Result<(), Error> {
        let query = postgres_query::query!(
            r#"
                INSERT INTO trakt_watched_episodes (link, season, episode)
                VALUES ($link, $season, $episode)
            "#,
            link = self.imdb_url,
            season = self.season,
            episode = self.episode
        );
        pool.get()
            .await?
            .execute(query.sql(), query.parameters())
            .await
            .map(|_| ())
            .map_err(Into::into)
    }

    pub async fn delete_episode(&self, pool: &PgPool) -> Result<(), Error> {
        let query = postgres_query::query!(
            r#"
            DELETE FROM trakt_watched_episodes
            WHERE link=$link AND season=$season AND episode=$episode
        "#,
            link = self.imdb_url,
            season = self.season,
            episode = self.episode
        );
        pool.get()
            .await?
            .execute(query.sql(), query.parameters())
            .await
            .map(|_| ())
            .map_err(Into::into)
    }
}

pub async fn get_watched_shows_db(
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

    pool.get()
        .await?
        .query(query.sql(), &[])
        .await?
        .iter()
        .map(|row| {
            let imdb_url: StackString = row.try_get("link")?;
            let title: StackString = row.try_get("title")?;
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
    pub title: StackString,
    pub imdb_url: StackString,
}

impl fmt::Display for WatchedMovie {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{} {}", self.title, self.imdb_url)
    }
}

impl WatchedMovie {
    pub async fn get_index(&self, pool: &PgPool) -> Result<Option<i32>, Error> {
        let query = postgres_query::query!(
            r#"
                SELECT id
                FROM trakt_watched_movies
                WHERE link=$link
            "#,
            link = self.imdb_url
        );
        if let Some(row) = pool
            .get()
            .await?
            .query(query.sql(), query.parameters())
            .await?
            .get(0)
        {
            let id: i32 = row.try_get("id")?;
            Ok(Some(id))
        } else {
            Ok(None)
        }
    }

    pub async fn get_watched_movie(pool: &PgPool, link: &str) -> Result<Option<Self>, Error> {
        let query = postgres_query::query!(
            r#"
                SELECT a.link, b.title
                FROM trakt_watched_movies a
                JOIN imdb_ratings b ON a.link = b.link
                WHERE a.link = $link
            "#,
            link = link
        );
        if let Some(row) = pool
            .get()
            .await?
            .query(query.sql(), query.parameters())
            .await?
            .get(0)
        {
            let imdb_url: StackString = row.try_get("link")?;
            let title: StackString = row.try_get("title")?;
            Ok(Some(Self { title, imdb_url }))
        } else {
            Ok(None)
        }
    }

    pub async fn insert_movie(&self, pool: &PgPool) -> Result<(), Error> {
        let query = postgres_query::query!(
            r#"
                INSERT INTO trakt_watched_movies (link)
                VALUES ($link)
            "#,
            link = self.imdb_url
        );
        pool.get()
            .await?
            .execute(query.sql(), query.parameters())
            .await
            .map(|_| ())
            .map_err(Into::into)
    }

    pub async fn delete_movie(&self, pool: &PgPool) -> Result<(), Error> {
        let query = postgres_query::query!(
            r#"
                DELETE FROM trakt_watched_movies
                WHERE link=$link
            "#,
            link = self.imdb_url
        );
        pool.get()
            .await?
            .execute(query.sql(), query.parameters())
            .await
            .map(|_| ())
            .map_err(Into::into)
    }
}

pub async fn get_watched_movies_db(pool: &PgPool) -> Result<Vec<WatchedMovie>, Error> {
    let query = postgres_query::query!(
        r#"
            SELECT a.link, b.title
            FROM trakt_watched_movies a
            JOIN imdb_ratings b ON a.link = b.link
            ORDER BY b.show
        "#
    );
    pool.get()
        .await?
        .query(query.sql(), &[])
        .await?
        .iter()
        .map(|row| {
            let imdb_url: StackString = row.try_get("link")?;
            let title: StackString = row.try_get("title")?;
            Ok(WatchedMovie { title, imdb_url })
        })
        .collect()
}

pub async fn sync_trakt_with_db(mc: &MovieCollection) -> Result<(), Error> {
    let watchlist_shows_db = Arc::new(get_watchlist_shows_db(&mc.pool).await?);
    TRAKT_CONN.init().await;
    let watchlist_shows = TRAKT_CONN.get_watchlist_shows().await?;
    if watchlist_shows.is_empty() {
        return Ok(());
    }

    let futures = watchlist_shows.into_iter().map(|(link, show)| {
        let watchlist_shows_db = watchlist_shows_db.clone();
        async move {
            if !watchlist_shows_db.contains_key(&link) {
                show.insert_show(&mc.pool).await?;
                mc.stdout
                    .send(format!("insert watchlist {}", show).into())?;
            }
            Ok(())
        }
    });
    let results: Result<Vec<_>, Error> = try_join_all(futures).await;
    results?;

    let watched_shows_db: HashMap<(StackString, i32, i32), _> =
        get_watched_shows_db(&mc.pool, "", None)
            .await?
            .into_iter()
            .map(|s| ((s.imdb_url.clone(), s.season, s.episode), s))
            .collect();
    let watched_shows_db = Arc::new(watched_shows_db);
    let watched_shows = TRAKT_CONN.get_watched_shows().await?;
    if watched_shows.is_empty() {
        return Ok(());
    }
    let futures = watched_shows.into_iter().map(|(key, episode)| {
        let watched_shows_db = watched_shows_db.clone();
        async move {
            if !watched_shows_db.contains_key(&key) {
                episode.insert_episode(&mc.pool).await?;
                mc.stdout
                    .send(format!("insert watched {}", episode).into())?;
            }
            Ok(())
        }
    });
    let results: Result<Vec<_>, Error> = try_join_all(futures).await;
    results?;

    let watched_movies_db: HashMap<StackString, _> = get_watched_movies_db(&mc.pool)
        .await?
        .into_iter()
        .map(|s| (s.imdb_url.clone(), s))
        .collect();
    let watched_movies_db = Arc::new(watched_movies_db);
    let watched_movies = TRAKT_CONN.get_watched_movies().await?;
    let watched_movies = Arc::new(watched_movies);
    if watched_movies.is_empty() {
        return Ok(());
    }

    let futures = watched_movies.iter().map(|(key, movie)| {
        let watched_movies_db = watched_movies_db.clone();
        async move {
            if !watched_movies_db.contains_key(key) {
                movie.insert_movie(&mc.pool).await?;
                mc.stdout.send(format!("insert watched {}", movie).into())?;
            }
            Ok(())
        }
    });
    let results: Result<Vec<_>, Error> = try_join_all(futures).await;
    results?;

    let futures = watched_movies_db.iter().map(|(key, movie)| {
        let watched_movies = watched_movies.clone();
        async move {
            if !watched_movies.contains_key(key) {
                movie.delete_movie(&mc.pool).await?;
                mc.stdout.send(format!("delete watched {}", movie).into())?;
            }
            Ok(())
        }
    });
    let results: Result<Vec<_>, Error> = try_join_all(futures).await;
    results?;
    Ok(())
}

async fn get_imdb_url_from_show(
    mc: &MovieCollection,
    show: Option<&str>,
) -> Result<Option<String>, Error> {
    let result = if let Some(show) = show {
        let imdb_shows = mc.print_imdb_shows(show, false).await?;
        if imdb_shows.len() > 1 {
            for show in imdb_shows {
                debug!("{}", show);
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

async fn trakt_cal_list(mc: &MovieCollection) -> Result<(), Error> {
    TRAKT_CONN.init().await;
    let cal_entries = TRAKT_CONN.get_calendar().await?;
    for cal in cal_entries {
        let show = match ImdbRatings::get_show_by_link(&cal.link, &mc.pool).await? {
            Some(s) => s.show,
            None => "".into(),
        };
        let exists = if show.is_empty() {
            false
        } else {
            ImdbEpisodes {
                show: show.clone(),
                season: cal.season,
                episode: cal.episode,
                ..ImdbEpisodes::default()
            }
            .get_index(&mc.pool)
            .await?
            .is_some()
        };
        if !exists {
            mc.stdout.send(format!("{} {}", show, cal).into())?;
        }
    }
    Ok(())
}

async fn watchlist_add(mc: &MovieCollection, show: Option<&str>) -> Result<(), Error> {
    TRAKT_CONN.init().await;
    if let Some(imdb_url) = get_imdb_url_from_show(&mc, show).await? {
        let imdb_url_ = imdb_url.clone();
        mc.stdout.send(
            format!(
                "result: {}",
                TRAKT_CONN.add_watchlist_show(&imdb_url_).await?
            )
            .into(),
        )?;
        debug!("GOT HERE");
        if let Some(show) = TRAKT_CONN
            .get_watchlist_shows()
            .await?
            .get(imdb_url.as_str())
        {
            debug!("INSERT SHOW {}", show);
            show.insert_show(&mc.pool).await?;
        }
    }
    Ok(())
}

async fn watchlist_rm(mc: &MovieCollection, show: Option<&str>) -> Result<(), Error> {
    if let Some(imdb_url) = get_imdb_url_from_show(&mc, show).await? {
        let imdb_url_ = imdb_url.clone();
        TRAKT_CONN.init().await;
        mc.stdout.send(
            format!(
                "result: {}",
                TRAKT_CONN.remove_watchlist_show(&imdb_url_).await?
            )
            .into(),
        )?;
        if let Some(show) = WatchListShow::get_show_by_link(&imdb_url, &mc.pool).await? {
            show.delete_show(&mc.pool).await?;
        }
    }
    Ok(())
}

async fn watchlist_list(mc: &MovieCollection) -> Result<(), Error> {
    let show_map = get_watchlist_shows_db(&mc.pool).await?;
    for (_, show) in show_map {
        mc.stdout.send(format!("{}", show).into())?;
    }
    Ok(())
}

async fn watched_add(
    mc: &MovieCollection,
    show: Option<&str>,
    season: i32,
    episode: &[i32],
) -> Result<(), Error> {
    TRAKT_CONN.init().await;
    if let Some(imdb_url) = get_imdb_url_from_show(&mc, show).await? {
        if season != -1 && !episode.is_empty() {
            for epi in episode {
                let epi_ = *epi;
                let imdb_url_ = imdb_url.clone();
                TRAKT_CONN
                    .add_episode_to_watched(&imdb_url_, season, epi_)
                    .await?;
                WatchedEpisode {
                    imdb_url: imdb_url.clone().into(),
                    season,
                    episode: *epi,
                    ..WatchedEpisode::default()
                }
                .insert_episode(&mc.pool)
                .await?;
            }
        } else {
            let imdb_url_ = imdb_url.clone();
            TRAKT_CONN.add_movie_to_watched(&imdb_url_).await?;
            WatchedMovie {
                imdb_url: imdb_url.into(),
                title: "".into(),
            }
            .insert_movie(&mc.pool)
            .await?;
        }
    }
    Ok(())
}

async fn watched_rm(
    mc: &MovieCollection,
    show: Option<&str>,
    season: i32,
    episode: &[i32],
) -> Result<(), Error> {
    TRAKT_CONN.init().await;
    if let Some(imdb_url) = get_imdb_url_from_show(&mc, show).await? {
        if season != -1 && !episode.is_empty() {
            for epi in episode {
                let epi_ = *epi;
                let imdb_url_ = imdb_url.clone();
                TRAKT_CONN
                    .remove_episode_to_watched(&imdb_url_, season, epi_)
                    .await?;
                if let Some(epi_) =
                    WatchedEpisode::get_watched_episode(&mc.pool, &imdb_url, season, *epi).await?
                {
                    epi_.delete_episode(&mc.pool).await?;
                }
            }
        } else {
            let imdb_url_ = imdb_url.clone();
            TRAKT_CONN.remove_movie_to_watched(&imdb_url_).await?;
            if let Some(movie) = WatchedMovie::get_watched_movie(&mc.pool, &imdb_url).await? {
                movie.delete_movie(&mc.pool).await?;
            }
        }
    }
    Ok(())
}

async fn watched_list(mc: &MovieCollection, show: Option<&str>, season: i32) -> Result<(), Error> {
    let watched_shows = get_watched_shows_db(&mc.pool, "", None).await?;
    let watched_movies = get_watched_movies_db(&mc.pool).await?;

    if let Some(imdb_url) = get_imdb_url_from_show(&mc, show).await? {
        for show in &watched_shows {
            if season != -1 && show.season != season {
                continue;
            }
            if show.imdb_url.as_str() == imdb_url.as_str() {
                mc.stdout.send(show.to_string().into())?;
            }
        }
        for show in &watched_movies {
            if show.imdb_url.as_str() == imdb_url.as_str() {
                mc.stdout.send(show.to_string().into())?;
            }
        }
    } else {
        for show in &watched_shows {
            mc.stdout.send(show.to_string().into())?;
        }
        for show in &watched_movies {
            mc.stdout.send(show.to_string().into())?;
        }
    }
    Ok(())
}

pub async fn trakt_app_parse(
    trakt_command: &TraktCommands,
    trakt_action: TraktActions,
    show: Option<&str>,
    season: i32,
    episode: &[i32],
) -> Result<(), Error> {
    let mc = MovieCollection::new();
    let task = mc.stdout.spawn_stdout_task();
    match trakt_command {
        TraktCommands::Calendar => trakt_cal_list(&mc).await?,
        TraktCommands::WatchList => match trakt_action {
            TraktActions::Add => watchlist_add(&mc, show).await?,
            TraktActions::Remove => watchlist_rm(&mc, show).await?,
            TraktActions::List => watchlist_list(&mc).await?,
            _ => {}
        },
        TraktCommands::Watched => match trakt_action {
            TraktActions::Add => watched_add(&mc, show, season, episode).await?,
            TraktActions::Remove => watched_rm(&mc, show, season, episode).await?,
            TraktActions::List => watched_list(&mc, show, season).await?,
            _ => {}
        },
        _ => {}
    }
    mc.stdout.close().await?;
    task.await?
}

pub async fn watch_list_http_worker(
    pool: &PgPool,
    imdb_url: &str,
    season: i32,
) -> Result<String, Error> {
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

    let show = ImdbRatings::get_show_by_link(imdb_url, &pool)
        .await?
        .ok_or_else(|| format_err!("Show Doesn't exist"))?;

    let watched_episodes_db: HashSet<i32> = get_watched_shows_db(&pool, &show.show, Some(season))
        .await?
        .into_iter()
        .map(|s| s.episode)
        .collect();

    let queue: HashMap<(StackString, i32, i32), _> = mq
        .print_movie_queue(&[show.show.as_str()])
        .await?
        .into_iter()
        .filter_map(|s| match &s.show {
            Some(show) => match s.season {
                Some(season) => match s.episode {
                    Some(episode) => Some(((show.clone(), season, episode), s)),
                    None => None,
                },
                None => None,
            },
            None => None,
        })
        .collect();

    let entries: Vec<_> = mc.print_imdb_episodes(&show.show, Some(season)).await?;

    let mut collection_idx_map = HashMap::new();
    for r in &entries {
        if let Some(row) = queue.get(&(show.show.clone(), season, r.episode)) {
            if let Some(index) = mc.get_collection_index(&row.path).await? {
                collection_idx_map.insert(r.episode, index);
            }
        }
    }

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
                    r#"<a href="https://www.imdb.com/title/{}" target="_blank">s{} ep{}</a>"#,
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

pub async fn watched_action_http_worker(
    pool: &PgPool,
    action: TraktActions,
    imdb_url: &str,
    season: i32,
    episode: i32,
) -> Result<String, Error> {
    let mc = MovieCollection::with_pool(&pool)?;
    let imdb_url = Arc::new(imdb_url.to_owned());
    TRAKT_CONN.init().await;
    let body = match action {
        TraktActions::Add => {
            let result = if season != -1 && episode != -1 {
                let imdb_url_ = Arc::clone(&imdb_url);
                TRAKT_CONN
                    .add_episode_to_watched(&imdb_url_, season, episode)
                    .await?
            } else {
                let imdb_url_ = Arc::clone(&imdb_url);
                TRAKT_CONN.add_movie_to_watched(&imdb_url_).await?
            };
            if season != -1 && episode != -1 {
                WatchedEpisode {
                    imdb_url: imdb_url.to_string().into(),
                    season,
                    episode,
                    ..WatchedEpisode::default()
                }
                .insert_episode(&mc.pool)
                .await?;
            } else {
                WatchedMovie {
                    imdb_url: imdb_url.to_string().into(),
                    title: "".into(),
                }
                .insert_movie(&mc.pool)
                .await?;
            }

            format!("{}", result)
        }
        TraktActions::Remove => {
            let imdb_url_ = Arc::clone(&imdb_url);
            let result = if season != -1 && episode != -1 {
                TRAKT_CONN
                    .remove_episode_to_watched(&imdb_url_, season, episode)
                    .await?
            } else {
                TRAKT_CONN.remove_movie_to_watched(&imdb_url_).await?
            };

            if season != -1 && episode != -1 {
                if let Some(epi_) =
                    WatchedEpisode::get_watched_episode(&mc.pool, &imdb_url, season, episode)
                        .await?
                {
                    epi_.delete_episode(&mc.pool).await?;
                }
            } else if let Some(movie) = WatchedMovie::get_watched_movie(&mc.pool, &imdb_url).await?
            {
                movie.delete_movie(&mc.pool).await?;
            };

            format!("{}", result)
        }
        _ => "".to_string(),
    };
    Ok(body)
}

pub async fn trakt_cal_http_worker(pool: &PgPool) -> Result<Vec<String>, Error> {
    let button_add = format!(
        "{}{}",
        r#"<td><button type="submit" id="ID" "#,
        r#"onclick="imdb_update('SHOW', 'LINK', SEASON, '/list/trakt/cal');"
            >update database</button></td>"#
    );
    TRAKT_CONN.init().await;
    let cal_list = TRAKT_CONN.get_calendar().await?;
    let results: Vec<_> = cal_list
        .into_iter()
        .map(|cal| async {
            let show = match ImdbRatings::get_show_by_link(&cal.link, &pool).await? {
                Some(s) => s.show,
                None => "".into(),
            };
            let exists = if show.is_empty() {None} else {
                let idx_opt = ImdbEpisodes {
                    show: show.clone(),
                    season: cal.season,
                    episode: cal.episode,
                    ..ImdbEpisodes::default()
                }
                .get_index(&pool).await?;

                match idx_opt {
                    Some(idx) => ImdbEpisodes::from_index(idx, &pool).await?,
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
                    r#"<a href="https://www.imdb.com/title/{}" target="_blank">imdb</a>"#,
                    cal.link
                ),
                if let Some(link) = cal.ep_link {
                    format!(
                        r#"<a href="https://www.imdb.com/title/{}" target="_blank">{} {}</a>"#,
                        link, cal.season, cal.episode,
                    )
                } else if let Some(link) = exists.as_ref() {
                    format!(
                        r#"<a href="https://www.imdb.com/title/{}" target="_blank">{} {}</a>"#,
                        link, cal.season, cal.episode,
                    )
                } else {
                    format!("{} {}", cal.season, cal.episode,)
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
        .collect();
    join_all(results).await.into_iter().collect()
}
