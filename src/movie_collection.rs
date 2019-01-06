extern crate chrono;
extern crate failure;
extern crate r2d2;
extern crate r2d2_postgres;
extern crate rayon;
extern crate reqwest;
extern crate select;

use chrono::NaiveDate;
use failure::Error;
use r2d2::Pool;
use r2d2_postgres::{PostgresConnectionManager, TlsMode};
use rayon::prelude::*;
use reqwest::Url;
use select::document::Document;
use select::predicate::{Class, Name};
use std::fmt;

use crate::config::Config;
use crate::utils::map_result_vec;

pub type PgPool = Pool<PostgresConnectionManager>;

#[derive(Debug)]
pub struct MovieCollection {
    pub config: Config,
    pub pool: PgPool,
}

#[derive(Default, Clone)]
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
            self.title.clone().unwrap_or_else(|| "".to_string()),
            self.link.clone().unwrap_or_else(|| "".to_string()),
            self.rating.unwrap_or(-1.0),
            self.istv.unwrap_or(false),
            self.source.clone().unwrap_or_else(|| "".to_string()),
        )
    }
}

#[derive(Clone)]
pub struct ImdbEpisodes {
    pub show: String,
    pub title: String,
    pub season: i32,
    pub episode: i32,
    pub airdate: NaiveDate,
    pub rating: f64,
    pub eptitle: String,
}

impl fmt::Display for ImdbEpisodes {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "{} {} {} {} {} {} {} ",
            self.show,
            self.title,
            self.season,
            self.episode,
            self.airdate,
            self.rating,
            self.eptitle,
        )
    }
}

impl Default for MovieCollection {
    fn default() -> MovieCollection {
        MovieCollection::new()
    }
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

    pub fn insert_imdb_show(&self, input: &ImdbRatings) -> Result<(), Error> {
        let query = r#"
            INSERT INTO imdb_ratings
            (show, title, link, rating, istv, source)
            VALUES
            ($1, $2, $3, $4, $5, $6)
        "#;

        let conn = self.pool.get()?;
        conn.execute(
            query,
            &[
                &input.show,
                &input.title,
                &input.link,
                &input.rating,
                &input.istv,
                &input.source,
            ],
        )?;
        Ok(())
    }

    pub fn update_imdb_show(&self, input: &ImdbRatings) -> Result<(), Error> {
        let query = r#"
            UPDATE imdb_ratings SET rating=$1,title=$2 WHERE show=$3
        "#;

        let conn = self.pool.get()?;
        conn.execute(query, &[&input.rating, &input.title, &input.show])?;
        Ok(())
    }

    pub fn insert_imdb_episode(&self, input: &ImdbEpisodes) -> Result<(), Error> {
        let query = r#"
            INSERT INTO imdb_episodes
            (show, season, episode, airdate, rating, eptitle)
            VALUES
            ($1, $2, $3, $4, RATING, $5)
        "#;
        let query = query.replace("RATING", &input.rating.to_string());

        let conn = self.pool.get()?;
        conn.execute(
            &query,
            &[
                &input.show,
                &input.season,
                &input.episode,
                &input.airdate,
                &input.eptitle,
            ],
        )?;
        Ok(())
    }

    pub fn update_imdb_episodes(&self, input: &ImdbEpisodes) -> Result<(), Error> {
        let query = r#"
            UPDATE imdb_episodes SET rating=RATING,eptitle=$1 WHERE show=$2 AND season=$3 AND episode=$4
        "#;
        let query = query.replace("RATING", &input.rating.to_string());

        let conn = self.pool.get()?;
        conn.execute(
            &query,
            &[&input.eptitle, &input.show, &input.season, &input.episode],
        )?;
        Ok(())
    }

