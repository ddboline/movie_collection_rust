use failure::Error;
use std::fmt;

use crate::movie_collection::PgPool;
use crate::utils::option_string_wrapper;

#[derive(Default, Clone, Debug)]
pub struct ImdbRatings {
    pub index: i32,
    pub show: String,
    pub title: Option<String>,
    pub link: Option<String>,
    pub rating: Option<f64>,
    pub istv: Option<bool>,
    pub source: Option<String>,
}

impl fmt::Display for ImdbRatings {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "{} {} {} {} {} {} {} ",
            self.index,
            self.show,
            option_string_wrapper(&self.title),
            option_string_wrapper(&self.link),
            self.rating.unwrap_or(-1.0),
            self.istv.unwrap_or(false),
            option_string_wrapper(&self.source),
        )
    }
}

impl ImdbRatings {
    pub fn insert_show(&self, pool: &PgPool) -> Result<(), Error> {
        let query = r#"
            INSERT INTO imdb_ratings
            (show, title, link, rating, istv, source)
            VALUES
            ($1, $2, $3, $4, $5, $6)
        "#;
        pool.get()?.execute(
            query,
            &[
                &self.show,
                &self.title,
                &self.link,
                &self.rating,
                &self.istv,
                &self.source,
            ],
        )?;
        Ok(())
    }

    pub fn update_show(&self, pool: &PgPool) -> Result<(), Error> {
        let query = r#"
            UPDATE imdb_ratings SET rating=$1,title=$2 WHERE show=$3
        "#;
        pool.get()?
            .execute(query, &[&self.rating, &self.title, &self.show])?;
        Ok(())
    }

    pub fn get_show_by_link(link: &str, pool: &PgPool) -> Result<Option<ImdbRatings>, Error> {
        let query = r#"
            SELECT index, show, title, link, rating, istv, source
            FROM imdb_ratings
            WHERE link = $1
        "#;
        if let Some(row) = pool.get()?.query(query, &[&link])?.iter().nth(0) {
            let index: i32 = row.get(0);
            let show: String = row.get(1);
            let title: Option<String> = row.get(2);
            let link: Option<String> = row.get(3);
            let rating: Option<f64> = row.get(4);
            let istv: Option<bool> = row.get(5);
            let source: Option<String> = row.get(6);
            return Ok(Some(ImdbRatings {
                index,
                show,
                title,
                link,
                rating,
                istv,
                source,
            }));
        } else {
            Ok(None)
        }
    }
}
