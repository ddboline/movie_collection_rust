use anyhow::{format_err, Error};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use log::debug;
use maplit::hashmap;
use rand::{rng as thread_rng, RngExt};
use reqwest::{header::HeaderMap, Client, Url};
use serde::{Deserialize, Serialize};
use smallvec::SmallVec;
use stack_string::{format_sstr, StackString};
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::{Arc, LazyLock},
};
use time::OffsetDateTime;
use time_tz::OffsetDateTimeExt;
use tokio::{
    fs::{read, write},
    sync::{Mutex, RwLock},
};

use crate::{
    config::Config,
    date_time_wrapper::DateTimeWrapper,
    trakt_utils::{
        TraktCalEntry, TraktCalEntryList, TraktResult, WatchListShow, WatchedEpisode, WatchedMovie,
        WatchedShow,
    },
};

static CSRF_TOKEN: LazyLock<Mutex<Option<StackString>>> = LazyLock::new(|| Mutex::new(None));
static AUTH_TOKEN: LazyLock<RwLock<Option<Arc<AccessTokenResponse>>>> =
    LazyLock::new(|| RwLock::new(None));

#[derive(Clone)]
pub struct TraktConnection {
    config: Config,
    client: Client,
}

impl Default for TraktConnection {
    fn default() -> Self {
        let config = Config::with_config().expect("Failed to create");
        Self::new(config)
    }
}

impl TraktConnection {
    #[must_use]
    pub fn new(config: Config) -> Self {
        Self {
            config,
            client: Client::new(),
        }
    }

    #[must_use]
    pub fn get_client(&self) -> &Client {
        &self.client
    }

    /// # Errors
    /// Return error if `read_auth_token` fails
    pub async fn init(&self) -> Result<AccessTokenResponse, Error> {
        let auth_token = self.read_auth_token().await?;
        let auth_token = if auth_token.has_expired() {
            self.exchange_refresh_token(&auth_token).await?
        } else {
            auth_token
        };
        AUTH_TOKEN
            .write()
            .await
            .replace(Arc::new(auth_token.clone()));
        Ok(auth_token)
    }

    fn token_path() -> Result<PathBuf, Error> {
        let home_dir = dirs::home_dir().ok_or_else(|| format_err!("No home dir"))?;
        Ok(home_dir.join(".trakt").join("auth_token_web.json"))
    }

    /// # Errors
    /// Return error if parsing url fails
    pub async fn read_auth_token(&self) -> Result<AccessTokenResponse, Error> {
        serde_json::from_slice(&read(Self::token_path()?).await?).map_err(Into::into)
    }

    async fn write_auth_token(&self, token: &AccessTokenResponse) -> Result<(), Error> {
        write(&Self::token_path()?, &serde_json::to_vec(token)?)
            .await
            .map_err(Into::into)
    }

    fn get_random_string() -> String {
        let random_bytes: SmallVec<[u8; 16]> =
            (0..16).map(|_| thread_rng().random::<u8>()).collect();
        URL_SAFE_NO_PAD.encode(&random_bytes)
    }

    fn get_auth_url_impl(&self, state: &str) -> Result<Url, Error> {
        let domain = &self.config.domain;
        let redirect_uri = format_sstr!("https://{domain}/trakt/callback");
        let parameters = &[
            ("response_type", "code"),
            ("client_id", self.config.trakt_client_id.as_str()),
            ("redirect_uri", redirect_uri.as_str()),
            ("state", state),
        ];
        let trakt_endpoint = &self.config.trakt_endpoint;
        let url = format_sstr!("{trakt_endpoint}/oauth/authorize");
        Url::parse_with_params(&url, parameters).map_err(Into::into)
    }

    /// # Errors
    /// Return error if parsing url fails
    pub async fn get_auth_url(&self) -> Result<Url, Error> {
        let state = Self::get_random_string();
        let url = self.get_auth_url_impl(&state)?;
        CSRF_TOKEN.lock().await.replace(state.into());
        Ok(url)
    }

    async fn get_auth_token(
        &self,
        code: &str,
        state: Option<&str>,
    ) -> Result<AccessTokenResponse, Error> {
        if let Some(state) = state {
            let current_state = CSRF_TOKEN.lock().await.take();
            if let Some(current_state) = current_state {
                if state != current_state.as_str() {
                    return Err(format_err!("Incorrect state"));
                }
            } else {
                return Err(format_err!("Missing State"));
            }
        }

        let domain = &self.config.domain;
        let redirect_uri = format_sstr!("https://{domain}/trakt/callback");
        let trakt_endpoint = &self.config.trakt_api_endpoint;
        let url = format_sstr!("{trakt_endpoint}/oauth/token");
        let body = hashmap! {
            "code" => code,
            "client_id" => self.config.trakt_client_id.as_str(),
            "client_secret" => self.config.trakt_client_secret.as_str(),
            "redirect_uri" => redirect_uri.as_str(),
            "grant_type" => "authorization_code",
        };
        let mut headers = HeaderMap::new();
        headers.insert("Content-Type", "application/json".parse()?);
        let user_agent = &self.config.trakt_user_agent;
        headers.insert("User-Agent", user_agent.parse()?);
        let resp = self
            .client
            .post(url.as_str())
            .headers(headers)
            .json(&body)
            .send()
            .await?;
        if resp.status().as_u16() >= 400 {
            debug!("{resp:?}");
            let text = resp.text().await?;
            debug!("text {text}");
            return Err(format_err!("Forbidden"));
        }
        resp.error_for_status()?.json().await.map_err(Into::into)
    }

