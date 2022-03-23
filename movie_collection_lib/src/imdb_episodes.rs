use anyhow::Error;
use chrono::{DateTime, NaiveDate, Utc};
use postgres_query::{query, query_dyn, FromSqlRow, Parameter};
use serde::{Deserialize, Serialize};
use smallvec::{smallvec, SmallVec};
use stack_string::{format_sstr, StackString};
use std::{fmt, fmt::Write};

use crate::pgpool::PgPool;

#[derive(Clone, Serialize, Deserialize, FromSqlRow)]
pub struct ImdbEpisodes {
    pub show: StackString,
    pub title: StackString,
    pub season: i32,
    pub episode: i32,
    pub airdate: NaiveDate,
    pub rating: f64,
    pub eptitle: StackString,
    pub epurl: StackString,
}

impl fmt::Display for ImdbEpisodes {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "{} {} {} {} {} {} {} {}",
            self.show,
            self.title,
            self.season,
            self.episode,
            self.airdate,
            self.rating,
            self.eptitle,
            self.epurl,
        )
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
            airdate: NaiveDate::from_ymd(1970, 1, 1),
            rating: -1.0,
            eptitle: "".into(),
            epurl: "".into(),
        }
    }

    /// # Errors
    /// Returns error if db query fails
    pub async fn get_index(&self, pool: &PgPool) -> Result<Option<i32>, Error> {
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
    pub async fn from_index(idx: i32, pool: &PgPool) -> Result<Option<Self>, Error> {
        let query = query!(
            r#"
                SELECT a.show, b.title, a.season, a.episode, a.airdate,
                       cast(a.rating as double precision) as rating, a.eptitle, a.epurl
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
        timestamp: DateTime<Utc>,
        pool: &PgPool,
    ) -> Result<Vec<Self>, Error> {
        let query = query!(
            r#"
                SELECT a.show, b.title, a.season, a.episode, a.airdate,
                       cast(a.rating as double precision) as rating, a.eptitle, a.epurl
                FROM imdb_episodes a
                JOIN imdb_ratings b ON a.show = b.show
                WHERE a.last_modified >= $timestamp
            "#,
            timestamp = timestamp
        );
        let conn = pool.get().await?;
        query.fetch(&conn).await.map_err(Into::into)
    }

    /// # Errors
    /// Returns error if db queries fail
    pub async fn get_episodes_by_show_season_episode(
        show: &str,
        season: Option<i32>,
        episode: Option<i32>,
        pool: &PgPool,
    ) -> Result<Vec<Self>, Error> {
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
                SELECT a.show, b.title, a.season, a.episode, a.airdate,
                        cast(a.rating as double precision) as rating, a.eptitle, a.epurl
                FROM imdb_episodes a
                JOIN imdb_ratings b ON a.show = b.show
                WHERE {constraints}
                ORDER BY a.season, a.episode
                "#
            ),
            ..bindings
        )?;
        let conn = pool.get().await?;
        query.fetch(&conn).await.map_err(Into::into)
    }

    /// # Errors
    /// Returns error if db query fails
    pub async fn insert_episode(&self, pool: &PgPool) -> Result<(), Error> {
        if self.get_index(pool).await?.is_some() {
            return self.update_episode(pool).await;
        }
        let query = query_dyn!(
            &format_sstr!(
                r#"
                    INSERT INTO imdb_episodes (
                        show, season, episode, airdate, rating, eptitle, epurl, last_modified
                    ) VALUES (
                        $show, $season, $episode, $airdate, {}, $eptitle, $epurl, now()
                    )
                "#,
                self.rating
            ),
            show = self.show,
            season = self.season,
            episode = self.episode,
            airdate = self.airdate,
            eptitle = self.eptitle,
            epurl = self.epurl
        )?;
        let conn = pool.get().await?;
        query.execute(&conn).await.map(|_| ()).map_err(Into::into)
    }

    /// # Errors
    /// Returns error if db query fails
    pub async fn update_episode(&self, pool: &PgPool) -> Result<(), Error> {
        let query = query_dyn!(
            &format_sstr!(
                r#"
                    UPDATE imdb_episodes
                    SET rating={},eptitle=$eptitle,epurl=$epurl,airdate=$airdate,last_modified=now()
                    WHERE show=$show AND season=$season AND episode=$episode
                "#,
                self.rating
            ),
            eptitle = self.eptitle,
            epurl = self.epurl,
            airdate = self.airdate,
            show = self.show,
            season = self.season,
            episode = self.episode
        )?;
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
            StackString::from_display(self.airdate),
            StackString::from_display(self.rating),
            self.eptitle.clone(),
            self.epurl.clone(),
        ]
    }
}

#[derive(Debug, Default, FromSqlRow, Clone)]
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
    pub async fn get_seasons(show: &str, pool: &PgPool) -> Result<Vec<ImdbSeason>, Error> {
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
        query.fetch(&conn).await.map_err(Into::into)
    }
}