    pub fn print_imdb_shows(&self, show: &str, istv: bool) -> Result<Vec<ImdbRatings>, Error> {
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

        let shows: Vec<Result<_, Error>> = shows
            .iter()
            .map(|show| {
                let query = r#"
                    SELECT index, show, title, link, rating
                    FROM imdb_ratings
                    WHERE link is not null AND rating is not null"#;
                let query = format!("{} AND show = '{}'", query, show);
                let query = if istv {
                    format!("{} AND istv", query)
                } else {
                    query
                };

                let results: Vec<_> = conn
                    .query(&query, &[])?
                    .iter()
                    .map(|row| {
                        let index: i32 = row.get(0);
                        let show: String = row.get(1);
                        let title: String = row.get(2);
                        let link: String = row.get(3);
                        let rating: f64 = row.get(4);

                        println!("{} {} {} {}", show, title, link, rating);
                        ImdbRatings {
                            index,
                            show,
                            title: Some(title),
                            link: Some(link),
                            rating: Some(rating),
                            ..Default::default()
                        }
                    })
                    .collect();
                Ok(results)
            })
            .collect();
        let results: Vec<_> = map_result_vec(shows)?.into_iter().flatten().collect();
        Ok(results)
    }

    pub fn print_imdb_episodes(
        &self,
        show: &str,
        season: Option<i32>,
    ) -> Result<Vec<ImdbEpisodes>, Error> {
        let conn = self.pool.get()?;
        let query = r#"
            SELECT a.show, b.title, a.season, a.episode,
                   a.airdate,
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

        let result: Vec<_> = conn
            .query(&query, &[])?
            .iter()
            .map(|row| {
                let show: String = row.get(0);
                let title: String = row.get(1);
                let season: i32 = row.get(2);
                let episode: i32 = row.get(3);
                let airdate: NaiveDate = row.get(4);
                let rating: f64 = row.get(5);
                let eptitle: String = row.get(6);

                println!(
                    "{} {} {} {} {} {} {}",
                    show, title, season, episode, airdate, rating, eptitle
                );
                ImdbEpisodes {
                    show,
                    title,
                    season,
                    episode,
                    airdate,
                    rating,
                    eptitle,
                }
            })
            .collect();
        Ok(result)
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

pub struct ImdbTuple {
    pub title: String,
    pub link: String,
    pub rating: f64,
}

impl fmt::Display for ImdbTuple {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{} {} {}", self.title, self.link, self.rating)
    }
}

pub fn parse_imdb(title: &str) -> Result<Vec<ImdbTuple>, Error> {
    let endpoint = "http://www.imdb.com/find?";
    let url = Url::parse_with_params(endpoint, &[("s", "all"), ("q", title)])?;
    let body = reqwest::get(url.clone())?.text()?;

    let document = Document::from(body.as_str());

    let mut shows = Vec::new();

    for tr in document.find(Class("result_text")) {
        let title = tr.text().trim().to_string();
        for a in tr.find(Name("a")) {
            if let Some(link) = a.attr("href") {
                if let Some(link) = link.split('/').nth(2) {
                    if link.starts_with("tt") {
                        shows.push((title.clone(), link));
                    }
                }
            } else {
            };
        }
    }

    let results: Vec<Result<_, Error>> = shows
        .into_par_iter()
        .map(|(t, l)| {
            let r = parse_imdb_rating(l)?;
            Ok(ImdbTuple {
                title: t.to_string(),
                link: l.to_string(),
                rating: r.rating.unwrap_or(-1.0),
            })
        })
        .collect();

    let shows: Vec<_> = map_result_vec(results)?;

    Ok(shows)
}

#[derive(Debug)]
pub struct RatingOutput {
    pub rating: Option<f64>,
    pub count: Option<u64>,
}

pub fn parse_imdb_rating(title: &str) -> Result<RatingOutput, Error> {
    let mut output = RatingOutput {
        rating: None,
        count: None,
    };

    if !title.starts_with("tt") {
        return Ok(output);
    };

    let url = Url::parse("http://www.imdb.com/title/")?.join(title)?;
    let body = reqwest::get(url)?.text()?;
    let document = Document::from(body.as_str());

    for span in document.find(Name("span")) {
        if let Some("ratingValue") = span.attr("itemprop") {
            output.rating = Some(span.text().parse()?);
        }
        if let Some("ratingCount") = span.attr("itemprop") {
            output.count = Some(span.text().replace(",", "").parse()?);
        }
    }
    Ok(output)
}

#[derive(Default, Clone)]
pub struct ImdbEpisodeResult {
    pub season: i32,
    pub episode: i32,
    pub epurl: Option<String>,
    pub eptitle: Option<String>,
    pub airdate: Option<NaiveDate>,
    pub rating: Option<f64>,
    pub nrating: Option<u64>,
}

impl fmt::Display for ImdbEpisodeResult {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "{} {} {} {} {} {} {} ",
            self.season,
            self.episode,
            self.epurl.clone().unwrap_or_else(|| "".to_string()),
            self.eptitle.clone().unwrap_or_else(|| "".to_string()),
            self.airdate
                .unwrap_or_else(|| NaiveDate::from_ymd(1970, 1, 1)),
            self.rating.unwrap_or(-1.0),
            self.nrating.unwrap_or(0),
        )
    }
}