    async fn get_refresh_token(
        &self,
        current_auth_token: &AccessTokenResponse,
    ) -> Result<AccessTokenResponse, Error> {
        let domain = &self.config.domain;
        let redirect_uri = format_sstr!("https://{domain}/trakt/callback");
        let trakt_endpoint = &self.config.trakt_api_endpoint;
        let url = format_sstr!("{trakt_endpoint}/oauth/token");
        let body = hashmap! {
            "refresh_token" => current_auth_token.refresh_token.as_str(),
            "client_id" => self.config.trakt_client_id.as_str(),
            "client_secret" => self.config.trakt_client_secret.as_str(),
            "redirect_uri" => redirect_uri.as_str(),
            "grant_type" => "refresh_token",
        };
        let mut headers = HeaderMap::new();
        headers.insert("Content-Type", "application/json".parse()?);
        let user_agent = &self.config.trakt_user_agent;
        headers.insert("User-Agent", user_agent.parse()?);
        let resp = self
            .client
            .post(url.as_str())
            .headers(headers)
            .json(&body)
            .send()
            .await?;
        if resp.status().as_u16() >= 400 {
            debug!("{resp:?}");
        }
        resp.error_for_status()?.json().await.map_err(Into::into)
    }

    /// # Errors
    /// Return error if `get_auth_token` or `write_auth_token` fail
    pub async fn exchange_code_for_auth_token(
        &self,
        code: &str,
        state: Option<&str>,
    ) -> Result<(), Error> {
        let auth_token = self.get_auth_token(code, state).await?;
        self.write_auth_token(&auth_token).await?;
        AUTH_TOKEN.write().await.replace(Arc::new(auth_token));
        Ok(())
    }

    /// # Errors
    /// Return error if `get_refresh_token` or `write_auth_token` fail
    pub async fn exchange_refresh_token(
        &self,
        current_auth_token: &AccessTokenResponse,
    ) -> Result<AccessTokenResponse, Error> {
        let auth_token = self.get_refresh_token(current_auth_token).await?;
        self.write_auth_token(&auth_token).await?;
        AUTH_TOKEN
            .write()
            .await
            .replace(Arc::new(auth_token.clone()));
        Ok(auth_token)
    }

    fn get_ro_headers(&self) -> Result<HeaderMap, Error> {
        let mut headers = HeaderMap::new();
        headers.insert("Content-Type", "application/json".parse()?);
        headers.insert("trakt-api-key", self.config.trakt_client_id.parse()?);
        headers.insert("trakt-api-version", "2".parse()?);
        let user_agent = &self.config.trakt_user_agent;
        headers.insert("User-Agent", user_agent.parse()?);
        Ok(headers)
    }

    async fn get_rw_headers(&self) -> Result<HeaderMap, Error> {
        let mut headers = self.get_ro_headers()?;
        let auth_token = AUTH_TOKEN
            .read()
            .await
            .clone()
            .ok_or_else(|| format_err!("No auth token"))?;
        let access_token = &auth_token.access_token;
        let bearer = format_sstr!("Bearer {access_token}");
        headers.insert("Authorization", bearer.parse()?);
        let user_agent = &self.config.trakt_user_agent;
        headers.insert("User-Agent", user_agent.parse()?);
        Ok(headers)
    }

    async fn get_watchlist_shows_page(
        &self,
        page: usize,
        limit: usize,
    ) -> Result<Vec<WatchListShowsResponse>, Error> {
        let headers = self.get_rw_headers().await?;
        let trakt_endpoint = &self.config.trakt_api_endpoint;
        let url = format_sstr!("{trakt_endpoint}/sync/watchlist/shows");
        let page_str = StackString::from_display(page);
        let limit_str = StackString::from_display(limit);
        let url = Url::parse_with_params(&url, &[("page", &page_str), ("limit", &limit_str)])?;
        let resp = self
            .client
            .get(url)
            .headers(headers)
            .send()
            .await?
            .error_for_status()?;
        let headers = resp.headers();
        if let Some(current_page) = headers.get("X-Pagination-Page") {
            let current_page: usize = current_page.to_str()?.parse()?;
            assert_eq!(current_page, page);
        }
        resp.json().await.map_err(Into::into)
    }

    /// # Errors
    /// Return error if `get_watchlist_shows_page` fails
    pub async fn get_watchlist_shows(&self) -> Result<HashMap<StackString, WatchListShow>, Error> {
        let mut current_page = 1;
        let mut results = Vec::new();
        loop {
            let page = self.get_watchlist_shows_page(current_page, 20).await?;
            current_page += 1;
            if page.is_empty() {
                break;
            }
            results.extend_from_slice(&page);
        }
        let watchlist = results
            .into_iter()
            .filter_map(|r| {
                let slug = r.show.ids.slug;
                let link = format_sstr!("{}", r.show.ids.trakt);
                let imdb_link = r.show.ids.imdb;
                let title = r.show.title;
                r.show.year.map(|year| {
                    (
                        link.clone(),
                        WatchListShow {
                            link,
                            title,
                            year,
                            slug,
                            imdb_link,
                            ..WatchListShow::default()
                        },
                    )
                })
            })
            .collect();
        Ok(watchlist)
    }

