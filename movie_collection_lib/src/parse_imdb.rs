use anyhow::Error;
use clap::Parser;
use itertools::Itertools;
use stack_string::{format_sstr, StackString};
use std::collections::HashMap;
use stdout_channel::StdoutChannel;
use time::macros::date;

use crate::{
    config::Config, imdb_episodes::ImdbEpisodes, imdb_ratings::ImdbRatings,
    imdb_utils::ImdbConnection, movie_collection::MovieCollection, pgpool::PgPool,
    trakt_utils::WatchListMap,
};

#[derive(Parser, Default, Debug, Clone)]
/// Parse IMDB.com
pub struct ParseImdbOptions {
    /// Entry is TV
    #[clap(long, short)]
    pub tv: bool,

    /// Season Number
    #[clap(long, short)]
    pub season: Option<i32>,

    /// List Seasons
    #[clap(long, short)]
    pub all_seasons: bool,

    /// Pull from IMDB
    #[clap(long = "update", short = 'u')]
    pub do_update: bool,

    /// Manually over-ride imdb link
    #[clap(long = "imdblink", short)]
    pub imdb_link: Option<StackString>,

    /// Update database
    #[clap(long = "database", short = 'd')]
    pub update_database: bool,

    /// Show
    pub show: StackString,
}

#[derive(Default)]
pub struct ParseImdb {
    pub mc: MovieCollection,
}

impl ParseImdb {
    #[must_use]
    pub fn new(config: &Config, pool: &PgPool, stdout: &StdoutChannel<StackString>) -> Self {
        Self {
            mc: MovieCollection::new(config, pool, stdout),
        }
    }

    /// # Errors
    /// Return error if db queries fail
    pub async fn parse_imdb_worker(
        &self,
        opts: &ParseImdbOptions,
    ) -> Result<Vec<Vec<StackString>>, Error> {
        let shows: Vec<_> = if let Some(ilink) = &opts.imdb_link {
            self.mc
                .print_imdb_shows(&opts.show, opts.tv)
                .await?
                .into_iter()
                .filter_map(|s| {
                    if s.link.as_str() == ilink.as_str() {
                        Some((s.link.clone(), s))
                    } else {
                        None
                    }
                })
                .collect()
        } else {
            self.mc
                .print_imdb_shows(&opts.show, opts.tv)
                .await?
                .into_iter()
                .map(|s| (s.link.clone(), s))
                .collect()
        };

        let mut output = Vec::new();

        if !opts.do_update {
            for (_, s) in &shows {
                output.push(s.get_string_vec());
            }
        }

        let shows: HashMap<StackString, _> = shows.into_iter().collect();

        let episodes: Option<Vec<_>> = if opts.tv {
            if opts.all_seasons {
                if !opts.do_update {
                    let seasons = self.mc.print_imdb_all_seasons(&opts.show).await?;
                    for s in seasons {
                        output.push(s.get_string_vec());
                    }
                }
                None
            } else {
                let r = self
                    .mc
                    .print_imdb_episodes(&opts.show, opts.season)
                    .await?
                    .into_iter()
                    .map(|e| ((e.season, e.episode), e))
                    .collect();
                Some(r)
            }
        } else {
            None
        };

        if !opts.do_update {
            if let Some(v) = episodes.as_ref() {
                for ((_, _), e) in v {
                    output.push(e.get_string_vec());
                }
            }
        }

        let episodes: Option<HashMap<(i32, i32), _>> = episodes.map(|v| v.into_iter().collect());

        if opts.do_update {
            self.parse_imdb_update_worker(opts, &shows, &episodes, &mut output)
                .await?;
        }
        Ok(output)
    }

