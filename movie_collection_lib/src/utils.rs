use amqp::protocol::basic::BasicProperties;
use amqp::{Basic, Channel, Options, Session, Table};
use anyhow::{format_err, Error};
use log::error;
use reqwest::blocking::{Client, Response};
use reqwest::Url;
use retry::{delay::jitter, delay::Exponential, retry};
use serde::{Deserialize, Serialize};
use std::env::var;
use std::fs::create_dir_all;
use std::fs::rename;
use std::fs::{File, OpenOptions};
use std::io::{stdout, Write};
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::string::ToString;
use subprocess::{Exec, Redirection};

use crate::config::Config;

#[inline]
pub fn option_string_wrapper(s: &Option<String>) -> &str {
    s.as_ref().map_or("", String::as_str)
}

pub fn walk_directory(path: &str, match_strs: &[String]) -> Result<Vec<String>, Error> {
    let results = Path::new(path)
        .read_dir()?
        .filter_map(|f| match f {
            Ok(fpath) => match fpath.file_type() {
                Ok(ftype) => {
                    let path_name = fpath.path().to_string_lossy().to_string();

                    if ftype.is_dir() {
                        Some(match walk_directory(&path_name, match_strs) {
                            Ok(v) => v,
                            Err(e) => panic!("{} {}", path_name, e),
                        })
                    } else if match_strs.is_empty() {
                        Some(vec![path_name])
                    } else {
                        let path_names: Vec<_> = match_strs
                            .iter()
                            .filter_map(|m| {
                                if path_name.contains(m) {
                                    Some(path_name.to_string())
                                } else {
                                    None
                                }
                            })
                            .collect();
                        if path_names.is_empty() {
                            None
                        } else {
                            Some(path_names)
                        }
                    }
                }
                Err(_) => None,
            },
            Err(_) => None,
        })
        .flatten()
        .collect();
    Ok(results)
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

pub fn open_transcode_channel(queue: &str) -> Result<Channel, Error> {
    let options: Options = Options::default();
    let mut session = Session::new(options)?;
    let mut channel = session.open_channel(1)?;
    channel.queue_declare(queue, false, true, false, false, false, Table::new())?;
    Ok(channel)
}

#[derive(Serialize, Deserialize)]
struct ScriptStruct {
    script: String,
}

pub fn publish_transcode_job_to_queue(
    script: &str,
    queue: &str,
    routing_key: &str,
) -> Result<(), Error> {
    let mut channel = open_transcode_channel(queue)?;
    channel
        .basic_publish(
            "",
            routing_key,
            true,
            false,
            BasicProperties {
                content_type: Some("text".to_string()),
                ..BasicProperties::default()
            },
            serde_json::to_string(&ScriptStruct {
                script: script.to_string(),
            })?
            .into_bytes(),
        )
        .map(|_| ())
        .map_err(Into::into)
}

pub fn read_transcode_jobs_from_queue(queue: &str) -> Result<(), Error> {
    let mut channel = open_transcode_channel(queue)?;

    let consumer_name = channel.basic_consume(
        move |_: &mut Channel, _, _, body: Vec<u8>| {
            let body: ScriptStruct = serde_json::from_slice(&body).unwrap();
            let script = body.script;

            let mut output_file = OpenOptions::new()
                .append(true)
                .create(true)
                .open("/tmp/temp_encoding.out")
                .unwrap();

            let path = Path::new(&script);
            let file_name = path.file_name().unwrap().to_string_lossy();
            let home_dir = var("HOME").unwrap_or_else(|_| "/tmp".to_string());
            if path.exists() {
                let command = format!("sh {}", script);

                let stream = Exec::shell(&command)
                    .stderr(Redirection::Merge)
                    .stream_stdout()
                    .unwrap();

                for line in BufReader::new(stream).lines() {
                    write!(output_file, "{}", line.unwrap()).unwrap();
                }
                rename(&script, &format!("{}/tmp_avi/{}", home_dir, file_name)).unwrap();
            }
        },
        queue,
        "",
        false,
        false,
        false,
        false,
        Table::new(),
    )?;

    writeln!(stdout().lock(), "Starting consumer {}", consumer_name)?;

    channel.start_consuming();

    Ok(())
}

pub fn create_transcode_script(config: &Config, path: &Path) -> Result<String, Error> {
    let full_path = path.to_string_lossy().to_string();
    let fstem = path
        .file_stem()
        .ok_or_else(|| format_err!("No stem"))?
        .to_string_lossy();
    let script_file = format!("{}/dvdrip/jobs/{}.sh", config.home_dir, fstem);
    if Path::new(&script_file).exists() {
        Err(format_err!("File exists"))
    } else {
        let output_file = format!("{}/dvdrip/avi/{}.mp4", config.home_dir, fstem);
        let template_file = include_str!("../../templates/transcode_script.sh")
            .replace("INPUT_FILE", &full_path)
            .replace("OUTPUT_FILE", &output_file)
            .replace("PREFIX", &fstem);
        let mut file = File::create(&script_file)?;
        file.write_all(&template_file.into_bytes())?;
        Ok(script_file)
    }
}

pub fn create_move_script(
    config: &Config,
    directory: Option<&str>,
    unwatched: bool,
    path: &Path,
) -> Result<String, Error> {
    let file = path.to_string_lossy();
    let file_name = path.file_name().unwrap().to_string_lossy();
    let prefix = path.file_stem().unwrap().to_string_lossy().to_string();
    let output_dir = if let Some(d) = directory {
        let d = format!("{}/Documents/movies/{}", config.preferred_dir, d);
        if !Path::new(&d).exists() {
            return Err(format_err!("Directory {} does not exist", d));
        }
        d
    } else if unwatched {
        let d = format!("{}/television/unwatched", config.preferred_dir);
        if !Path::new(&d).exists() {
            return Err(format_err!("Directory {} does not exist", d));
        }
        d
    } else {
        let file_stem = path.file_stem().unwrap().to_string_lossy().to_string();

        let (show, season, episode) = parse_file_stem(&file_stem);

        if season == -1 || episode == -1 {
            panic!("Failed to parse show season {} episode {}", season, episode);
        }

        let d = format!(
            "{}/Documents/television/{}/season{}",
            config.preferred_dir, show, season
        );
        if !Path::new(&d).exists() {
            create_dir_all(&d)?;
        }
        d
    };
    let mp4_script = format!("{}/dvdrip/jobs/{}_copy.sh", config.home_dir, prefix);

    let script_str = include_str!("../../templates/move_script.sh")
        .replace("SHOW", &prefix)
        .replace("OUTNAME", &format!("{}/{}", output_dir, prefix))
        .replace("FNAME", &file)
        .replace("BNAME", &file_name)
        .replace("ONAME", &format!("{}/{}", output_dir, prefix));

    let mut f = File::create(&mp4_script)?;
    f.write_all(&script_str.into_bytes())?;

    writeln!(stdout().lock(), "dir {} file {}", output_dir, file)?;
    Ok(mp4_script)
}

pub fn parse_file_stem(file_stem: &str) -> (String, i32, i32) {
    let entries: Vec<_> = file_stem.split('_').collect();

    if entries.len() < 3 {
        return (file_stem.to_string(), -1, -1);
    }

    let show = entries[..(entries.len() - 2)].join("_");

    let season = entries[(entries.len() - 2)];
    let season: i32 = if season.starts_with('s') {
        season.replace("s", "").parse().unwrap_or(-1)
    } else {
        -1
    };

    let episode = entries[(entries.len() - 1)];
    let episode: i32 = if episode.starts_with("ep") {
        episode.replace("ep", "").parse().unwrap_or(-1)
    } else {
        -1
    };

    if season == -1 || episode == -1 {
        (file_stem.to_string(), -1, -1)
    } else {
        (show, season, episode)
    }
}

pub fn get_video_runtime(f: &str) -> Result<String, Error> {
    let command = if f.ends_with(".avi") {
        format!("aviindex -i {} -o /dev/null", f)
    } else {
        format!("ffprobe {} 2>&1", f)
    };

    let mut timeval = "".to_string();

    let stream = Exec::shell(command).stream_stdout()?;
    let results: Result<Vec<_>, Error> = BufReader::new(stream)
        .lines()
        .map(|l| {
            let items: Vec<_> = l?.split_whitespace().map(ToString::to_string).collect();
            if items.len() > 5 && items[1] == "V:" {
                let fps: f64 = items[2].parse()?;
                let nframes: u64 = items[5]
                    .trim_start_matches("frames=")
                    .trim_matches(',')
                    .parse()?;
                let nsecs: f64 = nframes as f64 / fps;
                let nmin = (nsecs / 60.) as u64;
                let nhour = (nmin as f64 / 60.) as u64;
                timeval = format!("{:02}:{:02}:{:02}", nhour, nmin % 60, nsecs as u64 % 60);
            }
            if items.len() > 1 && items[0] == "Duration:" {
                let its: Vec<_> = items[1].trim_matches(',').split(':').collect();
                let nhour: u64 = its[0].parse()?;
                let nmin: u64 = its[1].parse()?;
                let nsecs: f64 = its[2].parse()?;
                timeval = format!("{:02}:{:02}:{:02}", nhour, nmin, nsecs as u64);
            }
            Ok(())
        })
        .collect();
    results?;
    Ok(timeval)
}

pub fn remcom_single_file(
    file: &str,
    directory: Option<&str>,
    unwatched: bool,
) -> Result<(), Error> {
    let config = Config::with_config()?;
    let path = Path::new(&file);
    let ext = path
        .extension()
        .ok_or_else(|| format_err!("no extension"))?
        .to_string_lossy();

    let stdout = stdout();

    if ext != "mp4" {
        match create_transcode_script(&config, &path) {
            Ok(s) => {
                writeln!(stdout.lock(), "script {}", s)?;
                publish_transcode_job_to_queue(&s, &config.remcom_queue, &config.remcom_queue)?;
            }
            Err(e) => writeln!(stdout.lock(), "error {}", e)?,
        }
    }

    create_move_script(&config, directory, unwatched, &path)
        .and_then(|s| {
            writeln!(stdout.lock(), "script {}", s)?;
            publish_transcode_job_to_queue(&s, &config.remcom_queue, &config.remcom_queue)
        })
        .map_err(|e| {
            error!("{:?}", e);
            e
        })
}

pub trait ExponentialRetry {
    fn get_client(&self) -> &Client;

    fn get(&self, url: &Url) -> Result<Response, Error> {
        retry(
            Exponential::from_millis(2)
                .map(jitter)
                .map(|x| x * 500)
                .take(6),
            || {
                self.get_client().get(url.clone()).send().map_err(|e| {
                    error!("Got error {:?} , retrying", e);
                    e
                })
            },
        )
        .map_err(|e| format_err!("{:?}", e))
    }
}
