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
use reqwest::Url;
use reqwest::{Client, Response};
use select::document::Document;
use select::predicate::{Class, Name};
use std::collections::HashMap;
use std::fmt;
use std::io;
use std::io::Write;
use std::thread::sleep;
use std::time::Duration;

use crate::common::imdb_episodes::ImdbEpisodes;
use crate::common::imdb_ratings::ImdbRatings;
use crate::common::movie_collection::{MovieCollection, MovieCollectionDB};
use crate::common::utils::{map_result_vec, option_string_wrapper};

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

impl ImdbConnection {
    pub fn new() -> ImdbConnection {
        ImdbConnection {
            client: Client::new(),
        }
    }

    pub fn get(&self, url: &Url) -> Result<Response, Error> {
        let mut timeout: u64 = 1;
        loop {
            match self.client.get(url.clone()).send() {
                Ok(x) => return Ok(x),
                Err(e) => {
                    sleep(Duration::from_secs(timeout));
                    timeout *= 2;
                    if timeout >= 64 {
                        return Err(err_msg(e));
                    }
                }
            }
        }
    }

    pub fn parse_imdb(&self, title: &str) -> Result<Vec<ImdbTuple>, Error> {
        let endpoint = "http://www.imdb.com/find?";
        let url = Url::parse_with_params(endpoint, &[("s", "all"), ("q", title)])?;
        let body = self.get(&url)?.text()?;

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
                    let episode_url = "http://www.imdb.com/title";
                    let episodes_url = format!("{}/{}/episodes/{}", episode_url, imdb_id, link);
                    results
                        .extend_from_slice(&self.parse_episodes_url(&episodes_url, Some(season_))?);
                }
            }
        }
        Ok(results)
    }

    fn parse_episodes_url(
        &self,
        episodes_url: &str,
        season: Option<i32>,
    ) -> Result<Vec<ImdbEpisodeResult>, Error> {
        let episodes_url = Url::parse(&episodes_url)?;
        let body = self.get(&episodes_url)?.text()?;
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

pub fn parse_imdb_worker(
    show: &str,
    tv: bool,
    imdb_link: Option<String>,
    all_seasons: bool,
    season: Option<i32>,
    do_update: bool,
    update_database: bool,
) -> Result<(), Error> {
    let mc = MovieCollectionDB::new();

    let shows: Vec<_> = if let Some(ilink) = &imdb_link {
        mc.print_imdb_shows(show, tv)?
            .into_iter()
            .filter_map(|s| match &s.link {
                Some(l) if l == ilink => Some((l.clone(), s)),
                _ => None,
            })
            .collect()
    } else {
        mc.print_imdb_shows(show, tv)?
            .into_iter()
            .filter_map(|s| match &s.link {
                Some(l) => Some((l.clone(), s)),
                None => None,
            })
            .collect()
    };

    for (_, s) in &shows {
        writeln!(io::stdout().lock(), "{}", s)?;
    }

    let shows: HashMap<String, _> = shows.into_iter().collect();

    let episodes: Option<Vec<_>> = if tv {
        if all_seasons {
            for s in mc.print_imdb_all_seasons(show)? {
                writeln!(io::stdout().lock(), "{}", s)?;
            }
            None
        } else {
            let r = mc
                .print_imdb_episodes(show, season)?
                .into_iter()
                .map(|e| ((e.season, e.episode), e))
                .collect();
            Some(r)
        }
    } else {
        None
    };

    if let Some(v) = episodes.as_ref() {
        for ((_, _), e) in v {
            writeln!(io::stdout().lock(), "{}", e)?;
        }
    }

    let episodes: Option<HashMap<(i32, i32), _>> = episodes.map(|v| v.into_iter().collect());

    if do_update {
        let imdb_conn = ImdbConnection::new();
        let results = imdb_conn.parse_imdb(&show.replace("_", " "))?;
        let results = if let Some(ilink) = &imdb_link {
            results.into_iter().filter(|r| &r.link == ilink).collect()
        } else {
            results
        };

        let link = if let Some(link) = imdb_link {
            Some(link)
        } else if let Some(result) = results.get(0) {
            Some(result.link.clone())
        } else {
            None
        };

        if !tv {
            if update_database {
                if let Some(result) = results.get(0) {
                    match shows.get(&result.link) {
                        Some(s) => {
                            if (result.rating - s.rating.unwrap_or(-1.0)).abs() > 0.1 {
                                let mut new = s.clone();
                                new.title = Some(result.title.clone());
                                new.rating = Some(result.rating);
                                new.update_show(&mc.pool)?;
                                writeln!(
                                    io::stdout().lock(),
                                    "exists {} {} {}",
                                    show,
                                    s,
                                    result.rating
                                )?;
                            }
                        }
                        None => {
                            writeln!(io::stdout().lock(), "not exists {} {}", show, result)?;
                            let istv = result.title.contains("TV Series")
                                || result.title.contains("TV Mini-Series");

                            ImdbRatings {
                                show: show.to_string(),
                                title: Some(result.title.clone()),
                                link: Some(result.link.clone()),
                                rating: Some(result.rating),
                                istv: Some(istv),
                                ..Default::default()
                            }
                            .insert_show(&mc.pool)?;
                        }
                    }
                }
            }

            for result in &results {
                writeln!(io::stdout().lock(), "{}", result)?;
            }
        } else if let Some(link) = link {
            writeln!(io::stdout().lock(), "Using {}", link)?;
            if let Some(result) = shows.get(&link) {
                for episode in imdb_conn.parse_imdb_episode_list(&link, season)? {
                    writeln!(io::stdout().lock(), "{} {}", result, episode)?;
                    if update_database {
                        let key = (episode.season, episode.episode);
                        if let Some(episodes) = &episodes {
                            match episodes.get(&key) {
                                Some(e) => {
                                    if (e.rating - episode.rating.unwrap_or(-1.0)).abs() > 0.1 {
                                        writeln!(
                                            io::stdout().lock(),
                                            "exists {} {} {}",
                                            result,
                                            episode,
                                            e.rating
                                        )?;
                                        let mut new = e.clone();
                                        new.eptitle =
                                            episode.eptitle.unwrap_or_else(|| "".to_string());
                                        new.rating = episode.rating.unwrap_or(-1.0);
                                        new.update_episode(&mc.pool)?;
                                    }
                                }
                                None => {
                                    writeln!(
                                        io::stdout().lock(),
                                        "not exists {} {}",
                                        result,
                                        episode
                                    )?;
                                    ImdbEpisodes {
                                        show: show.to_string(),
                                        title: result
                                            .title
                                            .clone()
                                            .unwrap_or_else(|| "".to_string()),
                                        season: episode.season,
                                        episode: episode.episode,
                                        airdate: episode
                                            .airdate
                                            .unwrap_or_else(|| NaiveDate::from_ymd(1970, 1, 1)),
                                        rating: episode.rating.unwrap_or(-1.0),
                                        eptitle: episode.eptitle.unwrap_or_else(|| "".to_string()),
                                        epurl: episode.epurl.unwrap_or_else(|| "".to_string()),
                                    }
                                    .insert_episode(&mc.pool)?;
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    Ok(())
}
