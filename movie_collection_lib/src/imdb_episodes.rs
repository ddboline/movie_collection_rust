use anyhow::Error;
use chrono::{DateTime, NaiveDate, Utc};
use postgres_query::FromSqlRow;
use serde::{Deserialize, Serialize};
use std::fmt;

use crate::pgpool::PgPool;

#[derive(Clone, Serialize, Deserialize, FromSqlRow)]
pub struct ImdbEpisodes {
    pub show: String,
    pub title: String,
    pub season: i32,
    pub episode: i32,
    pub airdate: NaiveDate,
    pub rating: f64,
    pub eptitle: String,
    pub epurl: String,
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
    pub fn new() -> Self {
        Self {
            show: "".to_string(),
            title: "".to_string(),
            season: -1,
            episode: -1,
            airdate: NaiveDate::from_ymd(1970, 1, 1),
            rating: -1.0,
            eptitle: "".to_string(),
            epurl: "".to_string(),
        }
    }

    pub async fn get_index(&self, pool: &PgPool) -> Result<Option<i32>, Error> {
        let query = postgres_query::query!(
            r#"
            SELECT id
            FROM imdb_episodes
            WHERE show=$show AND season=$season AND episode=$episode
        "#,
            show = self.show,
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

    pub async fn from_index(idx: i32, pool: &PgPool) -> Result<Option<Self>, Error> {
        let query = r#"
            SELECT a.show, b.title, a.season, a.episode, a.airdate,
                   cast(a.rating as double precision) as rating, a.eptitle, a.epurl
            FROM imdb_episodes a
            JOIN imdb_ratings b ON a.show = b.show
            WHERE a.id = $1"#;
        if let Some(row) = pool.get().await?.query(query, &[&idx]).await?.get(0) {
            let epi = Self::from_row(row)?;
            Ok(Some(epi))
        } else {
            Ok(None)
        }
    }

    pub async fn get_episodes_after_timestamp(
        timestamp: DateTime<Utc>,
        pool: &PgPool,
    ) -> Result<Vec<Self>, Error> {
        let query = postgres_query::query!(
            r#"
            SELECT a.show, b.title, a.season, a.episode, a.airdate,
                   cast(a.rating as double precision) as rating, a.eptitle, a.epurl
            FROM imdb_episodes a
            JOIN imdb_ratings b ON a.show = b.show
            WHERE a.last_modified >= $timestamp
        "#,
            timestamp = timestamp
        );
        pool.get()
            .await?
            .query(query.sql(), query.parameters())
            .await?
            .iter()
            .map(|row| {
                let epi = Self::from_row(row)?;
                Ok(epi)
            })
            .collect()
    }

    pub async fn insert_episode(&self, pool: &PgPool) -> Result<(), Error> {
        if self.get_index(pool).await?.is_some() {
            return self.update_episode(pool).await;
        }
        let query = postgres_query::query_dyn!(
            &format!(
                r#"
                    INSERT INTO imdb_episodes
                    (show, season, episode, airdate, rating, eptitle, epurl, last_modified)
                    VALUES
                    ($show, $season, $episode, $airdate, {}, $eptitle, $epurl, now())
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
        pool.get()
            .await?
            .execute(query.sql(), query.parameters())
            .await
            .map(|_| ())
            .map_err(Into::into)
    }

    pub async fn update_episode(&self, pool: &PgPool) -> Result<(), Error> {
        let query = postgres_query::query_dyn!(
            &format!(
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
        pool.get()
            .await?
            .execute(query.sql(), query.parameters())
            .await
            .map(|_| ())
            .map_err(Into::into)
    }

    pub fn get_string_vec(&self) -> Vec<String> {
        vec![
            self.show.to_string(),
            self.title.to_string(),
            self.season.to_string(),
            self.episode.to_string(),
            self.airdate.to_string(),
            self.rating.to_string(),
            self.eptitle.to_string(),
            self.epurl.to_string(),
        ]
    }
}
