use std::env::var;
use std::path::Path;

#[derive(Debug, Default, Clone)]
pub struct Config {
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
}

impl Config {
    pub fn new() -> Config {
        Config {
            home_dir: "/tmp".to_string(),
            pgurl: "".to_string(),
            movie_dirs: Vec::new(),
            suffixes: vec!["avi".to_string(), "mp4".to_string(), "mkv".to_string()],
            preferred_dir: "".to_string(),
            queue_table: "".to_string(),
            collection_table: "".to_string(),
            ratings_table: "".to_string(),
            episode_table: "".to_string(),
            port: 8042,
            domain: "localhost".to_string(),
        }
    }

    pub fn with_config() -> Config {
        let mut config = Config::new();

        config.home_dir = var("HOME").expect("No HOME directory...");

        let env_file = format!(
            "{}/.config/movie_collection_rust/config.env",
            config.home_dir
        );

        if Path::new("config.env").exists() {
            dotenv::from_filename("config.env").ok();
        } else if Path::new(&env_file).exists() {
            dotenv::from_path(&env_file).ok();
        } else if Path::new("config.env").exists() {
            dotenv::from_filename("config.env").ok();
        } else {
            dotenv::dotenv().ok();
        }

        config.pgurl = var("PGURL").expect("No PGURL specified");

        config.movie_dirs = var("MOVIEDIRS")
            .expect("MOVIEDIRS env variable not set")
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

        config
    }
}
