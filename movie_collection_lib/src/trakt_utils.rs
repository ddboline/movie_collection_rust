use anyhow::Error;
use chrono::NaiveDate;
use futures::future::try_join_all;
use itertools::Itertools;
use log::debug;
use postgres_query::{query, query_dyn, FromSqlRow};
use serde::{Deserialize, Serialize};
use stack_string::StackString;
use std::{
    borrow::Borrow,
    collections::{HashMap, HashSet},
    fmt,
    hash::{Hash, Hasher},
    str::FromStr,
    sync::Arc,
};
use stdout_channel::StdoutChannel;

use crate::{
    config::Config, imdb_episodes::ImdbEpisodes, imdb_ratings::ImdbRatings,
    movie_collection::MovieCollection, pgpool::PgPool, trakt_connection::TraktConnection,
};

use crate::{tv_show_source::TvShowSource, utils::option_string_wrapper};

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
            option_string_wrapper(self.ep_link.as_ref()),
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

#[derive(Serialize, Deserialize, Debug, Default, FromSqlRow, Eq)]
pub struct WatchListShow {
    pub link: StackString,
    pub show: Option<StackString>,
    pub title: StackString,
    pub year: i32,
}

impl PartialEq for WatchListShow {
    fn eq(&self, other: &Self) -> bool {
        self.link == other.link
    }
}

impl Hash for WatchListShow {
    fn hash<H>(&self, state: &mut H)
    where
        H: Hasher,
    {
        self.link.hash(state);
    }
}

impl Borrow<str> for WatchListShow {
    fn borrow(&self) -> &str {
        self.link.as_str()
    }
}

impl fmt::Display for WatchListShow {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{} {} {}", self.link, self.title, self.year,)
    }
}

impl WatchListShow {
    pub async fn get_show_by_link(link: &str, pool: &PgPool) -> Result<Option<Self>, Error> {
        #[derive(FromSqlRow)]
        struct TitleYear {
            show: Option<StackString>,
            title: StackString,
            year: i32,
        }
        let query = query!(
            "
                SELECT a.show, a.title, a.year
                FROM trakt_watchlist a
                JOIN imdb_ratings b ON a.show = b.show
                WHERE (a.link = $link OR b.link = $link)
            ",
            link = link
        );
        let conn = pool.get().await?;
        Ok(query.fetch_opt(&conn).await?.map(|row| {
            let TitleYear { show, title, year } = row;
            Self {
                link: link.into(),
                show,
                title,
                year,
            }
        }))
    }

    pub async fn get_index(&self, pool: &PgPool) -> Result<Option<i32>, Error> {
        let query = query!(
            "
                SELECT a.id
                FROM trakt_watchlist a
                JOIN imdb_ratings b ON a.show = b.show
                WHERE (a.link = $link OR b.link = $link)
            ",
            link = self.link
        );
        let conn = pool.get().await?;
        let id = query.fetch_opt(&conn).await?;
        Ok(id.map(|(x,)| x))
    }

    pub async fn insert_show(&self, pool: &PgPool) -> Result<(), Error> {
        let query = query!(
            "
                INSERT INTO trakt_watchlist (link, show, title, year)
                VALUES ($link, $show, $title, $year)
            ",
            link = self.link,
            show = self.show,
            title = self.title,
            year = self.year
        );
        let conn = pool.get().await?;
        query.execute(&conn).await.map(|_| ()).map_err(Into::into)
    }

    pub async fn delete_show(&self, pool: &PgPool) -> Result<(), Error> {
        let query = query!(
            "
                DELETE FROM trakt_watchlist
                WHERE (link=$link OR show=(SELECT a.show FROM imdb_ratings a WHERE a.link = $link))
            ",
            link = self.link
        );
        let conn = pool.get().await?;
        query.execute(&conn).await.map(|_| ()).map_err(Into::into)
    }
}