    /// # Errors
    /// Return error if api call fails
    pub async fn get_show_by_imdb_id(
        &self,
        imdb_id: &str,
    ) -> Result<Vec<TraktShowSearchResponse>, Error> {
        let headers = self.get_ro_headers()?;
        let trakt_endpoint = &self.config.trakt_api_endpoint;
        let url = format_sstr!("{trakt_endpoint}/search/imdb/{imdb_id}?type=show");
        let results: Vec<TraktShowSearchResponse> = self
            .client
            .get(url.as_str())
            .headers(headers)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        if results.is_empty() {
            let headers = self.get_ro_headers()?;
            let url = format_sstr!("{trakt_endpoint}/search/trakt/{imdb_id}?type=show");
            self.client
                .get(url.as_str())
                .headers(headers)
                .send()
                .await?
                .error_for_status()?
                .json()
                .await
                .map_err(Into::into)
        } else {
            Ok(results)
        }
    }

    /// # Errors
    /// Return error if api call fails
    pub async fn get_episode_by_trakt_id(
        &self,
        trakt_id: &str,
    ) -> Result<Vec<TraktShowSearchResponse>, Error> {
        let headers = self.get_ro_headers()?;
        let trakt_endpoint = &self.config.trakt_api_endpoint;
        let url = format_sstr!("{trakt_endpoint}/search/trakt/{trakt_id}?type=episode");
        self.client
            .get(url.as_str())
            .headers(headers)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await
            .map_err(Into::into)
    }

    /// # Errors
    /// Return error if api call fails
    pub async fn search_show(&self, title: &str) -> Result<Vec<TraktShowSearchResponse>, Error> {
        let headers = self.get_ro_headers()?;
        let trakt_endpoint = &self.config.trakt_api_endpoint;
        let endpoint = format_sstr!("{trakt_endpoint}/search/show?");
        let url = Url::parse_with_params(&endpoint, &[("query", title)])?;
        self.client
            .get(url.as_str())
            .headers(headers)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await
            .map_err(Into::into)
    }

    /// # Errors
    /// Return error if api call fails
    pub async fn get_movie_by_imdb_id(
        &self,
        imdb_id: &str,
    ) -> Result<Vec<TraktMovieSearchResponse>, Error> {
        let headers = self.get_ro_headers()?;
        let trakt_endpoint = &self.config.trakt_api_endpoint;
        let url = format_sstr!("{trakt_endpoint}/search/imdb/{imdb_id}?type=movie");
        self.client
            .get(url.as_str())
            .headers(headers)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await
            .map_err(Into::into)
    }

    /// # Errors
    /// Return error if api call fails
    pub async fn search_movie(&self, title: &str) -> Result<Vec<TraktMovieSearchResponse>, Error> {
        let headers = self.get_ro_headers()?;
        let trakt_endpoint = &self.config.trakt_api_endpoint;
        let endpoint = format_sstr!("{trakt_endpoint}/search/movie?");
        let url = Url::parse_with_params(&endpoint, &[("query", title)])?;
        self.client
            .get(url.as_str())
            .headers(headers)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await
            .map_err(Into::into)
    }

    /// # Errors
    /// Return error if api call fails
    pub async fn get_movie_rating(&self, imdb_id: &str) -> Result<TraktRating, Error> {
        let headers = self.get_ro_headers()?;
        let trakt_endpoint = &self.config.trakt_api_endpoint;
        let url = format_sstr!("{trakt_endpoint}/movies/{imdb_id}/ratings");
        self.client
            .get(url.as_str())
            .headers(headers)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await
            .map_err(Into::into)
    }

    /// # Errors
    /// Return error if api call fails
    pub async fn get_seasons(&self, imdb_id: &str) -> Result<Vec<TraktSeasonObject>, Error> {
        let headers = self.get_ro_headers()?;
        let trakt_endpoint = &self.config.trakt_api_endpoint;
        let url = format_sstr!("{trakt_endpoint}/shows/{imdb_id}/seasons");
        self.client
            .get(url.as_str())
            .headers(headers)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await
            .map_err(Into::into)
    }

    /// # Errors
    /// Return error if api call fails
    pub async fn get_season_episodes(
        &self,
        imdb_id: &str,
        season: i32,
    ) -> Result<Vec<TraktEpisodeObject>, Error> {
        let headers = self.get_ro_headers()?;
        let trakt_endpoint = &self.config.trakt_api_endpoint;
        let url = format_sstr!("{trakt_endpoint}/shows/{imdb_id}/seasons/{season}");
        self.client
            .get(url.as_str())
            .headers(headers)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await
            .map_err(Into::into)
    }

    /// # Errors
    /// Return error if api call fails
    pub async fn get_episode(
        &self,
        imdb_id: &str,
        season: i32,
        episode: i32,
        extended: bool,
    ) -> Result<TraktEpisodeObject, Error> {
        let headers = self.get_ro_headers()?;
        let trakt_endpoint = &self.config.trakt_api_endpoint;
        let mut url =
            format_sstr!("{trakt_endpoint}/shows/{imdb_id}/seasons/{season}/episodes/{episode}");
        if extended {
            url = format_sstr!("{url}?extended=full");
        }
        self.client
            .get(url.as_str())
            .headers(headers)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await
            .map_err(Into::into)
    }

