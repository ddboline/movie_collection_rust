extern crate chrono;
extern crate failure;
extern crate r2d2;
extern crate r2d2_postgres;
extern crate rayon;
extern crate reqwest;
extern crate select;

use chrono::NaiveDate;
use failure::Error;
use std::fmt;

use crate::common::pgpool::PgPool;

#[derive(Clone)]
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
            let id: i32 = row.get(0);
            Ok(Some(id))
        } else {
            Ok(None)
        }
    }

    pub fn insert_episode(&self, pool: &PgPool) -> Result<(), Error> {
        let query = r#"
            INSERT INTO imdb_episodes
            (show, season, episode, airdate, rating, eptitle, epurl)
            VALUES
            ($1, $2, $3, $4, RATING, $5, $6)
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
            UPDATE imdb_episodes SET rating=RATING,eptitle=$1,epurl=$2,airdate=$3 WHERE show=$4 AND season=$5 AND episode=$6
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
            self.show.clone(),
            self.title.clone(),
            self.season.to_string(),
            self.episode.to_string(),
            self.airdate.to_string(),
            self.rating.to_string(),
            self.eptitle.clone(),
            self.epurl.clone(),
        ]
    }
}
