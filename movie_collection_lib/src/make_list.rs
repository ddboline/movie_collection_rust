use anyhow::Error;
use futures::future::join_all;
use itertools::Itertools;
use log::debug;
use rayon::iter::{IntoParallelRefIterator, ParallelIterator};
use stack_string::StackString;
use std::{collections::HashMap, ffi::OsStr, fmt::Write, path::PathBuf};
use stdout_channel::StdoutChannel;
use tokio::{
    fs,
    task::{spawn, spawn_blocking},
};
use tokio_stream::{wrappers::ReadDirStream, StreamExt};

use crate::{
    config::Config,
    transcode_service::transcode_status,
    utils::{get_video_runtime, walk_directory},
};

#[derive(Default)]
pub struct FileLists {
    pub local_file_list: Vec<StackString>,
    pub file_list: Vec<PathBuf>,
}

impl FileLists {
    pub async fn get_file_lists(config: &Config) -> Result<Self, Error> {
        let movies_dir = config.home_dir.join("Documents").join("movies");

        let mut local_file_list: Vec<StackString> =
            ReadDirStream::new(fs::read_dir(movies_dir).await?)
                .filter_map(|f| {
                    let fname = f.ok()?;
                    let file_name = fname.file_name().to_string_lossy().into_owned();
                    for suffix in &config.suffixes {
                        if file_name.ends_with(suffix.as_str()) {
                            return Some(file_name.into());
                        }
                    }
                    None
                })
                .collect()
                .await;

        if local_file_list.is_empty() {
            return Ok(Self::default());
        }

        local_file_list.sort();

        let patterns: Vec<_> = local_file_list
            .iter()
            .map(|f| {
                f.replace(".mkv", "")
                    .replace(".avi", "")
                    .replace(".mp4", "")
            })
            .collect();

        let config = config.clone();
        let file_list: Result<Vec<_>, Error> = spawn_blocking(move || {
            config
                .movie_dirs
                .par_iter()
                .filter(|d| d.exists())
                .map(|d| walk_directory(d, &patterns))
                .collect::<Result<Vec<_>, Error>>()
                .map(|x| x.into_iter().flatten().sorted().collect())
        })
        .await?;
        let file_list = file_list?;

        Ok(Self {
            local_file_list,
            file_list,
        })
    }

    pub fn get_file_map(&self) -> HashMap<StackString, &PathBuf> {
        self.file_list
            .iter()
            .map(|f| {
                let file_name = f
                    .file_stem()
                    .unwrap_or_else(|| OsStr::new(""))
                    .to_string_lossy()
                    .to_string()
                    .into();
                (file_name, f)
            })
            .collect()
    }
}

pub async fn make_list(stdout: &StdoutChannel<StackString>) -> Result<(), Error> {
    let config = Config::with_config()?;
    let transcode_task = {
        let config = config.clone();
        spawn(async move { transcode_status(&config).await })
    };

    let file_lists = FileLists::get_file_lists(&config).await?;
    let file_map = file_lists.get_file_map();

    if file_lists.local_file_list.is_empty() {
        return Ok(());
    }

    let proc_map = transcode_task.await??.get_proc_map();
    debug!("{:?}", proc_map);

    for f in &file_lists.local_file_list {
        let mut f_key = f.as_str();
        if let Some(s) = f_key.strip_suffix(".mkv") {
            f_key = s;
        }
        if let Some(s) = f_key.strip_suffix(".avi") {
            f_key = s;
        }
        if let Some(s) = f_key.strip_suffix(".mp4") {
            f_key = s;
        }
        let mut fout = StackString::new();
        if let Some(full_path) = file_map.get(f_key) {
            write!(fout, "{} {}", f, full_path.to_string_lossy()).unwrap();
        } else if let Some(Some(status)) = proc_map.get(f_key) {
            write!(fout, "{} {}", f, status).unwrap();
        } else {
            write!(fout, "{}", f).unwrap();
        }
        stdout.send(fout);
    }

    let futures = file_lists.file_list.iter().map(|f| async move {
        let timeval = get_video_runtime(f).await.unwrap_or_else(|_| "".into());
        let mut buf = StackString::new();
        write!(buf, "{} {}", timeval, f.to_string_lossy()).unwrap();
        buf
    });

    for e in join_all(futures).await {
        stdout.send(e);
    }

    Ok(())
}
