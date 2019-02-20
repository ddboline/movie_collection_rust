use failure::Error;
use std::fmt;

use crate::common::pgpool::PgPool;
use crate::common::tv_show_source::TvShowSource;
use crate::common::utils::option_string_wrapper;

#[derive(Default, Clone, Debug)]
pub struct ImdbRatings {
    pub index: i32,
    pub show: String,
    pub title: Option<String>,
    pub link: String,
    pub rating: Option<f64>,
    pub istv: Option<bool>,
    pub source: Option<TvShowSource>,
}

impl fmt::Display for ImdbRatings {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "{} {} {} {} {} {} ",
            self.show,
            option_string_wrapper(&self.title),
            self.link,
            self.rating.unwrap_or(-1.0),
            self.istv.unwrap_or(false),
            self.source
                .as_ref()
                .map(|s| s.to_string())
                .unwrap_or("".to_string()),
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
        let source = self.source.as_ref().map(|s| s.to_string());
        pool.get()?.execute(
            query,
            &[
                &self.show,
                &self.title,
                &self.link,
                &self.rating,
                &self.istv,
                &source,
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
            WHERE (link = $1 OR show = $1)
        "#;
        if let Some(row) = pool.get()?.query(query, &[&link])?.iter().nth(0) {
            let index: i32 = row.get(0);
            let show: String = row.get(1);
            let title: Option<String> = row.get(2);
            let link: String = row.get(3);
            let rating: Option<f64> = row.get(4);
            let istv: Option<bool> = row.get(5);
            let source: Option<String> = row.get(6);
            let source: Option<TvShowSource> = match source {
                Some(s) => s.parse().ok(),
                None => None,
            };
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

    pub fn get_string_vec(&self) -> Vec<String> {
        vec![
            self.show.clone(),
            option_string_wrapper(&self.title).to_string(),
            self.link.clone(),
            self.rating.unwrap_or(-1.0).to_string(),
            self.istv.unwrap_or(false).to_string(),
            self.source
                .as_ref()
                .map(|s| s.to_string())
                .unwrap_or("".to_string()),
        ]
    }
}
