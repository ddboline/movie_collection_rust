use anyhow::{format_err, Error};
use derive_more::Display;
use futures::{future::try_join_all, TryStreamExt};
use itertools::Itertools;
use stack_string::{format_sstr, StackString};
use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
};
use stdout_channel::StdoutChannel;

use crate::{
    config::Config, movie_collection::MovieCollection, movie_queue::MovieQueueDB, pgpool::PgPool,
    utils::get_video_runtime,
};

#[derive(Debug, Display, Clone)]
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

/// # Errors
/// Return error on file system errors
#[allow(clippy::cognitive_complexity)]
pub async fn make_queue_worker(
    config: &Config,
    add_files: &[PathOrIndex],
    del_files: &[PathOrIndex],
    do_time: bool,
    patterns: &[&str],
    do_shows: bool,
    stdout: &StdoutChannel<StackString>,
) -> Result<(), Error> {
    let pool = PgPool::new(&config.pgurl);
    let mc = MovieCollection::new(config, &pool, stdout);
    let mq = MovieQueueDB::new(config, &pool, stdout);

    if do_shows {
        let shows: Vec<_> = mc
            .print_tv_shows()
            .await?
            .map_ok(StackString::from_display)
            .try_collect()
            .await?;
        let shows = shows.join("\n");
        stdout.send(shows);
    } else if !del_files.is_empty() {
        for file in del_files {
            match file {
                PathOrIndex::Index(idx) => mq.remove_from_queue_by_idx(*idx).await?,
                PathOrIndex::Path(path) => {
                    mq.remove_from_queue_by_path(&path.to_string_lossy())
                        .await?;
                }
            };
        }
    } else if add_files.is_empty() {
        let movie_queue = mq.print_movie_queue(patterns, None, None, None).await?;
        if do_time {
            let futures = movie_queue.into_iter().map(|result| async move {
                let path = Path::new(result.path.as_str());
                let timeval = get_video_runtime(path).await?;
                Ok(format_sstr!("{result} {timeval}"))
            });
            let results: Result<Vec<_>, Error> = try_join_all(futures).await;
            stdout.send(results?.join("\n"));
        } else {
            stdout.send(
                movie_queue
                    .into_iter()
                    .map(StackString::from_display)
                    .join("\n"),
            );
        }
    } else if add_files.len() == 1 {
        let max_idx = mq.get_max_queue_index().await?;
        if let PathOrIndex::Path(path) = &add_files[0] {
            mq.insert_into_queue(max_idx + 1, &path.to_string_lossy())
                .await?;
        } else {
            return Err(format_err!("No file specified"));
        }
    } else if add_files.len() == 2 {
        if let PathOrIndex::Index(idx) = &add_files[0] {
            stdout.send(format_sstr!("inserting into {idx}"));
            if let PathOrIndex::Path(path) = &add_files[1] {
                mq.insert_into_queue(*idx, &path.to_string_lossy()).await?;
            } else {
                return Err(format_err!("{} is not a path", add_files[1]));
            }
        } else {
            for file in add_files {
                let max_idx = mq.get_max_queue_index().await?;
                if let PathOrIndex::Path(path) = file {
                    mq.insert_into_queue(max_idx + 1, &path.to_string_lossy())
                        .await?;
                } else {
                    return Err(format_err!("{} is not a path", file));
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
                return Err(format_err!("{} is not a path", file));
            }
        }
    }

    Ok(())
}
