extern crate amqp;
extern crate failure;
extern crate serde_json;
extern crate subprocess;

use amqp::{protocol, Basic, Channel, Options, Session, Table};
use failure::{err_msg, Error};
use std::env::var;
use std::fs::create_dir_all;
use std::fs::rename;
use std::fs::File;
use std::io::Write;
use std::io::{BufRead, BufReader};
use std::path::Path;
use subprocess::{Exec, Redirection};

use crate::config::Config;

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
                        if path_names.is_empty() {
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
        move |_: &mut Channel, _, _, body: Vec<u8>| {
            let body: ScriptStruct = serde_json::from_slice(&body).unwrap();
            let script = body.script;

            let path = Path::new(&script);
            let file_name = path.file_name().unwrap().to_str().unwrap();
            let home_dir = var("HOME").unwrap_or_else(|_| "/tmp".to_string());
            if path.exists() {
                let command = format!("sh {}", script);

                let stream = Exec::shell(&command)
                    .stderr(Redirection::Merge)
                    .stream_stdout()
                    .unwrap();

                for line in BufReader::new(stream).lines() {
                    println!("{}", line.unwrap());
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

    println!("Starting consumer {}", consumer_name);

    channel.start_consuming();

    Ok(())
}

pub fn create_transcode_script(config: &Config, path: &Path) -> Result<String, Error> {
    let full_path = path.to_str().ok_or_else(|| err_msg("Bad path"))?;
    let fstem = path
        .file_stem()
        .ok_or_else(|| err_msg("No stem"))?
        .to_str()
        .ok_or_else(|| err_msg("failure"))?;
    let script_file = format!("{}/dvdrip/jobs/{}.sh", config.home_dir, fstem);
    if Path::new(&script_file).exists() {
        Err(err_msg("File exists"))
    } else {
        let output_file = format!("{}/dvdrip/avi/{}.mp4", config.home_dir, fstem);
        let template_file = include_str!("../templates/transcode_script.sh")
            .replace("INPUT_FILE", full_path)
            .replace("OUTPUT_FILE", &output_file)
            .replace("PREFIX", &fstem);
        let mut file = File::create(script_file.clone())?;
        file.write_all(&template_file.into_bytes())?;
        Ok(script_file)
    }
}

pub fn create_move_script(
    config: &Config,
    directory: Option<String>,
    unwatched: bool,
    path: &Path,
) -> Result<String, Error> {
    let file = path.to_str().unwrap();
    let file_name = path.file_name().unwrap().to_str().unwrap();
    let prefix = path.file_stem().unwrap().to_str().unwrap();
    let output_dir = if let Some(d) = directory {
        let d = format!("{}/Documents/movies/{}", config.preferred_dir, d);
        if !Path::new(&d).exists() {
            return Err(err_msg(format!("Directory {} does not exist", d)));
        }
        d
    } else if unwatched {
        let d = format!("{}/television/unwatched", config.preferred_dir);
        if !Path::new(&d).exists() {
            return Err(err_msg(format!("Directory {} does not exist", d)));
        }
        d
    } else {
        let file_stem: Vec<_> = path
            .file_stem()
            .unwrap()
            .to_str()
            .unwrap()
            .split('_')
            .collect();
        let show = file_stem[..(file_stem.len() - 2)].join("_");
        let season: i64 = file_stem[(file_stem.len() - 2)]
            .replace("s", "")
            .parse()
            .unwrap_or(-1);
        let episode: i64 = file_stem[(file_stem.len() - 1)]
            .replace("ep", "")
            .parse()
            .unwrap_or(-1);

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

    let script_str = include_str!("../templates/move_script.sh")
        .replace("SHOW", prefix)
        .replace("OUTNAME", &format!("{}/{}", output_dir, prefix))
        .replace("FNAME", file)
        .replace("BNAME", file_name)
        .replace("ONAME", &format!("{}/{}", output_dir, prefix));

    let mut f = File::create(mp4_script.clone())?;
    f.write_all(&script_str.into_bytes())?;

    println!("dir {} file {}", output_dir, file);
    Ok(mp4_script)
}
