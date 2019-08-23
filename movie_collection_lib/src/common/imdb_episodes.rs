use chrono::{DateTime, NaiveDate, Utc};
use failure::Error;
use std::fmt;

use crate::common::pgpool::PgPool;
use crate::common::row_index_trait::RowIndexTrait;
use crate::common::utils::map_result;

#[derive(Clone, Serialize, Deserialize)]
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
    fn default() -> ImdbEpisodes {
        ImdbEpisodes::new()
    }
}

impl ImdbEpisodes {
    pub fn new() -> ImdbEpisodes {
        ImdbEpisodes {
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

    pub fn get_index(&self, pool: &PgPool) -> Result<Option<i32>, Error> {
        let query = r#"
            SELECT id
            FROM imdb_episodes
            WHERE show=$1 AND season=$2 AND episode=$3
        "#;
        if let Some(row) = pool
            .get()?
            .query(query, &[&self.show, &self.season, &self.episode])?
            .iter()
            .nth(0)
        {
            let id: i32 = row.get_idx(0)?;
            Ok(Some(id))
        } else {
            Ok(None)
        }
    }

    pub fn from_index(idx: i32, pool: &PgPool) -> Result<Option<ImdbEpisodes>, Error> {
        let query = r#"
            SELECT a.show, b.title, a.season, a.episode, a.airdate,
                   cast(a.rating as double precision), a.eptitle, a.epurl
            FROM imdb_episodes a
            JOIN imdb_ratings b ON a.show = b.show
            WHERE a.id = $1"#;
        if let Some(row) = pool.get()?.query(query, &[&idx])?.iter().nth(0) {
            let show: String = row.get_idx(0)?;
            let title: String = row.get_idx(1)?;
            let season: i32 = row.get_idx(2)?;
            let episode: i32 = row.get_idx(3)?;
            let airdate: NaiveDate = row.get_idx(4)?;
            let rating: f64 = row.get_idx(5)?;
            let eptitle: String = row.get_idx(6)?;
            let epurl: String = row.get_idx(7)?;

            let epi = ImdbEpisodes {
                show,
                title,
                season,
                episode,
                airdate,
                rating,
                eptitle,
                epurl,
            };

            Ok(Some(epi))
        } else {
            Ok(None)
        }
    }

    pub fn get_episodes_after_timestamp(
        timestamp: DateTime<Utc>,
        pool: &PgPool,
    ) -> Result<Vec<ImdbEpisodes>, Error> {
        let query = r#"
            SELECT a.show, b.title, a.season, a.episode, a.airdate,
                   cast(a.rating as double precision), a.eptitle, a.epurl
            FROM imdb_episodes a
            JOIN imdb_ratings b ON a.show = b.show
            WHERE a.last_modified >= $1
        "#;
        let episodes: Vec<_> = pool
            .get()?
            .query(query, &[&timestamp])?
            .iter()
            .map(|row| {
                let show: String = row.get_idx(0)?;
                let title: String = row.get_idx(1)?;
                let season: i32 = row.get_idx(2)?;
                let episode: i32 = row.get_idx(3)?;
                let airdate: NaiveDate = row.get_idx(4)?;
                let rating: f64 = row.get_idx(5)?;
                let eptitle: String = row.get_idx(6)?;
                let epurl: String = row.get_idx(7)?;

                Ok(ImdbEpisodes {
                    show,
                    title,
                    season,
                    episode,
                    airdate,
                    rating,
                    eptitle,
                    epurl,
                })
            })
            .collect();
        map_result(episodes)
    }

    pub fn insert_episode(&self, pool: &PgPool) -> Result<(), Error> {
        if self.get_index(pool)?.is_some() {
            return self.update_episode(pool);
        }
        let query = r#"
            INSERT INTO imdb_episodes
            (show, season, episode, airdate, rating, eptitle, epurl, last_modified)
            VALUES
            ($1, $2, $3, $4, RATING, $5, $6, now())
        "#;
        let query = query.replace("RATING", &self.rating.to_string());
        pool.get()?.execute(
            &query,
            &[
                &self.show,
                &self.season,
                &self.episode,
                &self.airdate,
                &self.eptitle,
                &self.epurl,
            ],
        )?;
        Ok(())
    }

    pub fn update_episode(&self, pool: &PgPool) -> Result<(), Error> {
        let query = r#"
            UPDATE imdb_episodes
            SET rating=RATING,eptitle=$1,epurl=$2,airdate=$3,last_modified=now()
            WHERE show=$4 AND season=$5 AND episode=$6
        "#;
        let query = query.replace("RATING", &self.rating.to_string());

        pool.get()?.execute(
            &query,
            &[
                &self.eptitle,
                &self.epurl,
                &self.airdate,
                &self.show,
                &self.season,
                &self.episode,
            ],
        )?;
        Ok(())
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
