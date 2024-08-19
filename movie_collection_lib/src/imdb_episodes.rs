use anyhow::Error;
use futures::Stream;
use postgres_query::{query, query_dyn, Error as PqError, FromSqlRow, Parameter};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use smallvec::{smallvec, SmallVec};
use stack_string::{format_sstr, StackString};
use std::fmt;
use time::{Date, OffsetDateTime};
use uuid::Uuid;

use crate::pgpool::PgPool;

#[derive(Clone, Serialize, Deserialize, FromSqlRow, PartialEq, Eq, Debug)]
pub struct ImdbEpisodes {
    pub show: StackString,
    pub title: StackString,
    pub season: i32,
    pub episode: i32,
    pub airdate: Option<Date>,
    pub rating: Option<Decimal>,
    pub eptitle: StackString,
    pub epurl: StackString,
    pub id: Uuid,
}

impl fmt::Display for ImdbEpisodes {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "{} {} {} {}",
            self.show, self.title, self.season, self.episode
        )?;
        if let Some(airdate) = self.airdate {
            write!(f, " {airdate}")?;
        } else {
            write!(f, " ****-**-**")?;
        }
        if let Some(rating) = self.rating {
            write!(f, "{rating}")?;
        } else {
            write!(f, " --")?;
        }
        write!(f, " {} {}", self.eptitle, self.epurl)
    }
}

impl Default for ImdbEpisodes {
    fn default() -> Self {
        Self::new()
    }
}

impl ImdbEpisodes {
    #[must_use]
    pub fn new() -> Self {
        Self {
            show: "".into(),
            title: "".into(),
            season: -1,
            episode: -1,
            airdate: None,
            rating: None,
            eptitle: "".into(),
            epurl: "".into(),
            id: Uuid::new_v4(),
        }
    }

    /// # Errors
    /// Returns error if db query fails
    pub async fn get_index(&self, pool: &PgPool) -> Result<Option<Uuid>, Error> {
        let query = query!(
            r#"
                SELECT id
                FROM imdb_episodes
                WHERE show=$show AND season=$season AND episode=$episode
            "#,
            show = self.show,
            season = self.season,
            episode = self.episode
        );
        let conn = pool.get().await?;
        let id = query.fetch_opt(&conn).await?;
        Ok(id.map(|(x,)| x))
    }

    /// # Errors
    /// Returns error if db query fails
    pub async fn from_index(idx: Uuid, pool: &PgPool) -> Result<Option<Self>, Error> {
        let query = query!(
            r#"
                SELECT a.*, b.*
                FROM imdb_episodes a
                JOIN imdb_ratings b ON a.show = b.show
                WHERE a.id = $id
            "#,
            id = idx
        );
        let conn = pool.get().await?;
        query.fetch_opt(&conn).await.map_err(Into::into)
    }

    /// # Errors
    /// Returns error if db query fails
    pub async fn get_episodes_after_timestamp(
        timestamp: OffsetDateTime,
        pool: &PgPool,
    ) -> Result<impl Stream<Item = Result<Self, PqError>>, Error> {
        let query = query!(
            r#"
                SELECT a.show, b.*
                FROM imdb_episodes a
                JOIN imdb_ratings b ON a.show = b.show
                WHERE a.last_modified >= $timestamp
            "#,
            timestamp = timestamp
        );
        let conn = pool.get().await?;
        query.fetch_streaming(&conn).await.map_err(Into::into)
    }

    /// # Errors
    /// Returns error if db queries fail
    pub async fn get_episodes_by_show_season_episode(
        show: &str,
        season: Option<i32>,
        episode: Option<i32>,
        pool: &PgPool,
    ) -> Result<impl Stream<Item = Result<Self, PqError>>, Error> {
        let mut constraints: SmallVec<[_; 3]> = smallvec!["a.show = $show"];
        let mut bindings: SmallVec<[_; 3]> = smallvec![("show", &show as Parameter)];
        if let Some(season) = &season {
            constraints.push("a.season = $season");
            bindings.push(("season", season as Parameter));
        }
        if let Some(episode) = &episode {
            constraints.push("a.episode = $episode");
            bindings.push(("episode", episode as Parameter));
        }
        let constraints = constraints.join(" AND ");

        let query = query_dyn!(
            &format_sstr!(
                r#"
                SELECT a.*, b.title
                FROM imdb_episodes a
                JOIN imdb_ratings b ON a.show = b.show
                WHERE {constraints}
                ORDER BY a.season, a.episode
                "#
            ),
            ..bindings
        )?;
        let conn = pool.get().await?;
        query.fetch_streaming(&conn).await.map_err(Into::into)
    }

