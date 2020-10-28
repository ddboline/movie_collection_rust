use anyhow::{format_err, Error};
use derive_more::Display;
use futures::future::try_join_all;
use itertools::Itertools;
use stack_string::StackString;
use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
    sync::Arc,
};

use crate::{
    config::Config,
    movie_collection::MovieCollection,
    movie_queue::{MovieQueueDB, MovieQueueResult},
    pgpool::PgPool,
    stdout_channel::StdoutChannel,
    utils::{get_video_runtime, parse_file_stem},
};

#[derive(Debug, Display)]
pub enum PathOrIndex {
    #[display(fmt = "{:?}", _0)]
    Path(PathBuf),
    Index(i32),
}

impl From<&OsStr> for PathOrIndex {
    fn from(s: &OsStr) -> Self {
        if let Some(Ok(idx)) = s.to_str().map(str::parse::<i32>) {
            Self::Index(idx)
        } else {
            Self::Path(s.to_os_string().into())
        }
    }
}

impl From<i32> for PathOrIndex {
    fn from(i: i32) -> Self {
        Self::Index(i)
    }
}

impl From<&Path> for PathOrIndex {
    fn from(p: &Path) -> Self {
        Self::Path(p.to_path_buf())
    }
}

#[allow(clippy::cognitive_complexity)]
pub async fn make_queue_worker(
    config: &Config,
    add_files: &[PathOrIndex],
    del_files: &[PathOrIndex],
    do_time: bool,
    patterns: &[&str],
    do_shows: bool,
    stdout: &StdoutChannel,
) -> Result<(), Error> {
    let mc = MovieCollection::new(config.clone());
    let mq = MovieQueueDB::with_pool(&mc.pool);

    if do_shows {
        let shows = mc
            .print_tv_shows()
            .await?
            .into_iter()
            .map(|s| s.to_string())
            .join("\n");
        stdout.send(shows);
    } else if !del_files.is_empty() {
        for file in del_files {
            match file {
                PathOrIndex::Index(idx) => mq.remove_from_queue_by_idx(*idx).await?,
                PathOrIndex::Path(path) => {
                    mq.remove_from_queue_by_path(&path.to_string_lossy())
                        .await?
                }
            };
        }
    } else if add_files.is_empty() {
        let movie_queue = mq.print_movie_queue(&patterns).await?;
        if do_time {
            let futures = movie_queue.into_iter().map(|result| async move {
                let path = Path::new(result.path.as_str());
                let timeval = get_video_runtime(path).await?;
                Ok(format!("{} {}", result, timeval))
            });
            let results: Result<Vec<_>, Error> = try_join_all(futures).await;
            stdout.send(results?.join("\n"));
        } else {
            stdout.send(movie_queue.into_iter().map(|x| x.to_string()).join("\n"));
        }
    } else if add_files.len() == 1 {
        let max_idx = mq.get_max_queue_index().await?;
        if let PathOrIndex::Path(path) = &add_files[0] {
            mq.insert_into_queue(max_idx + 1, &path.to_string_lossy())
                .await?;
        } else {
            panic!("No file specified");
        }
    } else if add_files.len() == 2 {
        if let PathOrIndex::Index(idx) = &add_files[0] {
            stdout.send(format!("inserting into {}", idx));
            if let PathOrIndex::Path(path) = &add_files[1] {
                mq.insert_into_queue(*idx, &path.to_string_lossy()).await?;
            } else {
                panic!("{} is not a path", add_files[1]);
            }
        } else {
            for file in add_files {
                let max_idx = mq.get_max_queue_index().await?;
                if let PathOrIndex::Path(path) = file {
                    mq.insert_into_queue(max_idx + 1, &path.to_string_lossy())
                        .await?;
                } else {
                    panic!("{} is not a path", file);
                }
            }
        }
    } else {
        for file in add_files {
            let max_idx = mq.get_max_queue_index().await?;
            if let PathOrIndex::Path(path) = file {
                mq.insert_into_queue(max_idx + 1, &path.to_string_lossy())
                    .await?;
            } else {
                panic!("{} is not a path", file);
            }
        }
    }

    Ok(())
}

pub async fn movie_queue_http(
    queue: &[MovieQueueResult],
    pool: &PgPool,
) -> Result<Vec<StackString>, Error> {
    let mc = Arc::new(MovieCollection::with_pool(pool)?);

    let button = r#"<td><button type="submit" id="ID" onclick="delete_show('SHOW');"> remove </button></td>"#;

    let futures = queue.iter().map(|row| {
        let mc = mc.clone();
        async move {
        let path = Path::new(row.path.as_str());
        let ext = path
            .extension()
            .ok_or_else(|| format_err!("Cannot determine extension"))?
            .to_string_lossy();
        let file_name = path
            .file_name()
            .ok_or_else(|| format_err!("Invalid path"))?
            .to_string_lossy()
            .to_string();
        let file_stem = path
            .file_stem()
            .ok_or_else(|| format_err!("Invalid path"))?
            .to_string_lossy();
        let (_, season, episode) = parse_file_stem(&file_stem);

        let entry = if ext == "mp4" {
            let collection_idx = mc.get_collection_index(&row.path).await?.unwrap_or(-1);
            format!(
                r#"<a href="javascript:updateMainArticle('{}');">{}</a>"#,
                &format!("{}/{}", "/list/play", collection_idx),
                file_name
            )
        } else {
            file_name.clone()
        };

        let entry = if let Some(link) = row.link.as_ref() {
            format!(
                r#"<tr><td>{}</td><td><a href={} target="_blank">imdb</a></td>"#,
                entry,
                &format!("https://www.imdb.com/title/{}", link)
            )
        } else {
            format!("<tr>\n<td>{}</td>\n", entry)
        };

        let entry = format!(
            "{}\n{}",
            entry,
            button.replace("ID", &file_name).replace("SHOW", &file_name)
        ).into();

        let entry = if ext == "mp4" {
            entry
        } else if season != -1 && episode != -1 {
            format!(
                r#"{entry}<td><button type="submit" id="{file_name}" onclick="transcode_queue('{file_name}');"> transcode </button></td>"#,
                entry=entry, file_name=file_name
            ).into()
        } else {
            let entries: Vec<_> = row.path.split('/').collect();
            let len_entries = entries.len();
            let directory = entries[len_entries - 2];
            format!(
                r#"{entry}<td><button type="submit" id="{file_name}" onclick="transcode_queue_directory('{file_name}', '{directory}');"> transcode </button></td>"#,
                entry=entry, file_name=file_name, directory=directory
            ).into()
        };
        Ok(entry)
        }
    });
    try_join_all(futures).await
}