pub fn parse_imdb_episode_list(
    imdb_id: &str,
    season: Option<i32>,
) -> Result<Vec<ImdbEpisodeResult>, Error> {
    let endpoint: String = format!("http://m.imdb.com/title/{}/episodes", imdb_id);
    let url = Url::parse(&endpoint)?;
    let body = reqwest::get(url)?.text()?;
    let document = Document::from(body.as_str());

    let mut results = Vec::new();

    for a in document.find(Name("a")) {
        if let Some("season") = a.attr("class") {
            let season_: i32 = a.attr("season_number").unwrap_or("-1").parse()?;
            if let Some(link) = a.attr("href") {
                if let Some(s) = season {
                    if s != season_ {
                        continue;
                    }
                }
                let episodes_url =
                    format!("http://www.imdb.com/title/{}/episodes/{}", imdb_id, link);

                results.extend_from_slice(&parse_episodes_url(&episodes_url, season)?);
            }
        }
    }
    Ok(results)
}

fn parse_episodes_url(
    episodes_url: &str,
    season: Option<i32>,
) -> Result<Vec<ImdbEpisodeResult>, Error> {
    let episodes_url = Url::parse(&episodes_url)?;
    let body = reqwest::get(episodes_url)?.text()?;
    let document = Document::from(body.as_str());

    let mut results = Vec::new();

    for div in document.find(Name("div")) {
        if let Some("info") = div.attr("class") {
            if let Some("episodes") = div.attr("itemprop") {
                let mut result = ImdbEpisodeResult::default();
                if let Some(s) = season {
                    result.season = s;
                }
                for meta in div.find(Name("meta")) {
                    if let Some("episodeNumber") = meta.attr("itemprop") {
                        if let Some(episode) = meta.attr("content") {
                            result.episode = episode.parse()?;
                        }
                    }
                }
                for div_ in div.find(Name("div")) {
                    if let Some("airdate") = div_.attr("class") {
                        if let Ok(date) = NaiveDate::parse_from_str(div_.text().trim(), "%d %b. %Y")
                        {
                            result.airdate = Some(date);
                        }
                    }
                }
                for a_ in div.find(Name("a")) {
                    if result.epurl.is_some() {
                        continue;
                    };
                    if let Some(epi_url) = a_.attr("href") {
                        if let Some(link) = epi_url.split('/').nth(2) {
                            result.epurl = Some(link.to_string());
                            result.eptitle = Some(a_.text().trim().to_string());
                            let r = parse_imdb_rating(link)?;
                            result.rating = r.rating;
                            result.nrating = r.count;
                        }
                    }
                }
                results.push(result);
            }
        }
    }
    Ok(results)
}

/*
def parse_imdb_episode_list(imdb_id='tt3230854', season=None, proxy=False):
                                rating, nrating = parse_imdb_rating(epi_url, proxy=proxy)
                            else:
                                print('epi_url', epi_url)
                                epi_url = ''
                    if season != -1 and season_ >= 0 and episode >= 0 and airdate:
                        yield season_, episode, airdate, rating, nrating, epi_title, epi_url
            if season == -1:
                yield season_, -1, None, number_of_episodes[season_], -1, 'season', None

*/