    /// # Errors
    /// Return error if api call fails
    pub async fn get_episode_rating(
        &self,
        imdb_id: &str,
        season: i32,
        episode: i32,
    ) -> Result<TraktRating, Error> {
        let headers = self.get_ro_headers()?;
        let trakt_endpoint = &self.config.trakt_api_endpoint;
        let url = format_sstr!(
            "{trakt_endpoint}/shows/{imdb_id}/seasons/{season}/episodes/{episode}/ratings"
        );
        self.client
            .get(url.as_str())
            .headers(headers)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await
            .map_err(Into::into)
    }

    /// # Errors
    /// Return error if api call fails
    pub async fn get_show_rating(&self, imdb_id: &str) -> Result<TraktRating, Error> {
        let headers = self.get_ro_headers()?;
        let trakt_endpoint = &self.config.trakt_api_endpoint;
        let url = format_sstr!("{trakt_endpoint}/shows/{imdb_id}/ratings");
        self.client
            .get(url.as_str())
            .headers(headers)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await
            .map_err(Into::into)
    }

    /// # Errors
    /// Return error if api call fails
    pub async fn add_watchlist_show(&self, imdb_id: &str) -> Result<TraktResult, Error> {
        let show_obj = self
            .get_show_by_imdb_id(imdb_id)
            .await?
            .pop()
            .ok_or_else(|| format_err!("No show returned"))?;
        let headers = self.get_rw_headers().await?;
        let trakt_endpoint = &self.config.trakt_api_endpoint;
        let url = format_sstr!("{trakt_endpoint}/sync/watchlist");
        let data = hashmap! {
            "shows" => vec![show_obj.show],
        };
        debug!("shows: {}", serde_json::to_string_pretty(&data)?);
        let text = self
            .client
            .post(url.as_str())
            .headers(headers)
            .json(&data)
            .send()
            .await?
            .error_for_status()?
            .text()
            .await?;
        Ok(TraktResult {
            status: text.into(),
        })
    }

    /// # Errors
    /// Return error if api call fails
    pub async fn remove_watchlist_show(&self, imdb_id: &str) -> Result<TraktResult, Error> {
        let show_obj = self
            .get_show_by_imdb_id(imdb_id)
            .await?
            .pop()
            .ok_or_else(|| format_err!("No show returned"))?;
        let headers = self.get_rw_headers().await?;
        let trakt_endpoint = &self.config.trakt_api_endpoint;
        let url = format_sstr!("{trakt_endpoint}/sync/watchlist/remove");
        let data = hashmap! {
            "shows" => vec![show_obj.show],
        };
        let text = self
            .client
            .post(url.as_str())
            .headers(headers)
            .json(&data)
            .send()
            .await?
            .error_for_status()?
            .text()
            .await?;
        Ok(TraktResult {
            status: text.into(),
        })
    }

    /// # Errors
    /// Return error if api call fails
    pub async fn get_watched_shows(&self) -> Result<Vec<WatchedShow>, Error> {
        let mut watched_shows = Vec::new();
        let mut page = 1;
        loop {
            let headers = self.get_rw_headers().await?;
            let trakt_endpoint = &self.config.trakt_api_endpoint;
            let url = format_sstr!("{trakt_endpoint}/sync/watched/shows?page={page}");
            let new_shows: Vec<TraktWatchedShowResponse> = self
                .client
                .get(url.as_str())
                .headers(headers)
                .send()
                .await?
                .error_for_status()?
                .json()
                .await?;
            if new_shows.is_empty() {
                break;
            }
            watched_shows.extend(new_shows);
            page += 1;
        }
        let watched_shows = watched_shows
            .into_iter()
            .map(|entry| {
                let title = entry.show.title;
                let link = format_sstr!("{}", entry.show.ids.trakt);
                let imdb_link = entry.show.ids.imdb;
                let slug = entry
                    .show
                    .ids
                    .slug
                    .as_ref()
                    .map_or(StackString::new(), Clone::clone);
                let last_watched_at = entry.last_watched_at;
                WatchedShow {
                    title,
                    link,
                    slug,
                    last_watched_at,
                    imdb_link,
                    ..WatchedShow::default()
                }
            })
            .collect();
        Ok(watched_shows)
    }

    /// # Errors
    /// Return error if api call fails
    pub async fn get_watched_episodes(
        &self,
        last_watched_episode: Option<DateTimeWrapper>,
    ) -> Result<HashMap<(StackString, i32, i32), WatchedEpisode>, Error> {
        let mut watched_episodes = Vec::new();
        let mut page = 1;
        loop {
            let headers = self.get_rw_headers().await?;
            let trakt_endpoint = &self.config.trakt_api_endpoint;
            let url = format_sstr!("{trakt_endpoint}/sync/watched/episodes?page={page}");
            let mut new_episodes: Vec<TraktWatchedEpisodeNew> = self
                .client
                .get(url.as_str())
                .headers(headers)
                .send()
                .await?
                .error_for_status()?
                .json()
                .await?;
            if let Some(last_watched_episode) = last_watched_episode {
                new_episodes.retain(|e| e.last_watched_at >= last_watched_episode);
            }
            if new_episodes.is_empty() {
                break;
            }
            watched_episodes.extend(new_episodes);
            page += 1;
        }

        #[allow(clippy::manual_filter_map)]
        let episode_map = watched_episodes
            .into_iter()
            .map(|episode_entry| {
                let last_watched_at = Some(episode_entry.last_watched_at);
                let title = episode_entry.episode.title.clone();
                let episode = episode_entry.episode.number;
                let season = episode_entry.episode.season;
                let link = format_sstr!("{}", episode_entry.episode.ids.trakt);

                if let Some(imdb_url) = episode_entry.episode.ids.imdb {
                    let imdb_link = imdb_url.clone();
                    let epi = WatchedEpisode {
                        title,
                        link: link.clone(),
                        season,
                        episode,
                        last_watched_at,
                        imdb_link,
                        ..WatchedEpisode::default()
                    };
                    ((link, season, episode), epi)
                } else {
                    let epi = WatchedEpisode {
                        title: title.clone(),
                        link: link.clone(),
                        season,
                        episode,
                        last_watched_at,
                        ..WatchedEpisode::default()
                    };
                    ((link, season, episode), epi)
                }
            })
            .collect();
        Ok(episode_map)
    }

