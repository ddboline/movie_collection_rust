extern crate failure;
extern crate r2d2;
extern crate r2d2_postgres;

use failure::Error;
use r2d2::Pool;
use r2d2_postgres::{PostgresConnectionManager, TlsMode};

use crate::config::Config;

pub type PgPool = Pool<PostgresConnectionManager>;

#[derive(Debug)]
pub struct MovieCollection {
    pub config: Config,
    pub pool: PgPool,
}

impl MovieCollection {
    pub fn new() -> MovieCollection {
        let config = Config::with_config();
        let manager = PostgresConnectionManager::new(config.pgurl.clone(), TlsMode::None)
            .expect("Failed to open DB connection");
        MovieCollection {
            config,
            pool: Pool::new(manager).expect("Failed to open DB connection"),
        }
    }

    pub fn print_imdb_shows(&self, show: &str, istv: bool) -> Result<Vec<String>, Error> {
        let conn = self.pool.get()?;

        let query = format!("SELECT show FROM imdb_ratings WHERE show like '%{}%'", show);
        let query = if istv {
            format!("{} AND istv", query)
        } else {
            query
        };
        let shows: Vec<String> = conn.query(&query, &[])?.iter().map(|r| r.get(0)).collect();

        let shows = if shows.contains(&show.to_string()) {
            vec![show.to_string()]
        } else {
            shows
        };

        for show in &shows {
            let query = r#"
                SELECT show, title, link, rating
                FROM imdb_ratings
                WHERE link is not null AND rating is not null"#;
            let query = format!("{} AND show = '{}'", query, show);
            let query = if istv {
                format!("{} AND istv", query)
            } else {
                query
            };

            conn.query(&query, &[])?
                .iter()
                .map(|row| {
                    let show: String = row.get(0);
                    let title: String = row.get(1);
                    let link: String = row.get(2);
                    let rating: f64 = row.get(3);

                    println!("{} {} {} {}", show, title, link, rating);
                    show
                })
                .for_each(drop);
        }
        Ok(shows)
    }

    pub fn print_imdb_episodes(&self, show: &str, season: Option<i64>) -> Result<(), Error> {
        let conn = self.pool.get()?;
        let query = r#"
            SELECT a.show, b.title, a.season, a.episode,
                   cast(a.airdate as text),
                   cast(a.rating as double precision),
                   a.eptitle
            FROM imdb_episodes a
            JOIN imdb_ratings b ON a.show=b.show"#;
        let query = format!("{} WHERE a.show = '{}'", query, show);
        let query = if let Some(season) = season {
            format!("{} AND a.season = {}", query, season)
        } else {
            query
        };
        let query = format!("{} ORDER BY a.season, a.episode", query);

        for row in conn.query(&query, &[])?.iter() {
            let show: String = row.get(0);
            let title: String = row.get(1);
            let season: i32 = row.get(2);
            let episode: i32 = row.get(3);
            let airdate: String = row.get(4);
            let rating: f64 = row.get(5);
            let eptitle: String = row.get(6);

            println!(
                "{} {} {} {} {} {} {}",
                show, title, season, episode, airdate, rating, eptitle
            );
        }
        Ok(())
    }

    pub fn print_imdb_all_seasons(&self, show: &str) -> Result<(), Error> {
        let conn = self.pool.get()?;
        let query = r#"
            SELECT a.show, b.title, a.season, count(distinct a.episode)
            FROM imdb_episodes a
            JOIN imdb_ratings b ON a.show=b.show"#;
        let query = format!("{} WHERE a.show = '{}'", query, show);
        let query = format!("{} GROUP BY a.show, b.title, a.season", query);
        let query = format!("{} ORDER BY a.season", query);

        for row in conn.query(&query, &[])?.iter() {
            let show: String = row.get(0);
            let title: String = row.get(1);
            let season: i32 = row.get(2);
            let nepisodes: i64 = row.get(3);

            println!("{} {} {} {}", show, title, season, nepisodes);
        }
        Ok(())
    }
}
