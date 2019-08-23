use chrono::{DateTime, Utc};
use failure::Error;
use std::fmt;

use crate::common::pgpool::PgPool;
use crate::common::row_index_trait::RowIndexTrait;
use crate::common::tv_show_source::TvShowSource;
use crate::common::utils::map_result;
use crate::common::utils::option_string_wrapper;

#[derive(Default, Clone, Debug, Serialize, Deserialize)]
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
                .unwrap_or_else(|| "".to_string()),
        )
    }
}

impl ImdbRatings {
    pub fn insert_show(&self, pool: &PgPool) -> Result<(), Error> {
        let query = r#"
            INSERT INTO imdb_ratings
            (show, title, link, rating, istv, source, last_modified)
            VALUES
            ($1, $2, $3, $4, $5, $6, now())
        "#;
        debug!("{:?}", self);
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
            UPDATE imdb_ratings
            SET rating=$1,title=$2,last_modified=now()
            WHERE show=$3
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
            let index: i32 = row.get_idx(0)?;
            let show: String = row.get_idx(1)?;
            let title: Option<String> = row.get_idx(2)?;
            let link: String = row.get_idx(3)?;
            let rating: Option<f64> = row.get_idx(4)?;
            let istv: Option<bool> = row.get_idx(5)?;
            let source: Option<String> = row.get_idx(6)?;
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

    pub fn get_shows_after_timestamp(
        timestamp: DateTime<Utc>,
        pool: &PgPool,
    ) -> Result<Vec<ImdbRatings>, Error> {
        let query = r#"
            SELECT index, show, title, link, rating, istv, source
            FROM imdb_ratings
            WHERE last_modified >= $1
        "#;
        let shows: Vec<_> = pool
            .get()?
            .query(query, &[&timestamp])?
            .iter()
            .map(|row| {
                let index: i32 = row.get_idx(0)?;
                let show: String = row.get_idx(1)?;
                let title: Option<String> = row.get_idx(2)?;
                let link: String = row.get_idx(3)?;
                let rating: Option<f64> = row.get_idx(4)?;
                let istv: Option<bool> = row.get_idx(5)?;
                let source: Option<String> = row.get_idx(6)?;
                let source: Option<TvShowSource> = match source {
                    Some(s) => s.parse().ok(),
                    None => None,
                };
                Ok(ImdbRatings {
                    index,
                    show,
                    title,
                    link,
                    rating,
                    istv,
                    source,
                })
            })
            .collect();
        map_result(shows)
    }

    pub fn get_string_vec(&self) -> Vec<String> {
        vec![
            self.show.to_string(),
            option_string_wrapper(&self.title).to_string(),
            self.link.to_string(),
            self.rating.unwrap_or(-1.0).to_string(),
            self.istv.unwrap_or(false).to_string(),
            self.source
                .as_ref()
                .map(|s| s.to_string())
                .unwrap_or_else(|| "".to_string()),
        ]
    }
}
