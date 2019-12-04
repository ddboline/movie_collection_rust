use failure::{err_msg, Error};
use std::env::var;
use std::ops::Deref;
use std::path::Path;
use std::sync::Arc;

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

impl ConfigInner {
    pub fn new() -> ConfigInner {
        ConfigInner {
            home_dir: "/tmp".to_string(),
            suffixes: vec!["avi".to_string(), "mp4".to_string(), "mkv".to_string()],
            port: 8042,
            domain: "localhost".to_string(),
            n_db_workers: 2,
            ..Default::default()
        }
    }
}

impl Config {
    pub fn with_config() -> Result<Config, Error> {
        let mut config = ConfigInner::new();

        config.home_dir = var("HOME").map_err(|_| err_msg("No HOME directory..."))?;

        let env_file = format!(
            "{}/.config/movie_collection_rust/config.env",
            config.home_dir
        );

        dotenv::dotenv().ok();

        if Path::new("config.env").exists() {
            dotenv::from_filename("config.env").ok();
        } else if Path::new(&env_file).exists() {
            dotenv::from_path(&env_file).ok();
        } else if Path::new("config.env").exists() {
            dotenv::from_filename("config.env").ok();
        }

        config.pgurl = var("PGURL").map_err(|_| err_msg("No PGURL specified"))?;

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

        config.preferred_dir = var("PREFERED_DISK").unwrap_or_else(|_| "/tmp".to_string());
        config.queue_table = var("QUEUE_TABLE").unwrap_or_else(|_| "movie_queue".to_string());
        config.collection_table =
            var("COLLECTION_TABLE").unwrap_or_else(|_| "movie_collection".to_string());
        config.ratings_table = var("RATINGS_TABLE").unwrap_or_else(|_| "imdb_ratings".to_string());
        config.episode_table = var("EPISODE_TABLE").unwrap_or_else(|_| "imdb_episodes".to_string());
        if let Ok(port) = var("PORT") {
            config.port = port.parse().unwrap_or(8042);
        }
        if let Ok(domain) = var("DOMAIN") {
            config.domain = domain;
        }
        if let Ok(n_db_workers_str) = var("N_DB_WORKERS") {
            if let Ok(n_db_workers) = n_db_workers_str.parse() {
                config.n_db_workers = n_db_workers
            }
        }
        config.transcode_queue =
            var("TRANSCODE_QUEUE").unwrap_or_else(|_| "transcode_work_queue".to_string());
        config.remcom_queue =
            var("REMCOM_QUEUE").unwrap_or_else(|_| "remcom_worker_queue".to_string());

        Ok(Config(Arc::new(config)))
    }
}

impl Deref for Config {
    type Target = ConfigInner;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