    /// # Errors
    /// Return error if api call fails
    pub async fn get_watched_movies(&self) -> Result<HashSet<WatchedMovie>, Error> {
        let mut watched_movies = Vec::new();
        let mut page = 1;
        loop {
            let headers = self.get_rw_headers().await?;
            let trakt_endpoint = &self.config.trakt_api_endpoint;
            let url = format_sstr!("{trakt_endpoint}/sync/watched/movies?page={page}");
            let new_movies: Vec<TraktWatchedMovieResponse> = self
                .client
                .get(url.as_str())
                .headers(headers)
                .send()
                .await?
                .error_for_status()?
                .json()
                .await?;
            if new_movies.is_empty() {
                break;
            }
            watched_movies.extend(new_movies);
            page += 1;
        }

        let movie_map: HashSet<WatchedMovie> = watched_movies
            .into_iter()
            .map(|entry| {
                let slug = entry.movie.ids.slug.clone();
                let link = format_sstr!("{}", entry.movie.ids.trakt);
                let imdb_link = entry.movie.ids.imdb;
                WatchedMovie {
                    title: entry.movie.title,
                    link,
                    imdb_link,
                    last_watched_at: Some(entry.last_watched_at),
                    slug,
                }
            })
            .collect();
        Ok(movie_map)
    }

    /// # Errors
    /// Return error if api call fails
    pub async fn get_calendar(&self) -> Result<TraktCalEntryList, Error> {
        let local = DateTimeWrapper::local_tz();
        let today = DateTimeWrapper::now().date();
        let headers = self.get_rw_headers().await?;
        let trakt_endpoint = &self.config.trakt_api_endpoint;
        let url = format_sstr!("{trakt_endpoint}/calendars/my/shows/{today}/7");
        let new_episodes: Vec<TraktCalendarResponse> = self
            .client
            .get(url.as_str())
            .headers(headers)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        let cal_entries = new_episodes
            .into_iter()
            .map(|entry| {
                let trakt_id = entry.show.ids.trakt;
                let imdb: StackString = entry
                    .show
                    .ids
                    .imdb
                    .unwrap_or_else(|| format_sstr!("{trakt_id}"));
                TraktCalEntry {
                    ep_link: entry.episode.ids.imdb.clone(),
                    episode: entry.episode.number,
                    link: imdb,
                    season: entry.episode.season,
                    show: entry.show.title,
                    airdate: entry.first_aired.to_timezone(local).date(),
                }
            })
            .collect();
        Ok(cal_entries)
    }

    /// # Errors
    /// Return error if api call fails
    pub async fn add_episode_to_watched(
        &self,
        imdb_id: &str,
        season: i32,
        episode: i32,
    ) -> Result<TraktResult, Error> {
        let episode_obj = self.get_episode(imdb_id, season, episode, false).await?;
        let headers = self.get_rw_headers().await?;
        let trakt_endpoint = &self.config.trakt_api_endpoint;
        let url = format_sstr!("{trakt_endpoint}/sync/history");
        let data = hashmap! {
            "episodes" => vec![
                WatchedEpisodeRequest {
                    watched_at: DateTimeWrapper::now(),
                    ids: episode_obj.ids,
                }
            ]
        };
        self.client
            .post(url.as_str())
            .headers(headers)
            .json(&data)
            .send()
            .await?
            .error_for_status()?;
        Ok(TraktResult {
            status: "success add episode".into(),
        })
    }

    /// # Errors
    /// Return error if api call fails
    pub async fn add_movie_to_watched(&self, imdb_id: &str) -> Result<TraktResult, Error> {
        let movie_obj = {
            self.get_movie_by_imdb_id(imdb_id)
                .await?
                .into_iter()
                .find(|o| o.movie.year.is_some())
                .ok_or_else(|| format_err!("No show returned"))?
        };
        let headers = self.get_rw_headers().await?;
        let trakt_endpoint = &self.config.trakt_api_endpoint;
        let url = format_sstr!("{trakt_endpoint}/sync/history");
        let year = movie_obj.movie.year.ok_or_else(|| format_err!("No year"))?;
        let data = hashmap! {
            "movies" => vec![
                WatchedMovieRequest {
                    watched_at: DateTimeWrapper::now(),
                    title: movie_obj.movie.title.clone(),
                    year,
                    ids: movie_obj.movie.ids,
                }
            ]
        };
        self.client
            .post(url.as_str())
            .headers(headers)
            .json(&data)
            .send()
            .await?
            .error_for_status()?;
        Ok(TraktResult {
            status: "success add movie".into(),
        })
    }

