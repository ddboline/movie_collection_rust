use anyhow::{format_err, Error};
use serde::Deserialize;
use std::{
    ops::Deref,
    path::{Path, PathBuf},
    sync::Arc,
};

use crate::stack_string::StackString;

#[derive(Debug, Default, Deserialize)]
pub struct ConfigInner {
    #[serde(default = "default_home_dir")]
    pub home_dir: PathBuf,
    pub pgurl: StackString,
    pub movie_dirs: Vec<PathBuf>,
    #[serde(default = "default_suffixes")]
    pub suffixes: Vec<StackString>,
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
    pub trakt_client_id: StackString,
    pub trakt_client_secret: StackString,
    #[serde(default = "default_secret_key")]
    pub secret_key: StackString,
}

fn default_suffixes() -> Vec<StackString> {
    vec!["avi".into(), "mp4".into(), "mkv".into()]
}
fn default_preferred_dir() -> PathBuf {
    "/tmp".into()
}
fn default_home_dir() -> PathBuf {
    dirs::home_dir().expect("No home directory")
}
fn default_port() -> u32 {
    8042
}
fn default_secret_key() -> StackString {
    "0123".repeat(8).into()
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

#[derive(Debug, Default, Clone)]
pub struct Config(Arc<ConfigInner>);

impl ConfigInner {
    pub fn new() -> Self {
        Self {
            home_dir: default_home_dir(),
            suffixes: default_suffixes(),
            port: default_port(),
            domain: default_domain(),
            n_db_workers: default_n_db_workers(),
            ..Self::default()
        }
    }
}

impl Config {
    pub fn with_config() -> Result<Self, Error> {
        let config_dir = dirs::config_dir().ok_or_else(|| format_err!("No CONFIG directory"))?;
        let env_file = config_dir.join("movie_collection_rust").join("config.env");

        dotenv::dotenv().ok();

        if Path::new("config.env").exists() {
            dotenv::from_filename("config.env").ok();
        } else if env_file.exists() {
            dotenv::from_path(&env_file).ok();
        }

        let config: ConfigInner = envy::from_env()?;

        Ok(Self(Arc::new(config)))
    }
}

impl Deref for Config {
    type Target = ConfigInner;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
