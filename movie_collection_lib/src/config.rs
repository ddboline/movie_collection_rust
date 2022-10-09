use anyhow::{format_err, Error};
use serde::Deserialize;
use smallvec::{smallvec, SmallVec};
use std::{
    ops::Deref,
    path::{Path, PathBuf},
    sync::Arc,
};
use uuid::Uuid;

use stack_string::StackString;

use crate::timezone::TimeZone;

#[derive(Debug, Default, Deserialize)]
pub struct ConfigInner {
    #[serde(default = "default_home_dir")]
    pub home_dir: PathBuf,
    pub pgurl: StackString,
    pub movie_dirs: Vec<PathBuf>,
    #[serde(default = "default_suffixes")]
    pub suffixes: SmallVec<[StackString; 3]>,
    #[serde(default = "default_preferred_dir")]
    pub preferred_dir: PathBuf,
    #[serde(default = "default_queue_table")]
    pub queue_table: StackString,
    #[serde(default = "default_collection_table")]
    pub collection_table: StackString,
    #[serde(default = "default_ratings_table")]
    pub ratings_table: StackString,
    #[serde(default = "default_episode_table")]
    pub episode_table: StackString,
    #[serde(default = "default_host")]
    pub host: StackString,
    #[serde(default = "default_port")]
    pub port: u32,
    #[serde(default = "default_domain")]
    pub domain: StackString,
    #[serde(default = "default_n_db_workers")]
    pub n_db_workers: usize,
    #[serde(default = "default_transcode_queue")]
    pub transcode_queue: StackString,
    #[serde(default = "default_remcom_queue")]
    pub remcom_queue: StackString,
    #[serde(default = "default_trakt_endpoint")]
    pub trakt_endpoint: StackString,
    #[serde(default = "default_trakt_endpoint")]
    pub trakt_client_id: StackString,
    #[serde(default = "default_trakt_endpoint")]
    pub trakt_client_secret: StackString,
    #[serde(default = "default_secret_path")]
    pub secret_path: PathBuf,
    #[serde(default = "default_secret_path")]
    pub jwt_secret_path: PathBuf,
    pub video_playback_path: Option<PathBuf>,
    #[serde(default = "default_plex_webhook_key")]
    pub plex_webhook_key: Uuid,
    pub default_time_zone: Option<TimeZone>,
    pub plex_token: Option<StackString>,
    pub plex_host: Option<StackString>,
    pub plex_server: Option<StackString>,
}

fn default_suffixes() -> SmallVec<[StackString; 3]> {
    smallvec!["avi".into(), "mp4".into(), "mkv".into()]
}
fn default_preferred_dir() -> PathBuf {
    "/tmp".into()
}
fn default_home_dir() -> PathBuf {
    dirs::home_dir().expect("No home directory")
}
fn default_host() -> StackString {
    "0.0.0.0".into()
}
fn default_port() -> u32 {
    8042
}
fn default_domain() -> StackString {
    "localhost".into()
}
fn default_n_db_workers() -> usize {
    2
}
fn default_queue_table() -> StackString {
    "movie_queue".into()
}
fn default_collection_table() -> StackString {
    "movie_collection".into()
}
fn default_ratings_table() -> StackString {
    "imdb_ratings".into()
}
fn default_episode_table() -> StackString {
    "imdb_episodes".into()
}
fn default_transcode_queue() -> StackString {
    "transcode_work_queue".into()
}
fn default_remcom_queue() -> StackString {
    "remcom_worker_queue".into()
}
fn default_trakt_endpoint() -> StackString {
    "https://api.trakt.tv".into()
}
fn default_secret_path() -> PathBuf {
    dirs::config_dir()
        .unwrap()
        .join("aws_app_rust")
        .join("secret.bin")
}
fn default_plex_webhook_key() -> Uuid {
    Uuid::new_v4()
}

#[derive(Debug, Default, Clone)]
pub struct Config(Arc<ConfigInner>);

impl Config {
    /// # Errors
    /// Return error if parsing environment variables fails
    pub fn new() -> Result<Self, Error> {
        let config: ConfigInner = envy::from_env()?;
        Ok(Self(Arc::new(config)))
    }

    /// # Errors
    /// Return error if parsing environment variables fails
    pub fn with_config() -> Result<Self, Error> {
        let config_dir = dirs::config_dir().ok_or_else(|| format_err!("No CONFIG directory"))?;
        let env_file = config_dir.join("movie_collection_rust").join("config.env");

        dotenv::dotenv().ok();

        if Path::new("config.env").exists() {
            dotenv::from_filename("config.env").ok();
        } else if env_file.exists() {
            dotenv::from_path(&env_file).ok();
        }

        Self::new()
    }
}

impl Deref for Config {
    type Target = ConfigInner;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use anyhow::Error;
    use std::{ops::Deref, path::Path};
    use uuid::Uuid;

    use crate::{config::Config, init_env};

    #[test]
    fn test_config() -> Result<(), Error> {
        init_env();
        let config = Config::new()?;

        let plex_webhook_key: Uuid = "6f609260-4fd2-4a2c-919c-9c7766bd6400".parse()?;

        let temp_movie_dir = Path::new("/tmp");
        assert_eq!(
            &config.pgurl,
            "postgresql://USER:PASSWORD@localhost:5432/movie_queue"
        );
        assert_eq!(&config.movie_dirs[0], temp_movie_dir);
        assert_eq!(&config.preferred_dir, temp_movie_dir);
        assert_eq!(&config.domain, "DOMAIN");
        assert_eq!(&config.trakt_client_id, "8675309");
        assert_eq!(&config.trakt_client_secret, "8675309");
        assert_eq!(&config.secret_path, Path::new("/tmp/secret.bin"));
        assert_eq!(&config.jwt_secret_path, Path::new("/tmp/jwt_secret.bin"));
        assert_eq!(
            config.video_playback_path.as_ref().map(Deref::deref),
            Some(Path::new("/tmp/html"))
        );
        assert_eq!(config.plex_webhook_key, plex_webhook_key);
        Ok(())
    }
}