pub async fn get_watchlist_shows_db(pool: &PgPool) -> Result<HashSet<WatchListShow>, Error> {
    let query = query!(
        r#"
        SELECT a.link, a.show, a.title, a.year
        FROM trakt_watchlist a
    "#
    );
    let conn = pool.get().await?;
    let shows = query.fetch(&conn).await?.into_iter().collect();
    Ok(shows)
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
        SELECT b.show, b.link, a.title, a.year, b.source
        FROM trakt_watchlist a
        JOIN imdb_ratings b ON a.show=b.show
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
                        show: Some(row.show),
                        title: row.title,
                        year: row.year,
                    },
                    source,
                ),
            ))
        })
        .collect()
}

#[derive(Serialize, Deserialize, Debug, Default, PartialEq, Eq, Hash, FromSqlRow)]
pub struct WatchedEpisode {
    pub title: StackString,
    pub show: Option<StackString>,
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
        let query = query!(
            r#"
                SELECT id
                FROM trakt_watched_episodes
                WHERE link=$link AND season=$season AND episode=$episode
            "#,
            link = self.imdb_url,
            season = self.season,
            episode = self.episode
        );
        let conn = pool.get().await?;
        let id = query.fetch_opt(&conn).await?;
        Ok(id.map(|(x,)| x))
    }

    pub async fn get_watched_episode(
        pool: &PgPool,
        link: &str,
        season: i32,
        episode: i32,
    ) -> Result<Option<Self>, Error> {
        let query = query!(
            r#"
                SELECT a.link as imdb_url,
                       c.title,
                       a.season,
                       a.episode
                FROM trakt_watched_episodes a
                JOIN trakt_watchlist b ON a.link = b.link
                JOIN imdb_ratings c ON b.show = c.show
                WHERE c.link = $link AND a.season = $season AND a.episode = $episode
            "#,
            link = link,
            season = season,
            episode = episode
        );
        let conn = pool.get().await?;
        query.fetch_opt(&conn).await.map_err(Into::into)
    }

    pub async fn insert_episode(&self, pool: &PgPool) -> Result<(), Error> {
        let query = query!(
            r#"
                INSERT INTO trakt_watched_episodes (link, season, episode)
                VALUES ($link, $season, $episode)
                ON CONFLICT DO NOTHING
            "#,
            link = self.imdb_url,
            season = self.season,
            episode = self.episode
        );
        let conn = pool.get().await?;
        query.execute(&conn).await.map(|_| ()).map_err(Into::into)
    }

    pub async fn delete_episode(&self, pool: &PgPool) -> Result<(), Error> {
        let query = query!(
            r#"
            DELETE FROM trakt_watched_episodes
            WHERE link=$link AND season=$season AND episode=$episode
        "#,
            link = self.imdb_url,
            season = self.season,
            episode = self.episode
        );
        let conn = pool.get().await?;
        query.execute(&conn).await.map(|_| ()).map_err(Into::into)
    }
}

pub async fn get_watched_shows_db(
    pool: &PgPool,
    show: &str,
    season: Option<i32>,
) -> Result<Vec<WatchedEpisode>, Error> {
    let mut where_vec = Vec::new();
    if !show.is_empty() {
        where_vec.push(format!("c.show='{}'", show));
    }
    if let Some(season) = season {
        where_vec.push(format!("a.season={}", season));
    }

    let where_str = if where_vec.is_empty() {
        "".to_string()
    } else {
        format!("WHERE {}", where_vec.join(" AND "))
    };

    let query = query_dyn!(&format!(
        r#"
            SELECT a.link as imdb_url,
                   c.show,
                   c.title,
                   a.season,
                   a.episode
            FROM trakt_watched_episodes a
            JOIN trakt_watchlist b ON a.link = b.link
            JOIN imdb_ratings c ON b.show = c.show
            {}
            ORDER BY 3,4,5
        "#,
        where_str
    ))?;
    let conn = pool.get().await?;
    query.fetch(&conn).await.map_err(Into::into)
}

#[derive(Serialize, Deserialize, Debug, Default, Eq, FromSqlRow)]
pub struct WatchedMovie {
    pub title: StackString,
    pub imdb_url: StackString,
}

