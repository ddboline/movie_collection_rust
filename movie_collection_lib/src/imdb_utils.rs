use anyhow::Error;
use chrono::NaiveDate;
use futures::future::try_join_all;
use reqwest::{Client, Url};
use select::{
    document::Document,
    predicate::{Class, Name},
};
use stack_string::StackString;
use std::fmt;

use crate::utils::{option_string_wrapper, ExponentialRetry};

#[derive(Default)]
pub struct ImdbTuple {
    pub title: StackString,
    pub link: StackString,
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
    pub epurl: Option<StackString>,
    pub eptitle: Option<StackString>,
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
    fn default() -> Self {
        Self::new()
    }
}

impl ExponentialRetry for ImdbConnection {
    fn get_client(&self) -> &Client {
        &self.client
    }
}

impl ImdbConnection {
    pub fn new() -> Self {
        Self {
            client: Client::new(),
        }
    }

    pub async fn parse_imdb(&self, title: &str) -> Result<Vec<ImdbTuple>, Error> {
        let endpoint = "http://www.imdb.com/find?";
        let url = Url::parse_with_params(endpoint, &[("s", "all"), ("q", title)])?;
        let body = self.get(&url).await?.text().await?;

        let tl_vec: Vec<_> = Document::from(body.as_str())
            .find(Class("result_text"))
            .flat_map(|tr| {
                tr.find(Name("a"))
                    .filter_map(|a| {
                        if let Some(link) = a.attr("href") {
                            if let Some(link) = link.split('/').nth(2) {
                                if link.starts_with("tt") {
                                    Some((tr.text().trim().to_string(), link.to_string()))
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

        let futures = tl_vec.into_iter().map(|(t, l)| async move {
            let r = self.parse_imdb_rating(&l).await?;
            Ok(ImdbTuple {
                title: t.into(),
                link: l.into(),
                rating: r.rating.unwrap_or(-1.0),
            })
        });
        try_join_all(futures).await
    }

    pub async fn parse_imdb_rating(&self, title: &str) -> Result<RatingOutput, Error> {
        let mut output = RatingOutput {
            rating: None,
            count: None,
        };

        if !title.starts_with("tt") {
            return Ok(output);
        };

        let url = Url::parse("http://www.imdb.com/title/")?.join(title)?;
        let body = self.get(&url).await?.text().await?;

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

    pub async fn parse_imdb_episode_list(
        &self,
        imdb_id: &str,
        season: Option<i32>,
    ) -> Result<Vec<ImdbEpisodeResult>, Error> {
        let endpoint: String = format!("http://m.imdb.com/title/{}/episodes", imdb_id);
        let url = Url::parse(&endpoint)?;
        let body = self.get(&url).await?.text().await?;

        let ep_season_vec: Vec<_> = Document::from(body.as_str())
            .find(Name("a"))
            .filter_map(|a| {
                if let Some("season") = a.attr("class") {
                    let season_ = a.attr("season_number").unwrap_or("-1");
                    if let Some(link) = a.attr("href") {
                        let episodes_url = format!(
                            "{}/{}/episodes/{}",
                            "http://www.imdb.com/title", imdb_id, link
                        );
                        Some((episodes_url, season_.to_string()))
                    } else {
                        None
                    }
                } else {
                    None
                }
            })
            .collect();

        let futures = ep_season_vec
            .into_iter()
            .map(|(episodes_url, season_str)| async move {
                let season_: i32 = season_str.parse()?;
                if let Some(s) = season {
                    if s != season_ {
                        return Ok(Vec::new());
                    }
                }
                self.parse_episodes_url(&episodes_url, season_).await
            });

        Ok(try_join_all(futures).await?.into_iter().flatten().collect())
    }

    async fn parse_episodes_url(
        &self,
        episodes_url: &str,
        season: i32,
    ) -> Result<Vec<ImdbEpisodeResult>, Error> {
        let episodes_url = Url::parse(&episodes_url)?;
        let body = self.get(&episodes_url).await?.text().await?;

        let mut results = Vec::new();

        for div in Document::from(body.as_str()).find(Name("div")) {
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
                            } else if let Ok(date) =
                                NaiveDate::parse_from_str(div_.text().trim(), "%d %b %Y")
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
                                result.epurl = Some(link.into());
                                result.eptitle = Some(a_.text().trim().into());
                            }
                        }
                    }
                    results.push(result);
                }
            }
        }
        let results = results;

        let futures = results.into_iter().map(|mut result| async {
            if let Some(link) = result.epurl.as_ref() {
                let r = self.parse_imdb_rating(&link).await?;
                result.rating = r.rating;
                result.nrating = r.count;
            }
            Ok(result)
        });

        try_join_all(futures).await
    }
}