    /// # Errors
    /// Return error if api call fails
    pub async fn remove_episode_to_watched(
        &self,
        imdb_id: &str,
        season: i32,
        episode: i32,
    ) -> Result<TraktResult, Error> {
        let episode_obj = self.get_episode(imdb_id, season, episode, false).await?;
        let headers = self.get_rw_headers().await?;
        let trakt_endpoint = &self.config.trakt_api_endpoint;
        let url = format_sstr!("{trakt_endpoint}/sync/history/remove");
        let data = hashmap! {
            "episodes" => vec![
                WatchedEpisodeRequest {
                    watched_at: DateTimeWrapper::now(),
                    ids: episode_obj.ids,
                }
            ]
        };
        self.client
            .post(url.as_str())
            .headers(headers)
            .json(&data)
            .send()
            .await?
            .error_for_status()?;
        Ok(TraktResult {
            status: "success remove episode".into(),
        })
    }

    /// # Errors
    /// Return error if api call fails
    pub async fn remove_movie_to_watched(&self, imdb_id: &str) -> Result<TraktResult, Error> {
        let movie_obj = self
            .get_movie_by_imdb_id(imdb_id)
            .await?
            .into_iter()
            .find(|o| o.movie.year.is_some())
            .ok_or_else(|| format_err!("No show returned"))?;
        let headers = self.get_rw_headers().await?;
        let trakt_endpoint = &self.config.trakt_api_endpoint;
        let url = format_sstr!("{trakt_endpoint}/sync/history/remove");
        let year = movie_obj.movie.year.ok_or_else(|| format_err!("No year"))?;
        let data = hashmap! {
            "movies" => vec![
                WatchedMovieRequest {
                    watched_at: DateTimeWrapper::now(),
                    title: movie_obj.movie.title.clone(),
                    year,
                    ids: movie_obj.movie.ids,
                }
            ]
        };
        self.client
            .post(url.as_str())
            .headers(headers)
            .json(&data)
            .send()
            .await?
            .error_for_status()?;
        Ok(TraktResult {
            status: "success remove movie".into(),
        })
    }

    pub async fn get_last_activities(&self) -> Result<TraktActivities, Error> {
        let headers = self.get_rw_headers().await?;
        let trakt_endpoint = &self.config.trakt_api_endpoint;
        let url = format_sstr!("{trakt_endpoint}/sync/last_activities");
        self.client
            .get(url.as_str())
            .headers(headers)
            .send()
            .await?
            .json()
            .await
            .map_err(Into::into)
    }
}

#[derive(Serialize, Deserialize, Debug)]
struct WatchedMovieRequest {
    pub watched_at: DateTimeWrapper,
    pub title: StackString,
    pub year: i32,
    pub ids: TraktIdObject,
}

#[derive(Serialize, Deserialize, Debug)]
struct WatchedEpisodeRequest {
    pub watched_at: DateTimeWrapper,
    pub ids: TraktIdObject,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct AccessTokenResponse {
    access_token: StackString,
    token_type: StackString,
    expires_in: u64,
    refresh_token: StackString,
    scope: StackString,
    created_at: u64,
}

impl AccessTokenResponse {
    #[must_use]
    pub fn has_expired(&self) -> bool {
        let expires_at = (self.created_at + self.expires_in) as i64;
        expires_at < OffsetDateTime::now_utc().unix_timestamp()
    }