impl PartialEq for WatchedMovie {
    fn eq(&self, other: &Self) -> bool {
        self.imdb_url == other.imdb_url
    }
}

impl Hash for WatchedMovie {
    fn hash<H>(&self, state: &mut H)
    where
        H: Hasher,
    {
        self.imdb_url.hash(state);
    }
}

impl Borrow<str> for WatchedMovie {
    fn borrow(&self) -> &str {
        self.imdb_url.as_str()
    }
}

impl fmt::Display for WatchedMovie {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{} {}", self.title, self.imdb_url)
    }
}

impl WatchedMovie {
    pub async fn get_index(&self, pool: &PgPool) -> Result<Option<i32>, Error> {
        let query = query!(
            r#"
                SELECT id
                FROM trakt_watched_movies
                WHERE link=$link
            "#,
            link = self.imdb_url
        );
        let conn = pool.get().await?;
        let id = query.fetch_opt(&conn).await?;
        Ok(id.map(|(x,)| x))
    }

    pub async fn get_watched_movie(pool: &PgPool, link: &str) -> Result<Option<Self>, Error> {
        let query = query!(
            r#"
                SELECT a.link as imdb_url,
                       b.title
                FROM trakt_watched_movies a
                JOIN imdb_ratings b ON a.link = b.link
                WHERE a.link = $link
            "#,
            link = link
        );
        let conn = pool.get().await?;
        query.fetch_opt(&conn).await.map_err(Into::into)
    }

    pub async fn insert_movie(&self, pool: &PgPool) -> Result<(), Error> {
        let query = query!(
            r#"
                INSERT INTO trakt_watched_movies (link)
                VALUES ($link)
            "#,
            link = self.imdb_url
        );
        let conn = pool.get().await?;
        query.execute(&conn).await.map(|_| ()).map_err(Into::into)
    }

    pub async fn delete_movie(&self, pool: &PgPool) -> Result<(), Error> {
        let query = query!(
            r#"
                DELETE FROM trakt_watched_movies
                WHERE link=$link
            "#,
            link = self.imdb_url
        );
        let conn = pool.get().await?;
        query.execute(&conn).await.map(|_| ()).map_err(Into::into)
    }
}

pub async fn get_watched_movies_db(pool: &PgPool) -> Result<Vec<WatchedMovie>, Error> {
    let query = query!(
        r#"
            SELECT a.link as imdb_url, b.title
            FROM trakt_watched_movies a
            JOIN imdb_ratings b ON a.link = b.link
            ORDER BY b.show
        "#
    );
    let conn = pool.get().await?;
    query.fetch(&conn).await.map_err(Into::into)
}

