use anyhow::{format_err, Error};
use std::{env::var, ops::Deref, path::Path, sync::Arc};

use crate::stack_string::StackString;

#[derive(Debug, Default)]
pub struct ConfigInner {
    pub home_dir: StackString,
    pub pgurl: StackString,
    pub movie_dirs: Vec<StackString>,
    pub suffixes: Vec<StackString>,
    pub preferred_dir: StackString,
    pub queue_table: StackString,
    pub collection_table: StackString,
    pub ratings_table: StackString,
    pub episode_table: StackString,
    pub port: u32,
    pub domain: StackString,
    pub n_db_workers: usize,
    pub transcode_queue: StackString,
    pub remcom_queue: StackString,
    pub trakt_endpoint: StackString,
    pub trakt_client_id: StackString,
    pub trakt_client_secret: StackString,
}

#[derive(Debug, Default, Clone)]
pub struct Config(Arc<ConfigInner>);

macro_rules! set_config {
    ($s:ident, $id:ident) => {
        if let Ok($id) = var(&stringify!($id).to_uppercase()) {
            $s.$id = $id.into();
        }
    };
}

macro_rules! set_config_parse {
    ($s:ident, $id:ident, $d:expr) => {
        $s.$id = var(&stringify!($id).to_uppercase())
            .ok()
            .and_then(|x| x.parse().ok())
            .unwrap_or_else(|| $d);
    };
}

macro_rules! set_config_must {
    ($s:ident, $id:ident) => {
        $s.$id = var(&stringify!($id).to_uppercase())
            .map(Into::into)
            .map_err(|e| format_err!("{} must be set: {}", stringify!($id).to_uppercase(), e))?;
    };
}

macro_rules! set_config_default {
    ($s:ident, $id:ident, $d:expr) => {
        $s.$id = var(&stringify!($id).to_uppercase()).map_or_else(|_| $d, Into::into);
    };
}

impl ConfigInner {
    pub fn new() -> Self {
        Self {
            home_dir: "/tmp".into(),
            suffixes: vec!["avi".into(), "mp4".into(), "mkv".into()],
            port: 8042,
            domain: "localhost".into(),
            n_db_workers: 2,
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

        let mut config = ConfigInner::new();

        set_config_must!(config, pgurl);
        set_config_default!(config, preferred_dir, "/tmp".into());
        set_config_default!(config, queue_table, "movie_queue".into());
        set_config_default!(config, collection_table, "movie_collection".into());
        set_config_default!(config, ratings_table, "imdb_ratings".into());
        set_config_default!(config, episode_table, "imdb_episodes".into());
        set_config_parse!(config, port, 8042);
        set_config!(config, domain);
        set_config_parse!(config, n_db_workers, 2);
        set_config_default!(config, transcode_queue, "transcode_work_queue".into());
        set_config_default!(config, remcom_queue, "remcom_worker_queue".into());
        set_config_parse!(config, trakt_endpoint, "https://api.trakt.tv".into());
        set_config_default!(config, trakt_client_id, "".into());
        set_config_default!(config, trakt_client_secret, "".into());

        config.home_dir = dirs::home_dir()
            .ok_or_else(|| format_err!("No HOME directory..."))?
            .to_string_lossy()
            .to_string()
            .into();

        config.movie_dirs = var("MOVIEDIRS")
            .unwrap_or_else(|_| "".to_string())
            .split(',')
            .filter_map(|d| {
                if Path::new(d).exists() {
                    Some(d.into())
                } else {
                    None
                }
            })
            .collect();

        Ok(Self(Arc::new(config)))
    }
}

impl Deref for Config {
    type Target = ConfigInner;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
