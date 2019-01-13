use chrono::NaiveDate;
use failure::Error;
use std::fmt;

use crate::movie_collection::PgPool;

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

impl ImdbEpisodes {
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
            UPDATE imdb_episodes SET rating=RATING,eptitle=$1,epurl=$2 WHERE show=$3 AND season=$4 AND episode=$5
        "#;
        let query = query.replace("RATING", &self.rating.to_string());

        pool.get()?.execute(
            &query,
            &[
                &self.eptitle,
                &self.epurl,
                &self.show,
                &self.season,
                &self.episode,
            ],
        )?;
        Ok(())
    }
}
