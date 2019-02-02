extern crate chrono;
extern crate failure;
extern crate r2d2;
extern crate r2d2_postgres;
extern crate rayon;
extern crate reqwest;
extern crate select;

use chrono::NaiveDate;
use failure::{err_msg, Error};
use rayon::prelude::*;
use reqwest::{Client, Url};
use select::document::Document;
use select::predicate::{Class, Name};
use std::fmt;

use crate::common::utils::{map_result_vec, option_string_wrapper, ExponentialRetry};

#[derive(Default)]
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

#[derive(Debug)]
pub struct RatingOutput {
    pub rating: Option<f64>,
    pub count: Option<u64>,
}

#[derive(Default, Clone, Debug, PartialEq)]
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
            option_string_wrapper(&self.epurl),
            option_string_wrapper(&self.eptitle),
            self.airdate
                .unwrap_or_else(|| NaiveDate::from_ymd(1970, 1, 1)),
            self.rating.unwrap_or(-1.0),
            self.nrating.unwrap_or(0),
        )
    }
}

pub struct ImdbConnection {
    client: Client,
}

impl Default for ImdbConnection {
    fn default() -> ImdbConnection {
        ImdbConnection::new()
    }
}

impl ExponentialRetry for ImdbConnection {
    fn get_client(&self) -> &Client {
        &self.client
    }
}

impl ImdbConnection {
    pub fn new() -> ImdbConnection {
        ImdbConnection {
            client: Client::new(),
        }
    }

    pub fn parse_imdb(&self, title: &str) -> Result<Vec<ImdbTuple>, Error> {
        let endpoint = "http://www.imdb.com/find?";
        let url = Url::parse_with_params(endpoint, &[("s", "all"), ("q", title)])?;
        let body = self.get(&url)?.text()?;

        let document = Document::from(body.as_str());

        let shows: Vec<_> = document
            .find(Class("result_text"))
            .flat_map(|tr| {
                let title = tr.text().trim().to_string();
                tr.find(Name("a"))
                    .filter_map(|a| {
                        if let Some(link) = a.attr("href") {
                            if let Some(link) = link.split('/').nth(2) {
                                if link.starts_with("tt") {
                                    Some((title.clone(), link))
                                } else {
                                    None
                                }
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    })
                    .collect::<Vec<_>>()
            })
            .collect();

        let results: Vec<Result<_, Error>> = shows
            .into_par_iter()
            .map(|(t, l)| {
                let r = self.parse_imdb_rating(l)?;
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

    pub fn parse_imdb_rating(&self, title: &str) -> Result<RatingOutput, Error> {
        let mut output = RatingOutput {
            rating: None,
            count: None,
        };

        if !title.starts_with("tt") {
            return Ok(output);
        };

        let url = Url::parse("http://www.imdb.com/title/")?.join(title)?;
        let body = self.get(&url)?.text()?;
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

    pub fn parse_imdb_episode_list(
        &self,
        imdb_id: &str,
        season: Option<i32>,
    ) -> Result<Vec<ImdbEpisodeResult>, Error> {
        let endpoint: String = format!("http://m.imdb.com/title/{}/episodes", imdb_id);
        let url = Url::parse(&endpoint)?;
        let body = self.get(&url)?.text()?;
        let document = Document::from(body.as_str());

        let results: Vec<_> = document
            .find(Name("a"))
            .filter_map(|a| {
                if let Some("season") = a.attr("class") {
                    let season_ = a.attr("season_number").unwrap_or("-1");
                    if let Some(link) = a.attr("href") {
                        let episode_url = "http://www.imdb.com/title";
                        let episodes_url = format!("{}/{}/episodes/{}", episode_url, imdb_id, link);
                        Some((episodes_url, season_))
                    } else {
                        None
                    }
                } else {
                    None
                }
            })
            .collect();

        let results: Vec<_> = results
            .into_par_iter()
            .flat_map(|(episodes_url, season_str)| {
                let season_: i32 = match season_str.parse() {
                    Ok(s) => s,
                    Err(e) => return vec![Err(err_msg(e))],
                };
                if let Some(s) = season {
                    if s != season_ {
                        return Vec::new();
                    }
                }
                let results: Vec<Result<_, Error>> =
                    match self.parse_episodes_url(&episodes_url, season_) {
                        Ok(v) => v.into_iter().map(Ok).collect(),
                        Err(e) => vec![Err(err_msg(e))],
                    };
                results
            })
            .collect();
        let results: Vec<_> = map_result_vec(results)?;
        Ok(results)
    }

    fn parse_episodes_url(
        &self,
        episodes_url: &str,
        season: i32,
    ) -> Result<Vec<ImdbEpisodeResult>, Error> {
        let episodes_url = Url::parse(&episodes_url)?;
        let body = self.get(&episodes_url)?.text()?;
        let document = Document::from(body.as_str());

        let mut results = Vec::new();

        for div in document.find(Name("div")) {
            if let Some("info") = div.attr("class") {
                if let Some("episodes") = div.attr("itemprop") {
                    let mut result = ImdbEpisodeResult::default();
                    result.season = season;
                    for meta in div.find(Name("meta")) {
                        if let Some("episodeNumber") = meta.attr("itemprop") {
                            if let Some(episode) = meta.attr("content") {
                                result.episode = episode.parse()?;
                            }
                        }
                    }
                    for div_ in div.find(Name("div")) {
                        if let Some("airdate") = div_.attr("class") {
                            if let Ok(date) =
                                NaiveDate::parse_from_str(div_.text().trim(), "%d %b. %Y")
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
                                let r = self.parse_imdb_rating(link)?;
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
}