pub async fn sync_trakt_with_db(
    trakt: &TraktConnection,
    mc: &MovieCollection,
) -> Result<(), Error> {
    let watchlist_shows_db = Arc::new(get_watchlist_shows_db(&mc.pool).await?);
    trakt.init().await;
    let watchlist_shows = trakt.get_watchlist_shows().await?;
    if watchlist_shows.is_empty() {
        return Ok(());
    }

    let futures = watchlist_shows.into_iter().map(|(link, show)| {
        let watchlist_shows_db = watchlist_shows_db.clone();
        async move {
            if !watchlist_shows_db.contains(link.as_str()) {
                show.insert_show(&mc.pool).await?;
                mc.stdout.send(format!("insert watchlist {}", show));
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
    let watched_shows = trakt.get_watched_shows().await?;
    if watched_shows.is_empty() {
        return Ok(());
    }
    let futures = watched_shows.into_iter().map(|(key, episode)| {
        let watched_shows_db = watched_shows_db.clone();
        async move {
            if !watched_shows_db.contains_key(&key) {
                episode.insert_episode(&mc.pool).await?;
                mc.stdout
                    .send(format!("insert watched episode {}", episode));
            }
            Ok(())
        }
    });
    let results: Result<Vec<_>, Error> = try_join_all(futures).await;
    results?;

    let watched_movies_db: HashSet<_> =
        get_watched_movies_db(&mc.pool).await?.into_iter().collect();
    let watched_movies_db = Arc::new(watched_movies_db);
    let watched_movies = trakt.get_watched_movies().await?;
    let watched_movies = Arc::new(watched_movies);
    if watched_movies.is_empty() {
        return Ok(());
    }

    let futures = watched_movies.iter().map(|movie: &WatchedMovie| {
        let watched_movies_db = watched_movies_db.clone();
        async move {
            if !watched_movies_db.contains(movie.imdb_url.as_str()) {
                movie.insert_movie(&mc.pool).await?;
                mc.stdout.send(format!("insert watched movie {}", movie));
            }
            Ok(())
        }
    });
    let results: Result<Vec<_>, Error> = try_join_all(futures).await;
    results?;

    let futures = watched_movies_db.iter().map(|movie| {
        let watched_movies = watched_movies.clone();
        async move {
            if !watched_movies.contains(movie.imdb_url.as_str()) {
                movie.delete_movie(&mc.pool).await?;
                mc.stdout.send(format!("delete watched {}", movie));
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
    show: &str,
) -> Result<Option<StackString>, Error> {
    let imdb_shows = mc.print_imdb_shows(show, false).await?;
    let result = if imdb_shows.len() > 1 {
        for show in imdb_shows {
            debug!("{}", show);
        }
        None
    } else {
        Some(imdb_shows[0].link.clone())
    };
    Ok(result)
}

async fn trakt_cal_list(trakt: &TraktConnection, mc: &MovieCollection) -> Result<(), Error> {
    trakt.init().await;
    let cal_entries = trakt.get_calendar().await?;
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
            mc.stdout.send(format!("{} {}", show, cal));
        }
    }
    Ok(())
}

async fn watchlist_add(
    trakt: &TraktConnection,
    mc: &MovieCollection,
    show: &str,
    imdb_link: Option<&str>,
) -> Result<(), Error> {
    trakt.init().await;
    let imdb_url = if let Some(link) = imdb_link {
        link.into()
    } else if let Some(link) = get_imdb_url_from_show(mc, show).await? {
        link
    } else {
        return Ok(());
    };

    mc.stdout.send(format!(
        "result: {}",
        trakt.add_watchlist_show(&imdb_url).await?
    ));
    debug!("GOT HERE");
    if let Some(show_obj) = trakt
        .get_watchlist_shows()
        .await?
        .get_mut(imdb_url.as_str())
    {
        show_obj.show = Some(show.into());
        debug!("INSERT SHOW {}", show_obj);
        show_obj.insert_show(&mc.pool).await?;
    }
    Ok(())
}

async fn watchlist_rm(
    trakt: &TraktConnection,
    mc: &MovieCollection,
    show: &str,
) -> Result<(), Error> {
    if let Some(imdb_url) = get_imdb_url_from_show(mc, show).await? {
        let imdb_url_ = imdb_url.clone();
        trakt.init().await;
        mc.stdout.send(format!(
            "result: {}",
            trakt.remove_watchlist_show(&imdb_url_).await?
        ));
        if let Some(show) = WatchListShow::get_show_by_link(&imdb_url, &mc.pool).await? {
            show.delete_show(&mc.pool).await?;
        }
    }
    Ok(())
}

async fn watchlist_list(mc: &MovieCollection) -> Result<(), Error> {
    let show_map = get_watchlist_shows_db(&mc.pool).await?;
    mc.stdout
        .send(show_map.iter().map(ToString::to_string).join("\n"));
    Ok(())
}

async fn watched_add(
    trakt: &TraktConnection,
    mc: &MovieCollection,
    show: &str,
    season: i32,
    episode: &[i32],
) -> Result<(), Error> {
    trakt.init().await;
    if let Some(imdb_url) = get_imdb_url_from_show(mc, show).await? {
        if season != -1 && !episode.is_empty() {
            for epi in episode {
                let epi_ = *epi;
                let imdb_url_ = imdb_url.clone();
                trakt
                    .add_episode_to_watched(&imdb_url_, season, epi_)
                    .await?;
                WatchedEpisode {
                    imdb_url: imdb_url.clone(),
                    season,
                    episode: *epi,
                    ..WatchedEpisode::default()
                }
                .insert_episode(&mc.pool)
                .await?;
            }
        } else {
            let imdb_url_ = imdb_url.clone();
            trakt.add_movie_to_watched(&imdb_url_).await?;
            WatchedMovie {
                imdb_url,
                title: "".into(),
            }
            .insert_movie(&mc.pool)
            .await?;
        }
    }
    Ok(())
}

async fn watched_rm(
    trakt: &TraktConnection,
    mc: &MovieCollection,
    show: &str,
    season: i32,
    episode: &[i32],
) -> Result<(), Error> {
    trakt.init().await;
    if let Some(imdb_url) = get_imdb_url_from_show(mc, show).await? {
        if season != -1 && !episode.is_empty() {
            for epi in episode {
                let epi_ = *epi;
                let imdb_url_ = imdb_url.clone();
                trakt
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
            trakt.remove_movie_to_watched(&imdb_url_).await?;
            if let Some(movie) = WatchedMovie::get_watched_movie(&mc.pool, &imdb_url).await? {
                movie.delete_movie(&mc.pool).await?;
            }
        }
    }
    Ok(())
}

async fn watched_list(mc: &MovieCollection, show: &str, season: i32) -> Result<(), Error> {
    let watched_shows = get_watched_shows_db(&mc.pool, "", None).await?;
    let watched_movies = get_watched_movies_db(&mc.pool).await?;

    if let Some(imdb_url) = get_imdb_url_from_show(mc, show).await? {
        let lines = watched_shows
            .iter()
            .filter_map(|show| {
                if season != -1 && show.season != season {
                    return None;
                }
                if show.imdb_url.as_str() == imdb_url.as_str() {
                    Some(show.to_string())
                } else {
                    None
                }
            })
            .join("\n");
        mc.stdout.send(lines);
        let lines = watched_movies
            .iter()
            .filter_map(|show| {
                if show.imdb_url.as_str() == imdb_url.as_str() {
                    Some(show.to_string())
                } else {
                    None
                }
            })
            .join("\n");
        mc.stdout.send(lines);
    } else {
        mc.stdout
            .send(watched_shows.iter().map(ToString::to_string).join("\n"));
        mc.stdout
            .send(watched_movies.iter().map(ToString::to_string).join("\n"));
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub async fn trakt_app_parse(
    config: &Config,
    trakt: &TraktConnection,
    trakt_command: &TraktCommands,
    trakt_action: TraktActions,
    show: Option<&str>,
    imdb_link: Option<&str>,
    season: i32,
    episode: &[i32],
    stdout: &StdoutChannel<StackString>,
    pool: &PgPool,
) -> Result<(), Error> {
    let mc = MovieCollection::new(config, pool, stdout);
    match trakt_command {
        TraktCommands::Calendar => trakt_cal_list(trakt, &mc).await?,
        TraktCommands::WatchList => match trakt_action {
            TraktActions::Add => {
                if let Some(show) = show {
                    watchlist_add(trakt, &mc, show, imdb_link).await?;
                }
            }
            TraktActions::Remove => {
                if let Some(show) = show {
                    watchlist_rm(trakt, &mc, show).await?;
                }
            }
            TraktActions::List => watchlist_list(&mc).await?,
            TraktActions::None => {}
        },
        TraktCommands::Watched => match trakt_action {
            TraktActions::Add => {
                if let Some(show) = show {
                    watched_add(trakt, &mc, show, season, episode).await?;
                }
            }
            TraktActions::Remove => {
                if let Some(show) = show {
                    watched_rm(trakt, &mc, show, season, episode).await?;
                }
            }
            TraktActions::List => {
                if let Some(show) = show {
                    watched_list(&mc, show, season).await?;
                }
            }
            TraktActions::None => {}
        },
        TraktCommands::None => {}
    }
    mc.stdout.close().await
}
