use anyhow::Error;
use futures::{stream::FuturesUnordered, Stream, TryStreamExt};
use itertools::Itertools;
use log::debug;
use postgres_query::{query, query_dyn, Error as PqError, FromSqlRow, Query};
use postgres_query::Parameter;
use serde::{Deserialize, Serialize};
use stack_string::{format_sstr, StackString};
use std::{
    borrow::Borrow,
    collections::{HashMap, HashSet},
    fmt,
    fmt::Write,
    hash::{Hash, Hasher},
    str::FromStr,
    sync::Arc,
};
use stdout_channel::StdoutChannel;
use time::Date;
use time::OffsetDateTime;
use uuid::Uuid;
use rust_decimal::Decimal;

use crate::{
    config::Config, date_time_wrapper::DateTimeWrapper, imdb_episodes::ImdbEpisodes,
    imdb_ratings::ImdbRatings, movie_collection::MovieCollection, pgpool::PgPool,
    trakt_connection::TraktConnection,
};

use crate::{tv_show_source::TvShowSource, utils::option_string_wrapper};

#[derive(Clone, Copy, Deserialize, Serialize)]
pub enum TraktActions {
    #[serde(rename = "none")]
    None,
    #[serde(rename = "list")]
    List,
    #[serde(rename = "add")]
    Add,
    #[serde(rename = "remove")]
    Remove,
}

impl TraktActions {
    #[must_use]
    pub fn to_str(self) -> &'static str {
        match self {
            Self::List => "list",
            Self::Add => "add",
            Self::Remove => "rm",
            Self::None => "",
        }
    }
}

impl fmt::Display for TraktActions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.to_str())
    }
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

#[derive(Copy, Clone)]
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

