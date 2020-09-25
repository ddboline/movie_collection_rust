use anyhow::{format_err, Error};
use async_trait::async_trait;
use handlebars::Handlebars;
use lazy_static::lazy_static;
use rand::{
    distributions::{Distribution, Uniform},
    thread_rng,
};
use reqwest::{Client, Response, Url};
use serde::{Deserialize, Serialize};
use smallvec::SmallVec;
use stack_string::StackString;
use std::path::{Path, PathBuf};
use tokio::{
    process::Command,
    time::{delay_for, Duration},
};
use walkdir::WalkDir;

lazy_static! {
    pub static ref HBR: Handlebars<'static> = get_templates().expect("Failed to parse templates");
}

fn get_templates() -> Result<Handlebars<'static>, Error> {
    let mut h = Handlebars::new();
    h.register_template_string("index.html", include_str!("../../templates/index.html"))?;
    Ok(h)
}

#[inline]
pub fn option_string_wrapper<T: AsRef<str>>(s: &Option<T>) -> &str {
    s.as_ref().map_or("", AsRef::as_ref)
}

pub fn walk_directory<T: AsRef<str>>(path: &Path, match_strs: &[T]) -> Result<Vec<PathBuf>, Error> {
    WalkDir::new(path)
        .into_iter()
        .filter_map(|f| match f {
            Ok(fpath) => {
                let ftype = fpath.file_type();
                let path = fpath.path();
                let path_name = path.to_string_lossy().into_owned();
                if !ftype.is_dir()
                    && (match_strs.is_empty()
                        || match_strs.iter().any(|m| path_name.contains(m.as_ref())))
                {
                    Some(Ok(path.to_path_buf()))
                } else {
                    None
                }
            }
            Err(e) => Some(Err(e.into())),
        })
        .collect()
}

#[derive(Serialize, Deserialize)]
struct ScriptStruct {
    script: PathBuf,
}

pub fn parse_file_stem(file_stem: &str) -> (StackString, i32, i32) {
    let entries: Vec<_> = file_stem.split('_').collect();

    if entries.len() < 3 {
        return (file_stem.into(), -1, -1);
    }

    let show = entries[..(entries.len() - 2)].join("_").into();

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
        (file_stem.into(), -1, -1)
    } else {
        (show, season, episode)
    }
}

pub async fn get_video_runtime(f: &Path) -> Result<StackString, Error> {
    let ext = f
        .extension()
        .ok_or_else(|| format_err!("No extension"))?
        .to_string_lossy();
    let fname = f.to_string_lossy();

    let (command, args) = if ext == ".avi" {
        ("aviindex", vec!["-i", fname.as_ref(), "-o", "/dev/null"])
    } else {
        ("ffprobe", vec![fname.as_ref()])
    };

    let mut timeval = "".into();

    let output = Command::new(command).args(&args).output().await?;
    for l in output
        .stdout
        .split(|c| *c == b'\n')
        .chain(output.stderr.split(|c| *c == b'\n'))
    {
        let line = std::str::from_utf8(&l)?;
        let items: SmallVec<[&str; 6]> = line.split_whitespace().take(6).collect();
        if items.len() > 5 && items[1] == "V:" {
            let fps: f64 = items[2].parse()?;
            let nframes: u64 = items[5]
                .trim_start_matches("frames=")
                .trim_matches(',')
                .parse()?;
            let nsecs: f64 = nframes as f64 / fps;
            let nmin = (nsecs / 60.) as u64;
            let nhour = (nmin as f64 / 60.) as u64;
            timeval = format!("{:02}:{:02}:{:02}", nhour, nmin % 60, nsecs as u64 % 60).into();
        }
        if items.len() > 1 && items[0] == "Duration:" {
            let its: SmallVec<[&str; 3]> = items[1].trim_matches(',').split(':').take(3).collect();
            let nhour: u64 = its[0].parse()?;
            let nmin: u64 = its[1].parse()?;
            let nsecs: f64 = its[2].parse()?;
            timeval = format!("{:02}:{:02}:{:02}", nhour, nmin, nsecs as u64).into();
        }
    }
    Ok(timeval)
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
