use amqp::{protocol::basic::BasicProperties, Basic, Channel, Options, Session, Table};
use anyhow::{format_err, Error};
use async_trait::async_trait;
use log::{debug, error};
use rand::{
    distributions::{Distribution, Uniform},
    thread_rng,
};
use reqwest::{Client, Response, Url};
use serde::{Deserialize, Serialize};
use std::{
    fs::{create_dir_all, rename, File, OpenOptions},
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    string::ToString,
};
use subprocess::{Exec, Redirection};
use tokio::time::{delay_for, Duration};

use crate::{config::Config, stdout_channel::StdoutChannel};

#[inline]
pub fn option_string_wrapper<T: AsRef<str>>(s: &Option<T>) -> &str {
    s.as_ref().map_or("", AsRef::as_ref)
}

pub fn walk_directory<T: AsRef<str>>(path: &Path, match_strs: &[T]) -> Result<Vec<PathBuf>, Error> {
    let mut path_stack = vec![path.to_path_buf()];
    let mut output_paths = Vec::new();
    while let Some(path) = path_stack.pop() {
        if !path.exists() {
            continue;
        }
        for f in path.read_dir()? {
            if let Ok(fpath) = f {
                if let Ok(ftype) = fpath.file_type() {
                    let path = fpath.path();
                    let path_name = path.to_string_lossy().into_owned();
                    if ftype.is_dir() {
                        path_stack.push(path);
                    } else if match_strs.is_empty()
                        || match_strs.iter().any(|m| path_name.contains(m.as_ref()))
                    {
                        output_paths.push(path);
                    }
                }
            }
        }
    }
    Ok(output_paths)
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
    script: PathBuf,
}

pub fn publish_transcode_job_to_queue(
    script: &Path,
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
                script: script.into(),
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

            let file_name = script.file_name().unwrap().to_string_lossy();
            let home_dir = dirs::home_dir().unwrap_or_else(|| "/tmp".into());
            if script.exists() {
                let command = format!("sh {}", script.to_string_lossy());

                let stream = Exec::shell(&command)
                    .stderr(Redirection::Merge)
                    .stream_stdout()
                    .unwrap();
                let mut reader = BufReader::new(stream);
                let mut line = String::new();
                loop {
                    line.clear();
                    if reader.read_line(&mut line).unwrap() == 0 {
                        break;
                    }
                    write!(output_file, "{}", line).unwrap();
                }

                rename(&script, home_dir.join("tmp_avi").join(file_name.as_ref())).unwrap();
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

    debug!("Starting consumer {}", consumer_name);

    channel.start_consuming();

    Ok(())
}

pub fn create_transcode_script(config: &Config, path: &Path) -> Result<PathBuf, Error> {
    let full_path = path.to_string_lossy().to_string();
    let fstem = path.file_stem().ok_or_else(|| format_err!("No stem"))?;
    let script_file = config
        .home_dir
        .join("dvdrip")
        .join("jobs")
        .join(&fstem)
        .with_extension("sh");
    if Path::new(&script_file).exists() {
        Err(format_err!("File exists"))
    } else {
        let output_file = config
            .home_dir
            .join("dvdrip")
            .join("avi")
            .join(&fstem)
            .with_extension("mp4");
        let template_file = include_str!("../../templates/transcode_script.sh")
            .replace("INPUT_FILE", &full_path)
            .replace("OUTPUT_FILE", &output_file.to_string_lossy())
            .replace("PREFIX", &fstem.to_string_lossy());
        let mut file = File::create(&script_file)?;
        file.write_all(&template_file.into_bytes())?;
        Ok(script_file)
    }
}

pub fn create_move_script(
    config: &Config,
    directory: Option<&Path>,
    unwatched: bool,
    path: &Path,
) -> Result<PathBuf, Error> {
    let file = path.to_string_lossy();
    let file_name = path.file_name().unwrap().to_string_lossy();
    let prefix = path.file_stem().unwrap().to_string_lossy().to_string();
    let output_dir = if let Some(d) = directory {
        let d = config
            .preferred_dir
            .join("Documents")
            .join("movies")
            .join(d);
        if !d.exists() {
            return Err(format_err!(
                "Directory {} does not exist",
                d.to_string_lossy()
            ));
        }
        d
    } else if unwatched {
        let d = config.preferred_dir.join("television").join("unwatched");
        if !d.exists() {
            return Err(format_err!(
                "Directory {} does not exist",
                d.to_string_lossy()
            ));
        }
        d
    } else {
        let file_stem = path.file_stem().unwrap().to_string_lossy();

        let (show, season, episode) = parse_file_stem(&file_stem);

        if season == -1 || episode == -1 {
            panic!("Failed to parse show season {} episode {}", season, episode);
        }

        let d = config
            .preferred_dir
            .join("Documents")
            .join("television")
            .join(show)
            .join(format!("season{}", season));
        if !d.exists() {
            create_dir_all(&d)?;
        }
        d
    };
    let mp4_script = config
        .home_dir
        .join("dvdrip")
        .join("jobs")
        .join(format!("{}_copy.sh", prefix));
    let outname = output_dir.join(&prefix).to_string_lossy().to_string();
    let output_dir = output_dir.to_string_lossy();

    let script_str = include_str!("../../templates/move_script.sh")
        .replace("SHOW", &prefix)
        .replace("OUTNAME", &outname)
        .replace("FNAME", &file)
        .replace("BNAME", &file_name)
        .replace("ONAME", &outname);

    let mut f = File::create(&mp4_script)?;
    f.write_all(&script_str.into_bytes())?;

    debug!("dir {} file {}", output_dir, file);
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

pub fn get_video_runtime(f: &Path) -> Result<String, Error> {
    let ext = f
        .extension()
        .ok_or_else(|| format_err!("No extension"))?
        .to_string_lossy();
    let fname = f.to_string_lossy();
    let command = if ext == ".avi" {
        format!("aviindex -i {} -o /dev/null", fname)
    } else {
        format!("ffprobe {} 2>&1", fname)
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
    path: &Path,
    directory: Option<&Path>,
    unwatched: bool,
    stdout: &StdoutChannel,
) -> Result<(), Error> {
    let config = Config::with_config()?;
    let ext = path
        .extension()
        .ok_or_else(|| format_err!("no extension"))?
        .to_string_lossy();

    if ext != "mp4" {
        match create_transcode_script(&config, &path) {
            Ok(s) => {
                stdout.send(format!("script {:?}", s).into())?;
                publish_transcode_job_to_queue(&s, &config.remcom_queue, &config.remcom_queue)?;
            }
            Err(e) => error!("error {}", e),
        }
    }

    create_move_script(&config, directory, unwatched, &path)
        .and_then(|s| {
            stdout.send(format!("script {:?}", s).into())?;
            publish_transcode_job_to_queue(&s, &config.remcom_queue, &config.remcom_queue)
        })
        .map_err(|e| {
            error!("{:?}", e);
            e
        })
}

#[async_trait]
pub trait ExponentialRetry {
    fn get_client(&self) -> &Client;

    async fn get(&self, url: &Url) -> Result<Response, Error> {
        let mut timeout: f64 = 1.0;
        let range = Uniform::from(0..1000);
        loop {
            match self.get_client().get(url.clone()).send().await {
                Ok(resp) => return Ok(resp),
                Err(err) => {
                    delay_for(Duration::from_millis((timeout * 1000.0) as u64)).await;
                    timeout *= 4.0 * f64::from(range.sample(&mut thread_rng())) / 1000.0;
                    if timeout >= 64.0 {
                        return Err(err.into());
                    }
                }
            }
        }
    }
}
