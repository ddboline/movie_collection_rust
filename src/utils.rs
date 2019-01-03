extern crate amqp;
extern crate failure;
extern crate serde_json;
extern crate subprocess;

use amqp::{protocol, Basic, Channel, Options, Session, Table};
use failure::Error;
use std::env::var;
use std::io::{BufRead, BufReader};
use std::path::Path;
use subprocess::{Exec, Redirection};

pub fn map_result_vec<T, E>(input: Vec<Result<T, E>>) -> Result<Vec<T>, E> {
    let mut output: Vec<T> = Vec::new();
    for item in input {
        output.push(item?);
    }
    Ok(output)
}

pub fn walk_directory(path: &str, match_strs: &[String]) -> Result<Vec<String>, Error> {
    Ok(Path::new(path)
        .read_dir()?
        .filter_map(|f| match f {
            Ok(fpath) => match fpath.file_type() {
                Ok(ftype) => {
                    let path_name = fpath.path().to_str().unwrap().to_string();

                    if ftype.is_dir() {
                        Some(match walk_directory(&path_name, match_strs) {
                            Ok(v) => v,
                            Err(e) => panic!("{} {}", path_name, e),
                        })
                    } else {
                        let path_names: Vec<_> = match_strs
                            .iter()
                            .filter_map(|m| {
                                if path_name.contains(m) {
                                    Some(path_name.clone())
                                } else {
                                    None
                                }
                            })
                            .collect();
                        if path_names.len() > 0 {
                            Some(path_names)
                        } else {
                            None
                        }
                    }
                }
                Err(_) => None,
            },
            Err(_) => None,
        })
        .flatten()
        .collect())
}

#[derive(Debug, Default)]
pub struct Config {
    pub home_dir: String,
    pub pgurl: String,
    pub movie_dirs: Vec<String>,
    pub suffixes: Vec<String>,
}

impl Config {
    pub fn new() -> Config {
        Config {
            home_dir: "/tmp".to_string(),
            pgurl: "".to_string(),
            movie_dirs: Vec::new(),
            suffixes: vec!["avi".to_string(), "mp4".to_string(), "mkv".to_string()],
        }
    }

    pub fn with_config(mut self) -> Config {
        self.home_dir = var("HOME").expect("No HOME directory...");

        let env_file = format!("{}/.config/movie_collection_rust/config.env", self.home_dir);

        if Path::new("config.env").exists() {
            dotenv::from_filename("config.env").ok();
        } else if Path::new(&env_file).exists() {
            dotenv::from_path(&env_file).ok();
        } else if Path::new("config.env").exists() {
            dotenv::from_filename("config.env").ok();
        } else {
            dotenv::dotenv().ok();
        }

        self.pgurl = var("PGURL").expect("No PGURL specified");

        self.movie_dirs = var("MOVIEDIRS")
            .expect("MOVIEDIRS env variable not set")
            .split(",")
            .filter_map(|d| {
                if Path::new(d).exists() {
                    Some(d.to_string())
                } else {
                    None
                }
            })
            .collect();
        self
    }
}

pub fn get_version_number() -> String {
    format!(
        "{}.{}.{}{}",
        env!("CARGO_PKG_VERSION_MAJOR"),
        env!("CARGO_PKG_VERSION_MINOR"),
        env!("CARGO_PKG_VERSION_PATCH"),
        option_env!("CARGO_PKG_VERSION_PRE").unwrap_or("")
    )
}

#[derive(Serialize, Deserialize)]
struct ScriptStruct {
    script: String,
}

pub fn open_transcode_channel(queue: &str) -> Result<Channel, Error> {
    let options: Options = Default::default();
    let mut session = Session::new(options)?;
    let mut channel = session.open_channel(1)?;
    channel.queue_declare(queue, false, true, false, false, false, Table::new())?;
    Ok(channel)
}

pub fn publish_transcode_job_to_queue(
    script: &str,
    queue: &str,
    routing_key: &str,
) -> Result<(), Error> {
    let mut channel = open_transcode_channel(queue)?;
    channel.basic_publish(
        "",
        routing_key,
        true,
        false,
        protocol::basic::BasicProperties {
            content_type: Some("text".to_string()),
            ..Default::default()
        },
        serde_json::to_string(&ScriptStruct {
            script: script.to_string(),
        })?
        .into_bytes(),
    )?;
    Ok(())
}

pub fn read_transcode_jobs_from_queue(queue: &str) -> Result<(), Error> {
    let mut channel = open_transcode_channel(queue)?;

    let consumer_name = channel.basic_consume(
        move |ch: &mut Channel, _, _, body: Vec<u8>| {
            let body: ScriptStruct = serde_json::from_slice(&body).unwrap();
            let script = body.script;

            if Path::new(&script).exists() {
                let command = format!("sh {}", script);

                let stream = Exec::shell(&command)
                    .stderr(Redirection::Merge)
                    .stream_stdout()
                    .unwrap();

                for line in BufReader::new(stream).lines() {
                    println!("{}", line.unwrap());
                }
            }
        },
        queue,
        "",
        false,
        false,
        false,
        false,
        Table::new(),
    );

    channel.start_consuming();

    Ok(())
}