    #[must_use]
    pub fn expires_soon(&self) -> bool {
        let expires_at = (self.created_at + self.expires_in) as i64;
        expires_at < OffsetDateTime::now_utc().unix_timestamp() + 5400
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TraktIdObject {
    pub trakt: i32,
    pub slug: Option<StackString>,
    pub imdb: Option<StackString>,
    pub tvdb: Option<i32>,
    pub tmdb: Option<i32>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TraktShowObject {
    pub title: StackString,
    pub year: Option<i32>,
    pub ids: TraktIdObject,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TraktSeasonObject {
    pub number: i32,
    pub ids: TraktIdObject,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct TraktEpisodeObject {
    pub season: i32,
    pub number: i32,
    pub title: StackString,
    pub ids: TraktIdObject,
    pub first_aired: Option<DateTimeWrapper>,
    pub rating: Option<f64>,
    pub nrating: Option<i32>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct WatchListShowsResponse {
    pub show: TraktShowObject,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct TraktShowSearchResponse {
    pub show: TraktShowObject,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct TraktMovieSearchResponse {
    pub movie: TraktShowObject,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct TraktWatchedEpisodeNew {
    pub last_watched_at: DateTimeWrapper,
    pub episode: TraktEpisodeObject,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct TraktWatchedEpisode {
    pub number: i32,
    pub last_watched_at: DateTimeWrapper,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct TraktWatchedSeason {
    pub number: i32,
    pub episodes: Vec<TraktWatchedEpisode>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct TraktWatchedShowResponseNew {
    pub show: TraktShowObject,
    pub last_watched_at: DateTimeWrapper,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct TraktWatchedShowResponse {
    pub show: TraktShowObject,
    pub last_watched_at: DateTimeWrapper,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct TraktWatchedMovieResponse {
    pub movie: TraktShowObject,
    pub last_watched_at: DateTimeWrapper,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct TraktCalendarResponse {
    pub first_aired: DateTimeWrapper,
    pub episode: TraktEpisodeObject,
    pub show: TraktShowObject,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct TraktRating {
    pub rating: f64,
    pub votes: i32,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct TraktActivityDetail {
    pub reacted_at: Option<DateTimeWrapper>,
    pub updated_at: Option<DateTimeWrapper>,
    pub watched_at: Option<DateTimeWrapper>,
    pub collected_at: Option<DateTimeWrapper>,
    pub rated_at: Option<DateTimeWrapper>,
    pub watchlisted_at: Option<DateTimeWrapper>,
    pub favorited_at: Option<DateTimeWrapper>,
    pub recommendations_at: Option<DateTimeWrapper>,
    pub commented_at: Option<DateTimeWrapper>,
    pub paused_at: Option<DateTimeWrapper>,
    pub hidden_at: Option<DateTimeWrapper>,
    pub blocked_at: Option<DateTimeWrapper>,
    pub settings_at: Option<DateTimeWrapper>,
    pub requested_at: Option<DateTimeWrapper>,
    pub dropped_at: Option<DateTimeWrapper>,
    pub following_at: Option<DateTimeWrapper>,
    pub liked_at: Option<DateTimeWrapper>,
    pub pending_at: Option<DateTimeWrapper>,
    pub followed_at: Option<DateTimeWrapper>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct TraktActivities {
    pub all: DateTimeWrapper,
    pub movies: TraktActivityDetail,
    pub episodes: TraktActivityDetail,
    pub shows: TraktActivityDetail,
    pub seasons: TraktActivityDetail,
    pub comments: TraktActivityDetail,
    pub lists: TraktActivityDetail,
    pub watchlist: TraktActivityDetail,
    pub favorites: TraktActivityDetail,
    pub recommendations: TraktActivityDetail,
    pub collaborations: TraktActivityDetail,
    pub account: TraktActivityDetail,
    pub saved_filters: TraktActivityDetail,
    pub notes: TraktActivityDetail,
}

#[cfg(test)]
mod tests {
    use crate::{config::Config, trakt_connection::TraktConnection};
    use anyhow::Error;
    use log::debug;
    use stack_string::format_sstr;

    #[test]
    #[ignore]
    fn test_get_auth_url() -> Result<(), Error> {
        let config = Config::with_config()?;
        let conn = TraktConnection::new(config);
        let test_state = TraktConnection::get_random_string();
        let url = conn.get_auth_url_impl(test_state.as_str())?;
        debug!("url {}", url);
        let expected = format_sstr!(
            "https://trakt.tv/oauth/authorize?{a}{client_id}{b}{domain}%2Ftrakt%2Fcallback&state={state}",
            a="response_type=code&client_id=",
            client_id=conn.config.trakt_client_id,
            b="&redirect_uri=https%3A%2F%2F",
            domain=conn.config.domain,
            state=test_state,
        );
        assert_eq!(url.as_str(), expected.as_str());
        Ok(())
    }

    #[tokio::test]
    #[ignore]
    async fn test_read_auth_token() -> Result<(), Error> {
        let config = Config::with_config()?;
        let conn = TraktConnection::new(config);
        let auth_token = conn.read_auth_token().await?;
        assert_eq!(auth_token.scope, "public");
        assert_eq!(auth_token.has_expired(), false);
        Ok(())
    }

    #[tokio::test]
    #[ignore]
    async fn test_get_watchlist_shows() -> Result<(), Error> {
        let config = Config::with_config()?;
        let conn = TraktConnection::new(config);
        conn.init().await?;
        let result = conn.get_watchlist_shows().await?;
        assert!(result.len() > 10);
        Ok(())
    }

    #[tokio::test]
    #[ignore]
    async fn test_get_movie_rating() -> Result<(), Error> {
        let config = Config::with_config()?;
        let conn = TraktConnection::new(config);
        conn.init().await?;
        let result = conn.get_movie_rating("tt0457430").await?;
        debug!("{result:?}");
        assert!(result.rating > 7.0);
        Ok(())
    }

    #[tokio::test]
    #[ignore]
    async fn test_get_episode_rating() -> Result<(), Error> {
        let config = Config::with_config()?;
        let conn = TraktConnection::new(config);
        conn.init().await?;
        let result = conn.get_episode_rating("tt12708542", 1, 10).await?;
        assert!(result.rating > 7.0);
        Ok(())
    }

    #[tokio::test]
    #[ignore]
    async fn test_get_show_rating() -> Result<(), Error> {
        let config = Config::with_config()?;
        let conn = TraktConnection::new(config);
        conn.init().await?;
        let result = conn.get_show_rating("tt0141842").await?;
        debug!("{:?}", result);
        assert!(result.rating > 9.0);
        Ok(())
    }

    #[tokio::test]
    #[ignore]
    async fn test_get_show_season_episodes() -> Result<(), Error> {
        let config = Config::with_config()?;
        let conn = TraktConnection::new(config);
        conn.init().await?;
        let result = conn.get_season_episodes("tt0141842", 1).await?;
        assert!(result.len() == 13);
        assert!(result[0].title == "The Sopranos");
        assert!(result[0].ids.imdb == Some("tt0705282".into()));
        Ok(())
    }

    #[tokio::test]
    #[ignore]
    async fn test_get_show_season_episodes_alt() -> Result<(), Error> {
        let config = Config::with_config()?;
        let conn = TraktConnection::new(config);
        conn.init().await?;
        let result = conn.get_season_episodes("tt13629530", 1).await?;
        debug!("{:?}", result[0]);
        debug!("{}", result.len());
        assert!(result.len() == 14);
        assert!(result[0].title == "Element 1");
        assert!(result[0].ids.imdb == Some("tt14260644".into()));
        Ok(())
    }

    #[tokio::test]
    #[ignore]
    async fn test_search_show() -> Result<(), Error> {
        let config = Config::with_config()?;
        let conn = TraktConnection::new(config);
        conn.init().await?;
        let result = conn.search_show("the sopranos").await?;
        let top_result = result
            .iter()
            .filter(|s| &s.show.title == "The Sopranos")
            .next()
            .unwrap();
        assert_eq!(top_result.show.ids.imdb, Some("tt0141842".into()));
        Ok(())
    }

    #[tokio::test]
    #[ignore]
    async fn test_search_show_alternate() -> Result<(), Error> {
        let config = Config::with_config()?;
        let conn = TraktConnection::new(config);
        conn.init().await?;
        let result = conn.search_show("ark the animated series").await?;
        debug!("{:?}", result);
        let top_result = result
            .iter()
            .filter(|s| &s.show.title == "ARK: The Animated Series")
            .next()
            .unwrap();
        assert_eq!(top_result.show.ids.imdb, Some("tt13629530".into()));
        Ok(())
    }

    #[tokio::test]
    #[ignore]
    async fn test_search_movie() -> Result<(), Error> {
        let config = Config::with_config()?;
        let conn = TraktConnection::new(config);
        conn.init().await?;
        let result = conn.search_movie("pans labyrinth").await?;
        debug!("{:?}", result);
        let top_result = result
            .iter()
            .filter(|s| &s.movie.title == "Pan's Labyrinth")
            .next()
            .unwrap();
        assert_eq!(top_result.movie.ids.imdb, Some("tt0457430".into()));
        assert_eq!(top_result.movie.year, Some(2006));

        let result = conn.search_movie("bugonia").await?;
        debug!("{:?}", result);
        let top_result = result
            .iter()
            .filter(|s| &s.movie.title == "Bugonia")
            .next()
            .unwrap();
        assert_eq!(top_result.movie.ids.imdb, Some("tt12300742".into()));
        Ok(())
    }

    #[tokio::test]
    #[ignore]
    async fn test_get_show_by_imdb_id() -> Result<(), Error> {
        let imdb_id = "tt4270492";
        let config = Config::with_config()?;
        let conn = TraktConnection::new(config);
        conn.init().await?;
        let result = conn.get_show_by_imdb_id(imdb_id).await?;
        assert_eq!(result[0].show.title, "Billions");
        Ok(())
    }

    #[tokio::test]
    #[ignore]
    async fn test_get_show_by_imdb_id_trakt() -> Result<(), Error> {
        let imdb_id = "236198";
        let config = Config::with_config()?;
        let conn = TraktConnection::new(config);
        conn.init().await?;
        let result = conn.get_show_by_imdb_id(imdb_id).await?;
        debug!("result {:?}", result);
        assert_eq!(result[0].show.title, "The Vampire Lestat");
        Ok(())
    }

    #[tokio::test]
    #[ignore]
    async fn test_get_by_trakt_id() -> Result<(), Error> {
        let trakt_id = "1756097";
        let config = Config::with_config()?;
        let conn = TraktConnection::new(config);
        conn.init().await?;
        let result = conn.get_episode_by_trakt_id(trakt_id).await?;
        debug!("result {:?}", result[0]);
        assert_eq!(result[0].show.title.as_str(), "Shark Tank");
        assert_eq!(
            result[0].show.ids.imdb.as_ref().unwrap().as_str(),
            "tt1442550"
        );
        Ok(())
    }

    #[tokio::test]
    #[ignore]
    async fn test_get_watched_shows() -> Result<(), Error> {
        let config = Config::with_config()?;
        let conn = TraktConnection::new(config);
        conn.init().await?;
        let result = conn.get_watched_shows().await?;
        assert!(result.len() > 10);
        Ok(())
    }

    #[tokio::test]
    #[ignore]
    async fn test_get_watched_episodes() -> Result<(), Error> {
        let config = Config::with_config()?;
        let conn = TraktConnection::new(config);
        conn.init().await?;
        let result = conn.get_watched_episodes(None).await?;
        assert!(result.len() > 10);
        Ok(())
    }

    #[tokio::test]
    #[ignore]
    async fn test_get_last_activities() -> Result<(), Error> {
        let config = Config::with_config()?;
        let conn = TraktConnection::new(config);
        conn.init().await?;
        let result = conn.get_last_activities().await?;
        assert!(result.movies.watched_at.is_some());
        assert!(result.shows.watchlisted_at.is_some());
        assert!(result.episodes.watched_at.is_some());
        assert!(result.watchlist.updated_at.is_some());
        Ok(())
    }

    #[tokio::test]
    #[ignore]
    async fn test_get_watched_movies() -> Result<(), Error> {
        let config = Config::with_config()?;
        let conn = TraktConnection::new(config);
        conn.init().await?;
        let result = conn.get_watched_movies().await?;
        debug!("{}", result.len());
        assert!(result.len() > 5);
        Ok(())
    }

    #[tokio::test]
    #[ignore]
    async fn test_get_calendar() -> Result<(), Error> {
        let config = Config::with_config()?;
        let conn = TraktConnection::new(config);
        conn.init().await?;

        let result = conn.get_calendar().await?;
        debug!("{}", result.len());
        assert!(result.len() > 1);
        Ok(())
    }
}
