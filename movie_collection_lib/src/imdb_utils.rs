use anyhow::Error;
use chrono::NaiveDate;
use futures::future::try_join_all;
use log::debug;
use reqwest::{Client, Url};
use select::{
    document::Document,
    predicate::{Class, Name},
};
use serde::Deserialize;
use stack_string::{format_sstr, StackString};
use std::{fmt, fmt::Write};

use crate::utils::{option_string_wrapper, ExponentialRetry};

#[derive(Default, Debug)]
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

#[derive(Debug, Default)]
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
            "{} {} {} {} {} {} {}",
            self.season,
            self.episode,
            option_string_wrapper(self.epurl.as_ref()),
            option_string_wrapper(self.eptitle.as_ref()),
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
    #[must_use]
    pub fn new() -> Self {
        Self {
            client: Client::new(),
        }
    }

    /// # Errors
    /// Returns error if `parse_imdb_rating` fails
    #[allow(clippy::needless_collect)]
    pub async fn parse_imdb(&self, title: &str) -> Result<Vec<ImdbTuple>, Error> {
        let endpoint = "http://www.imdb.com/find?";
        let url = Url::parse_with_params(endpoint, &[("s", "all"), ("q", title)])?;
        let body = self.get(&url).await?.text().await?;

        let tl_vec: Vec<(StackString, StackString)> = Document::from(body.as_str())
            .find(Class("result_text"))
            .flat_map(|tr| {
                tr.find(Name("a"))
                    .filter_map(|a| {
                        a.attr("href").and_then(|link| {
                            link.split('/').nth(2).and_then(|imdb_id| {
                                if imdb_id.starts_with("tt") {
                                    Some((tr.text().trim().into(), imdb_id.into()))
                                } else {
                                    None
                                }
                            })
                        })
                    })
                    .collect::<Vec<_>>()
            })
            .collect();

        let futures = tl_vec.into_iter().map(|(title, link)| async move {
            let r = self.parse_imdb_rating(&link).await?;
            let rating = r.rating.unwrap_or(-1.0);
            Ok(ImdbTuple {
                title,
                link,
                rating,
            })
        });
        try_join_all(futures).await
    }

    /// # Errors
    /// Returns error if `parse_imdb_rating_body` fails
    pub async fn parse_imdb_rating(&self, title: &str) -> Result<RatingOutput, Error> {
        if !title.starts_with("tt") {
            return Ok(RatingOutput::default());
        };

        let url = Url::parse("http://www.imdb.com/title/")?.join(title)?;
        debug!("{:?}", url);
        let body = self.get(&url).await?.text().await?;
        Self::parse_imdb_rating_body(&body)
    }

    fn parse_imdb_rating_body(body: &str) -> Result<RatingOutput, Error> {
        let document = Document::from(body);
        for item in document.find(Name("script")) {
            #[derive(Deserialize, Debug)]
            struct InnerRatingStruct {
                #[serde(rename = "ratingValue")]
                rating_value: f64,
                #[serde(rename = "ratingCount")]
                rating_count: u64,
            }
            #[derive(Deserialize, Debug)]
            struct RatingStruct {
                #[serde(rename = "aggregateRating")]
                aggregate_rating: InnerRatingStruct,
            }
            if item.attr("type") != Some("application/ld+json") {
                continue;
            }
            let rating_str = item.text();
            if !rating_str.contains("aggregateRating") {
                continue;
            }
            let rating: RatingStruct = serde_json::from_str(&rating_str)?;
            debug!("{}", rating_str);
            return Ok(RatingOutput {
                rating: Some(rating.aggregate_rating.rating_value),
                count: Some(rating.aggregate_rating.rating_count),
            });
        }
        Ok(RatingOutput::default())
    }

    /// # Errors
    /// Returns error if `parse_episodes_url` fails
    #[allow(clippy::needless_collect)]
    pub async fn parse_imdb_episode_list(
        &self,
        imdb_id: &str,
        season: Option<i32>,
    ) -> Result<Vec<ImdbEpisodeResult>, Error> {
        let endpoint = format_sstr!("http://m.imdb.com/title/{imdb_id}/episodes");
        let url = Url::parse(&endpoint)?;
        let body = self.get(&url).await?.text().await?;

        let ep_season_vec: Vec<(StackString, StackString)> = Document::from(body.as_str())
            .find(Name("a"))
            .filter_map(|a| {
                if let Some("season") = a.attr("class") {
                    let season_ = a.attr("season_number").unwrap_or("-1");
                    if let Some(link) = a.attr("href") {
                        let episodes_url =
                            format_sstr!("http://www.imdb.com/title/{imdb_id}/episodes/{link}");
                        Some((episodes_url, season_.into()))
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
        let episodes_url = Url::parse(episodes_url)?;
        let body = self.get(&episodes_url).await?.text().await?;

        let mut results = Vec::new();

        for div in Document::from(body.as_str()).find(Name("div")) {
            if let Some("info") = div.attr("class") {
                if let Some("episodes") = div.attr("itemprop") {
                    let mut result = ImdbEpisodeResult {
                        season,
                        ..ImdbEpisodeResult::default()
                    };
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
                let r = self.parse_imdb_rating(link).await?;
                result.rating = r.rating;
                result.nrating = r.count;
            }
            Ok(result)
        });

        try_join_all(futures).await
    }
}

#[cfg(test)]
mod tests {
    use anyhow::Error;
    use chrono::{NaiveDate, Utc};
    use log::debug;
    use stack_string::format_sstr;
    use std::fmt::Write;

    use crate::imdb_utils::{ImdbConnection, ImdbEpisodeResult, ImdbTuple};

    #[test]
    fn test_parse_imdb_rating_body() -> Result<(), Error> {
        let body = include_str!("../../tests/data/imdb_rating_body.html");
        let rating = ImdbConnection::parse_imdb_rating_body(body)?;
        println!("{:?}", rating);
        assert_eq!(rating.rating, Some(7.5));
        Ok(())
    }

    #[tokio::test]
    async fn test_parse_imdb_rating() -> Result<(), Error> {
        let conn = ImdbConnection::default();
        let rating = conn.parse_imdb_rating("tt14418068").await?;
        println!("{:?}", rating);
        assert!(rating.rating.is_some());
        Ok(())
    }

    #[test]
    fn test_imdb_tuple_display() -> Result<(), Error> {
        let t = ImdbTuple {
            title: "Test Title".into(),
            link: "https://example.com/link".into(),
            rating: 0.85,
        };
        assert_eq!(
            format_sstr!("{t}"),
            format_sstr!("Test Title https://example.com/link 0.85")
        );
        Ok(())
    }

    #[test]
    fn test_imdb_episode_result_display() -> Result<(), Error> {
        let today = Utc::now().date().naive_utc();
        let t = ImdbEpisodeResult {
            season: 2,
            episode: 3,
            epurl: Some("https://example.com/test".into()),
            eptitle: Some("test_title".into()),
            airdate: Some(today),
            rating: Some(4.5),
            nrating: Some(10_000),
        };
        assert_eq!(
            format_sstr!("{t}"),
            format_sstr!("2 3 https://example.com/test test_title {today} 4.5 10000")
        );
        Ok(())
    }

    #[tokio::test]
    async fn test_parse_imdb() -> Result<(), Error> {
        let conn = ImdbConnection::new();
        let results = conn.parse_imdb("the sopranos").await?;
        let top_result = &results[0];
        assert_eq!(&top_result.title, "The Sopranos (1999) (TV Series)");
        assert_eq!(&top_result.link, "tt0141842");
        debug!("results {results:#?}");
        assert!(results.len() > 1);
        Ok(())
    }

    #[tokio::test]
    async fn test_parse_imdb_episode_list() -> Result<(), Error> {
        let conn = ImdbConnection::new();
        let results = conn.parse_imdb_episode_list("tt0141842", Some(1)).await?;
        debug!("{results:#?}");
        let first = &results[0];
        assert_eq!(first.season, 1);
        assert_eq!(first.episode, 1);
        assert_eq!(first.epurl, Some("tt0705282".into()));
        assert_eq!(first.eptitle, Some("Pilot".into()));
        assert_eq!(first.airdate, Some(NaiveDate::from_ymd(1999, 1, 10)));
        assert_eq!(results.len(), 13);
        Ok(())
    }
}