#[derive(Serialize, Deserialize, Debug, PartialEq, Eq, Clone)]
pub struct TraktCalEntry {
    pub ep_link: Option<StackString>,
    pub episode: i32,
    pub link: StackString,
    pub season: i32,
    pub show: StackString,
    pub airdate: Date,
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

#[derive(Serialize, Deserialize, Debug, Default, FromSqlRow, Eq, Clone)]
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
    /// # Errors
    /// Return error if db query fails
    pub async fn get_show_by_link(link: &str, pool: &PgPool) -> Result<Option<Self>, Error> {
        #[derive(FromSqlRow)]
        struct TitleYear {
            show: Option<StackString>,
            title: StackString,
            year: i32,
        }
        let query = query!(
            "
                SELECT tw.show, tw.title, tw.year
                FROM trakt_watchlist tw
                JOIN imdb_ratings ir ON tw.show = ir.show
                WHERE (tw.link = $link OR ir.link = $link)
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

    /// # Errors
    /// Return error if db query fails
    pub async fn get_index(&self, pool: &PgPool) -> Result<Option<Uuid>, Error> {
        let query = query!(
            "
                SELECT tw.id
                FROM trakt_watchlist tw
                JOIN imdb_ratings ir ON tw.show = ir.show
                WHERE (tw.link = $link OR ir.link = $link)
            ",
            link = self.link
        );
        let conn = pool.get().await?;
        let id = query.fetch_opt(&conn).await?;
        Ok(id.map(|(x,)| x))
    }

    /// # Errors
    /// Return error if db query fails
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

    /// # Errors
    /// Return error if db query fails
    pub async fn delete_show(&self, pool: &PgPool) -> Result<(), Error> {
        let query = query!(
            "
                DELETE FROM trakt_watchlist
                WHERE (link=$link OR show=(SELECT ir.show FROM imdb_ratings ir WHERE ir.link = $link))
            ",
            link = self.link
        );
        let conn = pool.get().await?;
        query.execute(&conn).await.map(|_| ()).map_err(Into::into)
    }
}

/// # Errors
/// Return error if db query fails
pub async fn get_watchlist_shows_db(pool: &PgPool) -> Result<HashSet<WatchListShow>, Error> {
    let query = query!(
        r#"
        SELECT tw.link, tw.show, tw.title, tw.year
        FROM trakt_watchlist tw
    "#
    );
    let conn = pool.get().await?;
    let shows = query.fetch_streaming(&conn).await?.try_collect().await?;
    Ok(shows)
}

pub type WatchListMap = HashMap<StackString, (StackString, WatchListShow, Option<TvShowSource>)>;

/// # Errors
/// Return error if db query fails
pub async fn get_watchlist_shows_db_map(
    pool: &PgPool,
    search_query: Option<&str>,
    source: Option<TvShowSource>,
    offset: Option<usize>,
    limit: Option<usize>,
) -> Result<WatchListMap, Error> {
    #[derive(FromSqlRow)]
    struct WatchlistShowDbMap {
        show: StackString,
        link: StackString,
        title: StackString,
        year: i32,
        source: Option<StackString>,
    }

    async fn get_watchlist_shows_db_map_impl(
        pool: &PgPool,
        search_query: Option<&str>,
        source: Option<&str>,
        offset: Option<usize>,
        limit: Option<usize>,
    ) -> Result<impl Stream<Item = Result<WatchlistShowDbMap, PqError>>, Error> {
        let mut constraints = Vec::new();
        if let Some(search_query) = search_query {
            constraints.push(format_sstr!("ir.show ilike '%{search_query}%'"));
        }
        if let Some(source) = source {
            if source != "all" {
                constraints.push(format_sstr!("ir.source = '{source}'"));
            }
        }
        let query = format_sstr!(
            "
                SELECT ir.show, ir.link, tw.title, tw.year, ir.source
                FROM trakt_watchlist tw
                JOIN imdb_ratings ir ON tw.show=ir.show
                {where_str}
                ORDER BY 1
                {offset}
                {limit}
            ",
            where_str = if constraints.is_empty() {
                StackString::new()
            } else {
                format_sstr!("WHERE {}", constraints.join(" AND "))
            },
            offset = if let Some(offset) = offset {
                format_sstr!("OFFSET {offset}")
            } else {
                StackString::new()
            },
            limit = if let Some(limit) = limit {
                format_sstr!("LIMIT {limit}")
            } else {
                StackString::new()
            }
        );
        let query: Query = query_dyn!(&query)?;
        let conn = pool.get().await?;
        query.fetch_streaming(&conn).await.map_err(Into::into)
    }
    let source = source.map(TvShowSource::to_str);
    get_watchlist_shows_db_map_impl(pool, search_query, source, offset, limit)
        .await?
        .map_ok(|row| {
            let source: Option<TvShowSource> = match row.source {
                Some(s) => s.parse().ok(),
                None => None,
            };

            (
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
            )
        })
        .try_collect()
        .await
        .map_err(Into::into)
}

#[derive(Serialize, Deserialize, Debug, Default, PartialEq, Eq, Hash, FromSqlRow)]
pub struct WatchedEpisode {
    pub title: StackString,
    pub show: Option<StackString>,
    pub imdb_url: StackString,
    pub episode: i32,
    pub season: i32,
    pub last_watched_at: Option<DateTimeWrapper>,
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
    /// # Errors
    /// Return error if db query fails
    pub async fn get_index(&self, pool: &PgPool) -> Result<Option<Uuid>, Error> {
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

    /// # Errors
    /// Return error if db query fails
    pub async fn get_watched_episode(
        pool: &PgPool,
        link: &str,
        season: i32,
        episode: i32,
    ) -> Result<Option<Self>, Error> {
        let query = query!(
            r#"
                SELECT twe.link as imdb_url,
                       ir.title,
                       ir.show,
                       twe.season,
                       twe.episode,
                       twe.last_watched_at
                FROM trakt_watched_episodes twe
                JOIN trakt_watchlist tw ON twe.link = tw.link
                JOIN imdb_ratings ir ON tw.show = ir.show
                WHERE ir.link = $link AND twe.season = $season AND twe.episode = $episode
            "#,
            link = link,
            season = season,
            episode = episode
        );
        let conn = pool.get().await?;
        query.fetch_opt(&conn).await.map_err(Into::into)
    }

    /// # Errors
    /// Return error if db query fails
    pub async fn insert_episode(&self, pool: &PgPool) -> Result<u64, Error> {
        let query = query!(
            r#"
                INSERT INTO trakt_watched_episodes (link, season, episode, last_watched_at)
                VALUES ($link, $season, $episode, $last_watched_at)
                ON CONFLICT DO NOTHING
            "#,
            link = self.imdb_url,
            season = self.season,
            episode = self.episode,
            last_watched_at = self.last_watched_at,
        );
        let conn = pool.get().await?;
        query.execute(&conn).await.map_err(Into::into)
    }

    /// # Errors
    /// Return error if db query fails
    pub async fn update_episode(&self, pool: &PgPool) -> Result<u64, Error> {
        let query = query!(
            r#"
                UPDATE trakt_watched_episodes
                SET last_watched_at = $last_watched_at
                WHERE link = $link
                  AND season = $season
                  AND episode = $episode
            "#,
            link = self.imdb_url,
            season = self.season,
            episode = self.episode,
            last_watched_at = self.last_watched_at,
        );
        let conn = pool.get().await?;
        query.execute(&conn).await.map_err(Into::into)
    }

    /// # Errors
    /// Return error if db query fails
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

/// # Errors
/// Return error if db query fails
pub async fn get_watched_shows_db(
    pool: &PgPool,
    show: &str,
    season: Option<i32>,
) -> Result<impl Stream<Item = Result<WatchedEpisode, PqError>>, Error> {
    let mut where_vec = Vec::new();
    if !show.is_empty() {
        where_vec.push(format_sstr!("ir.show='{show}'"));
    }
    if let Some(season) = season {
        where_vec.push(format_sstr!("twe.season={season}"));
    }

    let mut where_str = StackString::new();
    if !where_vec.is_empty() {
        write!(where_str, "WHERE {}", where_vec.join(" AND "))?;
    }
    let query = format_sstr!(
        r"
            SELECT twe.link as imdb_url,
                   ir.show,
                   ir.title,
                   twe.season,
                   twe.episode,
                   twe.last_watched_at
            FROM trakt_watched_episodes twe
            JOIN trakt_watchlist tw ON twe.link = tw.link
            JOIN imdb_ratings ir ON tw.show = ir.show
            {where_str}
            ORDER BY 3,4,5
        "
    );
    let query = query_dyn!(&query)?;
    let conn = pool.get().await?;
    query.fetch_streaming(&conn).await.map_err(Into::into)
}

#[derive(Serialize, Deserialize, Debug, Default, Eq, FromSqlRow)]
pub struct WatchedMovie {
    pub title: StackString,
    pub imdb_url: StackString,
    pub last_watched_at: Option<DateTimeWrapper>,
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
    /// # Errors
    /// Return error if db query fails
    pub async fn get_index(&self, pool: &PgPool) -> Result<Option<Uuid>, Error> {
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

    /// # Errors
    /// Return error if db query fails
    pub async fn get_watched_movie(pool: &PgPool, link: &str) -> Result<Option<Self>, Error> {
        let query = query!(
            r#"
                SELECT tw.link as imdb_url,
                       ir.title,
                       tw.last_watched_at
                FROM trakt_watched_movies tw
                JOIN imdb_ratings ir ON tw.link = ir.link
                WHERE tw.link = $link
            "#,
            link = link
        );
        let conn = pool.get().await?;
        query.fetch_opt(&conn).await.map_err(Into::into)
    }

    /// # Errors
    /// Return error if db query fails
    pub async fn insert_movie(&self, pool: &PgPool) -> Result<u64, Error> {
        let query = query!(
            r#"
                INSERT INTO trakt_watched_movies (link, last_watched_at)
                VALUES ($link, $last_watched_at)
            "#,
            link = self.imdb_url,
            last_watched_at = self.last_watched_at,
        );
        let conn = pool.get().await?;
        query.execute(&conn).await.map_err(Into::into)
    }

    /// # Errors
    /// Return error if db query fails
    pub async fn update_movie(&self, pool: &PgPool) -> Result<u64, Error> {
        let query = query!(
            r#"
                UPDATE trakt_watched_movies
                SET last_watched_at = $last_watched_at
                WHERE link = $link
            "#,
            link = self.imdb_url,
            last_watched_at = self.last_watched_at,
        );
        let conn = pool.get().await?;
        query.execute(&conn).await.map_err(Into::into)
    }

    /// # Errors
    /// Return error if db query fails
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

/// # Errors
/// Return error if db query fails
pub async fn get_watched_movies_db(
    pool: &PgPool,
) -> Result<impl Stream<Item = Result<WatchedMovie, PqError>>, Error> {
    let query = query!(
        r#"
            SELECT twm.link as imdb_url, ir.title, twm.last_watched_at
            FROM trakt_watched_movies twm
            JOIN imdb_ratings ir ON twm.link = ir.link
            ORDER BY ir.show
        "#
    );
    let conn = pool.get().await?;
    query.fetch_streaming(&conn).await.map_err(Into::into)
}

/// # Errors
/// Return error if db query fails
pub async fn sync_trakt_with_db(
    trakt: &TraktConnection,
    mc: &MovieCollection,
) -> Result<(), Error> {
    let watchlist_shows_db = Arc::new(get_watchlist_shows_db(&mc.pool).await?);
    trakt.init().await?;
    let watchlist_shows = trakt.get_watchlist_shows().await?;
    if watchlist_shows.is_empty() {
        return Ok(());
    }

    let futures: FuturesUnordered<_> = watchlist_shows
        .into_iter()
        .map(|(link, show)| {
            let watchlist_shows_db = watchlist_shows_db.clone();
            async move {
                if !watchlist_shows_db.contains(link.as_str()) {
                    show.insert_show(&mc.pool).await?;
                    mc.stdout.send(format_sstr!("insert watchlist {show}"));
                }
                Ok(())
            }
        })
        .collect();
    let results: Result<(), Error> = futures.try_collect().await;
    results?;

    let watched_shows_db: HashMap<(StackString, i32, i32), _> =
        get_watched_shows_db(&mc.pool, "", None)
            .await?
            .map_ok(|s| ((s.imdb_url.clone(), s.season, s.episode), s))
            .try_collect()
            .await?;
    let watched_shows_db = Arc::new(watched_shows_db);
    let watched_shows = trakt.get_watched_shows().await?;
    if watched_shows.is_empty() {
        return Ok(());
    }
    let futures: FuturesUnordered<_> = watched_shows
        .into_iter()
        .map(|(key, episode)| {
            let watched_shows_db = watched_shows_db.clone();
            async move {
                if let Some(episode_db) = watched_shows_db.get(&key) {
                    if episode_db.last_watched_at.is_none()
                        && episode.update_episode(&mc.pool).await? > 0
                    {
                        mc.stdout
                            .send(format_sstr!("update watched episode {episode}"));
                    }
                } else if episode.insert_episode(&mc.pool).await? > 0 {
                    mc.stdout
                        .send(format_sstr!("insert watched episode {episode}"));
                }
                Ok(())
            }
        })
        .collect();
    let results: Result<(), Error> = futures.try_collect().await;
    results?;

    let watched_movies_db: HashSet<_> =
        get_watched_movies_db(&mc.pool).await?.try_collect().await?;
    let watched_movies_db = Arc::new(watched_movies_db);
    let watched_movies = trakt.get_watched_movies().await?;
    let watched_movies = Arc::new(watched_movies);
    if watched_movies.is_empty() {
        return Ok(());
    }

    let futures: FuturesUnordered<_> = watched_movies
        .iter()
        .map(|movie: &WatchedMovie| {
            let watched_movies_db = watched_movies_db.clone();
            async move {
                if let Some(movie_db) = watched_movies_db.get(movie.imdb_url.as_str()) {
                    if movie_db.last_watched_at.is_none() && movie.update_movie(&mc.pool).await? > 0
                    {
                        mc.stdout.send(format_sstr!("update watched movie {movie}"));
                    }
                } else if !movie.imdb_url.is_empty() && movie.insert_movie(&mc.pool).await? > 0 {
                    mc.stdout.send(format_sstr!("insert watched movie {movie}"));
                }
                Ok(())
            }
        })
        .collect();
    let results: Result<(), Error> = futures.try_collect().await;
    results?;

    let futures: FuturesUnordered<_> = watched_movies_db
        .iter()
        .map(|movie| {
            let watched_movies = watched_movies.clone();
            async move {
                if !watched_movies.contains(movie.imdb_url.as_str()) {
                    movie.delete_movie(&mc.pool).await?;
                    mc.stdout.send(format_sstr!("delete watched {movie}"));
                }
                Ok(())
            }
        })
        .collect();
    futures.try_collect().await
}

async fn get_imdb_url_from_show(
    mc: &MovieCollection,
    show: &str,
) -> Result<Option<StackString>, Error> {
    let imdb_shows = mc.print_imdb_shows(show, false).await?;
    let result = if imdb_shows.is_empty() {
        let shows = mc.print_imdb_shows(show, true).await?;
        if shows.len() == 1 {
            Some(shows[0].link.clone())
        } else {
            for show in imdb_shows {
                debug!("{show}",);
            }
            None
        }
    } else if imdb_shows.len() > 1 {
        for show in imdb_shows {
            debug!("{show}",);
        }
        None
    } else {
        Some(imdb_shows[0].link.clone())
    };
    Ok(result)
}

async fn trakt_cal_list(trakt: &TraktConnection, mc: &MovieCollection) -> Result<(), Error> {
    trakt.init().await?;
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
            mc.stdout.send(format_sstr!("{show} {cal}"));
        }
    }
    Ok(())
}

/// # Errors
/// Return error if db query fails
pub async fn watchlist_add(
    trakt: &TraktConnection,
    mc: &MovieCollection,
    show: &str,
    imdb_link: Option<&str>,
) -> Result<Option<TraktResult>, Error> {
    trakt.init().await?;
    let imdb_url = if let Some(link) = imdb_link {
        link.into()
    } else if let Some(link) = get_imdb_url_from_show(mc, show).await? {
        link
    } else {
        return Ok(None);
    };
    let result = trakt.add_watchlist_show(&imdb_url).await?;
    mc.stdout.send(format_sstr!("result: {result}"));
    debug!("GOT HERE");
    if let Some(show_obj) = trakt
        .get_watchlist_shows()
        .await?
        .get_mut(imdb_url.as_str())
    {
        show_obj.show = Some(show.into());
        debug!("INSERT SHOW {show_obj}",);
        show_obj.insert_show(&mc.pool).await?;
    }
    Ok(Some(result))
}

/// # Errors
/// Return error if db query fails
pub async fn watchlist_rm(
    trakt: &TraktConnection,
    mc: &MovieCollection,
    show: &str,
) -> Result<Option<TraktResult>, Error> {
    if let Some(imdb_url) = get_imdb_url_from_show(mc, show).await? {
        let imdb_url_ = imdb_url.clone();
        trakt.init().await?;
        let result = trakt.remove_watchlist_show(&imdb_url_).await?;
        mc.stdout.send(format_sstr!("result: {result}"));
        if let Some(show) = WatchListShow::get_show_by_link(&imdb_url, &mc.pool).await? {
            show.delete_show(&mc.pool).await?;
        }
        Ok(Some(result))
    } else {
        Ok(None)
    }
}

async fn watchlist_list(mc: &MovieCollection) -> Result<(), Error> {
    let show_map = get_watchlist_shows_db(&mc.pool).await?;
    mc.stdout
        .send(show_map.iter().map(StackString::from_display).join("\n"));
    Ok(())
}

async fn watched_add(
    trakt: &TraktConnection,
    mc: &MovieCollection,
    show: &str,
    season: i32,
    episode: &[i32],
) -> Result<(), Error> {
    trakt.init().await?;
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
                last_watched_at: None,
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
    trakt.init().await?;
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
    let watched_shows: Vec<_> = get_watched_shows_db(&mc.pool, "", None)
        .await?
        .try_collect()
        .await?;
    let watched_movies: Vec<_> = get_watched_movies_db(&mc.pool).await?.try_collect().await?;

    if let Some(imdb_url) = get_imdb_url_from_show(mc, show).await? {
        let lines = watched_shows
            .iter()
            .filter_map(|show| {
                if season != -1 && show.season != season {
                    return None;
                }
                if show.imdb_url.as_str() == imdb_url.as_str() {
                    Some(StackString::from_display(show))
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
                    Some(StackString::from_display(show))
                } else {
                    None
                }
            })
            .join("\n");
        mc.stdout.send(lines);
    } else {
        mc.stdout.send(
            watched_shows
                .iter()
                .map(StackString::from_display)
                .join("\n"),
        );
        mc.stdout.send(
            watched_movies
                .iter()
                .map(StackString::from_display)
                .join("\n"),
        );
    }
    Ok(())
}

/// # Errors
/// Return error if db query fails
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
    mc.stdout.close().await.map_err(Into::into)
}

#[derive(FromSqlRow, PartialEq, Clone, Debug)]
pub struct TraktWatchedOutput {
    pub title: StackString,
    pub show_link: StackString,
    pub episode_title: StackString,
    pub season: i32,
    pub episode: i32,
    pub episode_url: StackString,
    pub airdate: Option<Date>,
    pub rating: Option<Decimal>,
    pub last_watched_at: DateTimeWrapper,
}

/// # Errors
/// Returns error if formatting fails
pub async fn get_trakt_watched_output_db(
    pool: &PgPool,
    start_timestamp: Option<OffsetDateTime>,
    offset: Option<usize>,
    limit: Option<usize>,
) -> Result<impl Stream<Item = Result<TraktWatchedOutput, PqError>>, Error> {
    // WHERE twe.last_watched_at IS NOT NULL
    let mut constraints = vec!["twe.last_watched_at IS NOT NULL"];
    let mut bindings = Vec::new();
    if let Some(start_timestamp) = &start_timestamp {
        constraints.push("twe.last_watched_at > $start_timestamp");
        bindings.push(("start_timestamp", start_timestamp as Parameter));
    }
 
    let query = format_sstr!(
        r"
            SELECT ir.title,
                   ir.link as show_link,
                   ie.eptitle as episode_title,
                   twe.season,
                   twe.episode,
                   ie.epurl as episode_url,
                   ie.airdate,
                   ie.rating,
                   twe.last_watched_at
            FROM trakt_watched_episodes twe
            JOIN imdb_ratings ir ON twe.link = ir.link
            JOIN imdb_episodes ie ON ie.show = ir.show AND ie.season = twe.season AND ie.episode = twe.episode
            {where_str}
            ORDER BY twe.last_watched_at DESC
            {limit}
            {offset}
        ",
        where_str = format_sstr!("WHERE {}", constraints.join(" AND ")),
        limit = if let Some(limit) = limit {
            format_sstr!("LIMIT {limit}")
        } else {
            StackString::new()
        },
        offset = if let Some(offset) = offset {
            format_sstr!("OFFSET {offset}")
        } else {
            StackString::new()
        }
    );
    let query: Query = query_dyn!(&query, ..bindings)?;
    let conn = pool.get().await?;
    query.fetch_streaming(&conn).await.map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use anyhow::Error;
    use futures::TryStreamExt;
    use crate::config::Config;
    use crate::pgpool::PgPool;
    use crate::trakt_utils::get_trakt_watched_output_db;

    #[tokio::test]
    #[ignore]
    async fn test_trakt_watched_output_db() -> Result<(), Error> {
        let config = Config::with_config()?;
        let pool = PgPool::new(&config.pgurl)?;

        let output: Vec<_> = get_trakt_watched_output_db(&pool, None, None, Some(10)).await?.try_collect().await?;

        println!("{output:#?}");
        assert_eq!(output.len(), 10);
        Ok(())
    }
}