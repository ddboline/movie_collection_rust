use anyhow::{format_err, Error};
use futures::future::try_join_all;
use log::debug;
use reqwest::{Client, Url};
use select::{
    document::Document,
    predicate::{Class, Name},
};
use serde::Deserialize;
use stack_string::{format_sstr, StackString};
use std::{convert::TryFrom, fmt, fmt::Write};
use time::{macros::date, Date, Month};

use crate::utils::{option_string_wrapper, ExponentialRetry};

#[derive(Clone, Copy, Debug)]
enum ImdbType {
    TvSeries,
    MiniSeries,
    Movie,
    TvMovie,
}

impl ImdbType {
    fn from_str(s: &str) -> Option<ImdbType> {
        match s {
            "movie" => Some(Self::Movie),
            "tvSeries" => Some(Self::TvSeries),
            "tvMiniSeries" => Some(Self::MiniSeries),
            "TvMovie" => Some(Self::TvMovie),
            _ => None,
        }
    }

    fn to_str(self) -> &'static str {
        match self {
            Self::TvSeries => "TV Series",
            Self::MiniSeries => "TV Mini-Series",
            Self::Movie => "Movie",
            Self::TvMovie => "TV Movie",
        }
    }
}

impl fmt::Display for ImdbType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_str())
    }
}

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
    pub airdate: Option<Date>,
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
            self.airdate.unwrap_or_else(|| date!(1970 - 01 - 01)),
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
            .find(Class("ipc-page-content-container"))
            .flat_map(|tr| {
                tr.find(Name("a"))
                    .filter_map(|a| {
                        a.attr("href").and_then(|link| {
                            link.split('/').nth(2).and_then(|imdb_id| {
                                if imdb_id.starts_with("tt") {
                                    Some((a.text().trim().into(), imdb_id.into()))
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
    /// Returns error if api calls fail
    pub async fn get_suggestions(&self, title: &str) -> Result<Vec<ImdbTuple>, Error> {
        #[derive(Deserialize)]
        struct ImdbSuggestion {
            id: StackString,
            l: StackString,
            qid: Option<StackString>,
            y: Option<i32>,
        }

        #[derive(Deserialize)]
        struct ImdbSuggestions {
            d: Vec<ImdbSuggestion>,
        }

        let url = format_sstr!("https://v3.sg.media-imdb.com/suggestion/x/{title}.json");
        let url: Url = url.parse()?;

        let suggestions: ImdbSuggestions = self.get(&url).await?.json().await?;
        let futures = suggestions.d.into_iter().map(|s| async move {
            let mut title = s.l;
            let year = s.y;
            let imdb_type = s.qid.and_then(|s| ImdbType::from_str(&s));
            let r = self.parse_imdb_rating(&s.id).await?;
            let rating = r.rating.unwrap_or(-1.0);
            if let Some(year) = year {
                write!(&mut title, " ({year})").unwrap();
            }
            if let Some(imdb_type) = imdb_type {
                write!(&mut title, " ({imdb_type})").unwrap();
            }
            Ok(ImdbTuple {
                title,
                link: s.id,
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
    ) -> Result<(Vec<usize>, Vec<ImdbEpisodeResult>), Error> {
        let endpoint = if let Some(season) = season {
            format_sstr!("http://m.imdb.com/title/{imdb_id}/episodes?season={season}")
        } else {
            format_sstr!("http://m.imdb.com/title/{imdb_id}/episodes")
        };
        let url = Url::parse(&endpoint)?;
        let body = self.get(&url).await?.text().await?;
        Self::parse_imdb_episode_list_body(&body)
    }

    /// # Errors
    /// Returns error if `parse_episodes_url` fails
    #[allow(clippy::needless_collect)]
    fn parse_imdb_episode_list_body(
        body: &str,
    ) -> Result<(Vec<usize>, Vec<ImdbEpisodeResult>), Error> {
        #[derive(Deserialize, Debug)]
        struct _Season {
            value: StackString,
        }

        #[derive(Deserialize, Debug)]
        struct _ReleaseDate {
            year: i32,
            month: Option<u8>,
            day: Option<u8>,
        }

        #[derive(Deserialize, Debug)]
        struct _EpisodeItem {
            id: StackString,
            season: StackString,
            episode: StackString,
            #[serde(alias = "titleText")]
            title_text: StackString,
            #[serde(alias = "releaseDate")]
            release_date: Option<_ReleaseDate>,
            #[serde(alias = "aggregateRating")]
            aggregate_rating: Option<f64>,
            #[serde(alias = "voteCount")]
            vote_count: Option<u64>,
        }

        #[derive(Deserialize, Debug)]
        struct _Episodes {
            items: Vec<_EpisodeItem>,
        }

        #[derive(Deserialize, Debug)]
        struct _Section {
            seasons: Vec<_Season>,
            episodes: _Episodes,
        }

        #[derive(Deserialize, Debug)]
        struct _ContentData {
            section: _Section,
        }

        #[derive(Deserialize, Debug)]
        struct _PageProps {
            #[serde(alias = "contentData")]
            content_data: _ContentData,
        }

        #[derive(Deserialize, Debug)]
        struct _Props {
            #[serde(alias = "pageProps")]
            page_props: _PageProps,
        }

        #[derive(Deserialize, Debug)]
        struct _ImdbDataObject {
            props: _Props,
        }

        for script in Document::from(body).find(Name("script")) {
            if let Some("__NEXT_DATA__") = script.attr("id") {
                let body = script.text();
                if body.contains("props") {
                    match serde_json::from_str::<_ImdbDataObject>(&body) {
                        Ok(imdb_data_object) => {
                            let seasons: Vec<usize> = imdb_data_object
                                .props
                                .page_props
                                .content_data
                                .section
                                .seasons
                                .iter()
                                .filter_map(|s| s.value.parse().ok())
                                .collect();
                            let episodes: Vec<ImdbEpisodeResult> = imdb_data_object
                                .props
                                .page_props
                                .content_data
                                .section
                                .episodes
                                .items
                                .into_iter()
                                .filter_map(|e| {
                                    let airdate = e.release_date.and_then(|rd| {
                                        let m = rd.month?;
                                        let month = Month::try_from(m).ok()?;
                                        let d = rd.day?;
                                        Date::from_calendar_date(rd.year, month, d).ok()
                                    });
                                    Some(ImdbEpisodeResult {
                                        season: e.season.parse().unwrap_or(1),
                                        episode: e.episode.parse().ok()?,
                                        epurl: Some(e.id),
                                        eptitle: Some(e.title_text),
                                        airdate,
                                        rating: e.aggregate_rating,
                                        nrating: e.vote_count,
                                    })
                                })
                                .collect();
                            return Ok((seasons, episodes));
                        }
                        Err(e) => println!("error {e:?}"),
                    }
                }
            }
        }
        Err(format_err!("Parsing failed"))
    }
}

#[cfg(test)]
mod tests {
    use anyhow::Error;
    use log::debug;
    use stack_string::format_sstr;
    use time::{macros::date, OffsetDateTime};

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
        let today = OffsetDateTime::now_utc().date();
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
    #[ignore]
    async fn test_parse_imdb() -> Result<(), Error> {
        let conn = ImdbConnection::new();
        let results = conn.parse_imdb("the sopranos").await?;
        let top_result = &results[0];
        assert_eq!(&top_result.title, "The Sopranos");
        assert_eq!(&top_result.link, "tt0141842");
        debug!("results {results:#?}");
        assert!(results.len() > 1);
        Ok(())
    }

    #[tokio::test]
    #[ignore]
    async fn test_get_suggestions() -> Result<(), Error> {
        let conn = ImdbConnection::new();
        let results = conn.get_suggestions("the_sopranos").await?;
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
        let (_, results) = conn.parse_imdb_episode_list("tt0141842", Some(1)).await?;
        debug!("{results:#?}");
        let first = &results[0];
        assert_eq!(first.season, 1);
        assert_eq!(first.episode, 1);
        assert_eq!(first.epurl, Some("tt0705282".into()));
        assert_eq!(first.eptitle, Some("Pilot".into()));
        assert_eq!(first.airdate, Some(date!(1999 - 01 - 10)));
        assert_eq!(results.len(), 13);
        Ok(())
    }

    #[test]
    fn test_parse_imdb_episode_list_body() -> Result<(), Error> {
        let body = include_str!("../../tests/data/imdb_season_episode_body.html");
        let (seasons, episodes) = ImdbConnection::parse_imdb_episode_list_body(body)?;
        assert_eq!(seasons.len(), 14);
        assert_eq!(episodes[0].season, 2);
        assert_eq!(episodes[0].episode, 1);
        assert_eq!(episodes[0].epurl, Some(format_sstr!("tt1824276")));
        Ok(())
    }

    #[test]
    fn test_parse_american_horror() -> Result<(), Error> {
        let body = include_str!("../../tests/data/american_horror.html");
        let (seasons, episodes) = ImdbConnection::parse_imdb_episode_list_body(body)?;
        assert_eq!(seasons.len(), 12);
        assert_eq!(episodes.len(), 6);
        assert_eq!(episodes[0].epurl, Some(format_sstr!("tt11578362")));
        assert_eq!(episodes[0].eptitle, Some(format_sstr!("Multiply Thy Pain")));
        Ok(())
    }
}