    /// # Errors
    /// Returns error if db query fails
    pub async fn get_episodes_not_recorded_in_trakt(
        pool: &PgPool,
    ) -> Result<impl Stream<Item = Result<Self, PqError>>, Error> {
        let query = query!(
            r#"
                SELECT ie.show, ir.title, ie.season, ie.episode, ie.airdate,
                       ie.rating, ie.eptitle, ie.epurl, ie.id
                FROM plex_event pe
                JOIN plex_filename pf ON pf.metadata_key = pe.metadata_key
                JOIN movie_collection mc ON mc.idx = pf.collection_id
                JOIN imdb_episodes ie ON ie.id = mc.episode_id
                JOIN imdb_ratings ir ON ir.index = mc.show_id
                JOIN trakt_watchlist tw ON tw.link = ir.link
                LEFT JOIN trakt_watched_episodes twe ON twe.link = ir.link AND twe.season = ie.season AND twe.episode = ie.episode
                WHERE twe.link IS NULL
                GROUP BY 1,2,3,4,5,6,7,8,9
                ORDER BY 1,2,3,4,5,6,7,8,9
            "#,
        );
        let conn = pool.get().await?;
        query.fetch_streaming(&conn).await.map_err(Into::into)
    }

    /// # Errors
    /// Returns error if db query fails
    pub async fn insert_episode(&self, pool: &PgPool) -> Result<(), Error> {
        if self.get_index(pool).await?.is_some() {
            return self.update_episode(pool).await;
        }
        let query = query!(
            r#"
                INSERT INTO imdb_episodes (
                    show, season, episode, airdate, rating, eptitle, epurl, id
                ) VALUES (
                    $show, $season, $episode, $airdate, $rating, $eptitle, $epurl, $id
                )
            "#,
            show = self.show,
            season = self.season,
            episode = self.episode,
            airdate = self.airdate,
            rating = self.rating,
            eptitle = self.eptitle,
            epurl = self.epurl,
            id = self.id,
        );
        let conn = pool.get().await?;
        query.execute(&conn).await.map(|_| ()).map_err(Into::into)
    }

    /// # Errors
    /// Returns error if db query fails
    pub async fn update_episode(&self, pool: &PgPool) -> Result<(), Error> {
        let query = query!(
            r#"
                UPDATE imdb_episodes
                SET rating=$rating,eptitle=$eptitle,epurl=$epurl,airdate=$airdate,last_modified=now()
                WHERE show=$show AND season=$season AND episode=$episode
            "#,
            rating = self.rating,
            eptitle = self.eptitle,
            epurl = self.epurl,
            airdate = self.airdate,
            show = self.show,
            season = self.season,
            episode = self.episode
        );
        let conn = pool.get().await?;
        query.execute(&conn).await.map(|_| ()).map_err(Into::into)
    }

    #[must_use]
    pub fn get_string_vec(&self) -> Vec<StackString> {
        vec![
            self.show.clone(),
            self.title.clone(),
            StackString::from_display(self.season),
            StackString::from_display(self.episode),
            self.airdate
                .map_or_else(StackString::new, StackString::from_display),
            self.rating
                .map_or_else(StackString::new, StackString::from_display),
            self.eptitle.clone(),
            self.epurl.clone(),
        ]
    }
}

#[derive(Debug, Default, FromSqlRow, Clone, PartialEq, Eq)]
pub struct ImdbSeason {
    pub show: StackString,
    pub title: StackString,
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
    #[must_use]
    pub fn get_string_vec(&self) -> Vec<StackString> {
        vec![
            self.show.clone(),
            self.title.clone(),
            StackString::from_display(self.season),
            StackString::from_display(self.nepisodes),
        ]
    }
}

impl ImdbSeason {
    /// # Errors
    /// Returns error if db queries fail
    pub async fn get_seasons(
        show: &str,
        pool: &PgPool,
    ) -> Result<impl Stream<Item = Result<ImdbSeason, PqError>>, Error> {
        let query = query!(
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
        let conn = pool.get().await?;
        query.fetch_streaming(&conn).await.map_err(Into::into)
    }
}

#[cfg(test)]
mod tests {
    use anyhow::Error;
    use futures::TryStreamExt;

    use crate::{
        config::Config, imdb_episodes::{ImdbEpisodes, ImdbSeason}, imdb_ratings::ImdbRatings, pgpool::PgPool
    };

    #[tokio::test]
    #[ignore]
    async fn test_imdb_episodes() -> Result<(), Error> {
        let config = Config::with_config()?;
        let pool = PgPool::new(&config.pgurl)?;

        let episodes: Vec<_> =
            ImdbEpisodes::get_episodes_by_show_season_episode("the_sopranos", None, None, &pool)
                .await?
                .try_collect()
                .await?;
        assert_eq!(episodes.len(), 86);
        let episode = episodes.first().unwrap();
        println!("{episode:?}");
        assert_eq!(episode.season, 1);
        assert_eq!(episode.episode, 1);
        let index = episode.get_index(&pool).await?.unwrap();
        assert_eq!(index, episode.id);
        let episode0 = ImdbEpisodes::from_index(episode.id, &pool).await?.unwrap();
        assert_eq!(episode.id, episode0.id);
        let seasons: Vec<_> = ImdbSeason::get_seasons("the_sopranos", &pool)
            .await?
            .try_collect()
            .await?;
        assert_eq!(seasons.len(), 6);
        let show = ImdbRatings::get_show_by_link("the_sopranos", &pool).await?.unwrap();
        assert_eq!(show.show, episode.show);
        Ok(())
    }
}