    #[allow(clippy::option_if_let_else)]
    async fn parse_imdb_update_worker(
        &self,
        opts: &ParseImdbOptions,
        shows: &HashMap<StackString, ImdbRatings>,
        episodes: &Option<HashMap<(i32, i32), ImdbEpisodes>>,
        output: &mut Vec<Vec<StackString>>,
    ) -> Result<(), Error> {
        let imdb_conn = ImdbConnection::new();
        let results = imdb_conn.parse_imdb(&opts.show.replace('_', " ")).await?;
        let results = if let Some(ilink) = &opts.imdb_link {
            results
                .into_iter()
                .filter(|r| r.link.as_str() == ilink.as_str())
                .collect()
        } else {
            results
        };

        let link = if let Some(link) = &opts.imdb_link {
            Some(link.clone())
        } else {
            results.get(0).map(|result| result.link.clone())
        };

        if !opts.tv {
            if opts.update_database {
                if let Some(result) = results.get(0) {
                    let show = &opts.show;
                    let rating = result.rating;
                    if let Some(s) = shows.get(&result.link) {
                        let mut new = s.clone();
                        if !result.title.is_empty() {
                            new.title = Some(result.title.clone());
                        }
                        if rating >= 0.0 {
                            new.rating = Some(rating);
                        }
                        new.update_show(&self.mc.pool).await?;
                        output.push(vec![format_sstr!("exists {show} {s} {rating}")]);
                    } else {
                        output.push(vec![format_sstr!("not exists {show} {result}")]);
                        let istv = result.title.contains("TV Series")
                            || result.title.contains("TV Mini-Series")
                            || result.title.contains("TV Mini Series");

                        ImdbRatings {
                            show: show.clone(),
                            title: Some(result.title.clone()),
                            link: result.link.clone(),
                            rating: Some(rating),
                            istv: Some(istv),
                            ..ImdbRatings::default()
                        }
                        .insert_show(&self.mc.pool)
                        .await?;
                    }
                }
            }
            for result in &results {
                output.push(vec![StackString::from_display(result)]);
            }
        } else if let Some(link) = link {
            output.push(vec![format_sstr!("Using {link}",)]);
            if let Some(result) = shows.get(&link) {
                let episode_list = imdb_conn
                    .parse_imdb_episode_list(&link, opts.season)
                    .await?;
                for episode in episode_list {
                    output.push(vec![format_sstr!("{result} {episode}")]);
                    if opts.update_database {
                        let key = (episode.season, episode.episode);
                        if let Some(episodes) = &episodes {
                            let airdate = episode.airdate.unwrap_or_else(|| date!(1970 - 01 - 01));

                            if let Some(e) = episodes.get(&key) {
                                let mut new = e.clone();
                                if episode.eptitle.is_some() {
                                    new.eptitle =
                                        episode.eptitle.clone().unwrap_or_else(|| "".into());
                                }
                                if let Some(rating) = &episode.rating {
                                    new.rating = *rating;
                                }
                                if new.airdate != airdate {
                                    new.airdate = airdate;
                                }
                                let rating = e.rating;
                                output
                                    .push(vec![format_sstr!("exists {result} {episode} {rating}")]);
                                new.update_episode(&self.mc.pool).await?;
                            } else {
                                output.push(vec![format_sstr!("not exists {result} {episode}")]);
                                ImdbEpisodes {
                                    show: opts.show.clone(),
                                    title: result.title.clone().unwrap_or_else(|| "".into()),
                                    season: episode.season,
                                    episode: episode.episode,
                                    airdate,
                                    rating: episode.rating.unwrap_or(-1.0),
                                    eptitle: episode.eptitle.unwrap_or_else(|| "".into()),
                                    epurl: episode.epurl.unwrap_or_else(|| "".into()),
                                }
                                .insert_episode(&self.mc.pool)
                                .await?;
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// # Errors
    /// Return error if db queries fail
    pub async fn parse_imdb_http_worker(
        &self,
        opts: &ParseImdbOptions,
        watchlist: &WatchListMap,
    ) -> Result<StackString, Error> {
        let button_add = r#"<td><button type="submit" id="ID"
            onclick="watchlist_add('SHOW');">add to watchlist</button></td>"#;
        let button_rm = r#"<td><button type="submit" id="ID" 
            onclick="watchlist_rm('SHOW');">remove from watchlist</button></td>"#;

        let output: Vec<_> =
            self.parse_imdb_worker(opts)
                .await?
                .into_iter()
                .map(|line| {
                    let mut imdb_url: StackString = "".into();
                    let tmp = line.into_iter()
                        .map(|imdb_url_| {
                            if imdb_url_.starts_with("tt") {
                                imdb_url = imdb_url_;
                                format_sstr!(
                                r#"<a href="https://www.imdb.com/title/{imdb_url}" target="_blank">{imdb_url}</a>"#
                            )
                            } else {
                                imdb_url_
                            }
                        }).join("</td><td>");
                    format_sstr!(
                        "<tr><td>{tmp}</td><td>{a}</td></tr>",
                        a=if watchlist.contains_key(&imdb_url) {
                            button_rm.replace("SHOW", &imdb_url)
                        } else {
                            button_add.replace("SHOW", &imdb_url)
                        }
                    )
                })
                .collect();

        Ok(output.join("\n").into())
    }
}
