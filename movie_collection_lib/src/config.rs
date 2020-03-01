use anyhow::{format_err, Error};
use std::{env::var, ops::Deref, path::Path, sync::Arc};

#[derive(Debug, Default)]
pub struct ConfigInner {
    pub home_dir: String,
    pub pgurl: String,
    pub movie_dirs: Vec<String>,
    pub suffixes: Vec<String>,
    pub preferred_dir: String,
    pub queue_table: String,
    pub collection_table: String,
    pub ratings_table: String,
    pub episode_table: String,
    pub port: u32,
    pub domain: String,
    pub n_db_workers: usize,
    pub transcode_queue: String,
    pub remcom_queue: String,
}

#[derive(Debug, Default, Clone)]
pub struct Config(Arc<ConfigInner>);

macro_rules! set_config {
    ($s:ident, $id:ident) => {
        if let Ok($id) = var(&stringify!($id).to_uppercase()) {
            $s.$id = $id;
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
            .map_err(|e| format_err!("{} must be set: {}", stringify!($id).to_uppercase(), e))?;
    };
}

macro_rules! set_config_default {
    ($s:ident, $id:ident, $d:expr) => {
        $s.$id = var(&stringify!($id).to_uppercase()).unwrap_or_else(|_| $d);
    };
}

impl ConfigInner {
    pub fn new() -> Self {
        Self {
            home_dir: "/tmp".to_string(),
            suffixes: vec!["avi".to_string(), "mp4".to_string(), "mkv".to_string()],
            port: 8042,
            domain: "localhost".to_string(),
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
        set_config_default!(config, preferred_dir, "/tmp".to_string());
        set_config_default!(config, queue_table, "movie_queue".to_string());
        set_config_default!(config, collection_table, "movie_collection".to_string());
        set_config_default!(config, ratings_table, "imdb_ratings".to_string());
        set_config_default!(config, episode_table, "imdb_episodes".to_string());
        set_config_parse!(config, port, 8042);
        set_config!(config, domain);
        set_config_parse!(config, n_db_workers, 2);
        set_config_default!(config, transcode_queue, "transcode_work_queue".to_string());
        set_config_default!(config, remcom_queue, "remcom_worker_queue".to_string());

        config.home_dir = dirs::home_dir()
            .ok_or_else(|| format_err!("No HOME directory..."))?
            .to_string_lossy()
            .into();

        config.movie_dirs = var("MOVIEDIRS")
            .unwrap_or_else(|_| "".to_string())
            .split(',')
            .filter_map(|d| {
                if Path::new(d).exists() {
                    Some(d.to_string())
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
