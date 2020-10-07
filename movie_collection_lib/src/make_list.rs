use anyhow::Error;
use futures::future::join_all;
use log::debug;
use rayon::{
    iter::{IntoParallelRefIterator, ParallelIterator},
    slice::ParallelSliceMut,
};
use stack_string::StackString;
use std::{collections::HashMap, path::PathBuf};
use tokio::task::{spawn, spawn_blocking};

use crate::{
    config::Config,
    stdout_channel::StdoutChannel,
    transcode_service::transcode_status,
    utils::{get_video_runtime, walk_directory},
};

#[derive(Default)]
pub struct FileLists {
    pub local_file_list: Vec<StackString>,
    pub file_list: Vec<PathBuf>,
}

impl FileLists {
    pub fn get_file_lists(config: &Config) -> Result<Self, Error> {
        use std::fs;

        let movies_dir = config.home_dir.join("Documents").join("movies");

        let mut local_file_list: Vec<_> = fs::read_dir(movies_dir)?
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
            .collect();

        if local_file_list.is_empty() {
            return Ok(Self::default());
        }

        local_file_list.sort();

        let file_list: Result<Vec<_>, Error> = config
            .movie_dirs
            .par_iter()
            .filter(|d| d.exists())
            .map(|d| walk_directory(&d, &local_file_list))
            .collect();

        let mut file_list: Vec<_> = file_list?.into_iter().flatten().collect();

        file_list.par_sort();

        Ok(Self {
            local_file_list,
            file_list,
        })
    }

    pub fn get_file_map(&self) -> HashMap<StackString, &PathBuf> {
        self.file_list
            .iter()
            .map(|f| {
                let file_name = f.file_name().unwrap().to_string_lossy().to_string().into();
                (file_name, f)
            })
            .collect()
    }
}

pub async fn make_list(stdout: &StdoutChannel) -> Result<(), Error> {
    let config = Config::with_config()?;
    let transcode_task = {
        let config = config.clone();
        spawn(async move { transcode_status(&config).await })
    };

    let file_lists = spawn_blocking(move || FileLists::get_file_lists(&config)).await??;
    let file_map = file_lists.get_file_map();

    if file_lists.local_file_list.is_empty() {
        return Ok(());
    }

    let proc_map = transcode_task.await??.get_proc_map();
    debug!("{:?}", proc_map);

    file_lists
        .local_file_list
        .iter()
        .map(|f| {
            if let Some(full_path) = file_map.get(f.as_str()) {
                format!("{} {}", f, full_path.to_string_lossy())
            } else if let Some(Some(status)) = proc_map.get(f.as_str()) {
                format!("{} {}", f, status)
            } else {
                f.to_string()
            }
        })
        .for_each(|e| stdout.send(e));

    let futures = file_lists.file_list.iter().map(|f| async move {
        let timeval = get_video_runtime(f).await.unwrap_or_else(|_| "".into());
        format!("{} {}", timeval, f.to_string_lossy())
    });

    for e in join_all(futures).await {
        stdout.send(e);
    }

    Ok(())
}
