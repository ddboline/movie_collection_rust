use anyhow::{format_err, Error};
use async_trait::async_trait;
use futures::Stream;
use jwalk::WalkDir;
use log::error;
use postgres_query::{query, Error as PqError, FromSqlRow};
use rand::{
    distr::{Distribution, Uniform},
    rng as thread_rng,
};
use reqwest::{Client, Response, Url};
use serde::{Deserialize, Serialize};
use smallvec::SmallVec;
use stack_string::{format_sstr, StackString};
use std::{
    convert::TryFrom,
    path::{Path, PathBuf},
};
use time::OffsetDateTime;
use tokio::{
    process::Command,
    time::{sleep, Duration},
};

use crate::pgpool::PgPool;

#[inline]
#[allow(clippy::needless_lifetimes)]
pub fn option_string_wrapper<'a>(s: Option<&'a impl AsRef<str>>) -> &'a str {
    s.map_or("", AsRef::as_ref)
}

/// # Errors
/// Return error if db query fails
pub fn walk_directory(path: &Path, match_strs: &[impl AsRef<str>]) -> Result<Vec<PathBuf>, Error> {
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
                    Some(Ok(path))
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

#[must_use]
pub fn parse_file_stem(file_stem: &str) -> (StackString, i32, i32) {
    let entries: Vec<_> = file_stem.split('_').collect();

    if entries.len() < 3 {
        return (file_stem.into(), -1, -1);
    }

    let show = entries[..(entries.len() - 2)].join("_").into();

    let season = entries[entries.len() - 2];
    let season: i32 = if season.starts_with('s') {
        season.replace('s', "").parse().unwrap_or(-1)
    } else {
        -1
    };

    let episode = entries[entries.len() - 1];
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

/// # Errors
/// Return error if db query fails
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
        let line = std::str::from_utf8(l)?;
        let items: SmallVec<[&str; 6]> = line.split_whitespace().take(6).collect();
        if items.len() > 5 && items[1] == "V:" {
            let fps: f64 = items[2].parse()?;
            let nframes: u64 = items[5]
                .trim_start_matches("frames=")
                .trim_matches(',')
                .parse()?;
            let nsecs = nframes as f64 / fps;
            let nmin = (nsecs / 60.) as u64 % 60;
            let nhour = (nmin as f64 / 60.) as u64;
            let nsecs = nsecs as u64 % 60;
            timeval = format_sstr!("{nhour:02}:{nmin:02}:{nsecs:02}");
        }
        if items.len() > 1 && items[0] == "Duration:" {
            let its: SmallVec<[&str; 3]> = items[1].trim_matches(',').split(':').take(3).collect();
            let nhour: u64 = its[0].parse()?;
            let nmin: u64 = its[1].parse()?;
            let nsecs: f64 = its[2].parse()?;
            let nsecs = nsecs as u64;
            timeval = format_sstr!("{nhour:02}:{nmin:02}:{nsecs:02}");
        }
    }
    Ok(timeval)
}

#[async_trait]
pub trait ExponentialRetry {
    fn get_client(&self) -> &Client;

    async fn get(&self, url: &Url) -> Result<Response, Error> {
        let mut timeout: f64 = 1.0;
        let range = Uniform::try_from(0..1000)?;
        loop {
            match self.get_client().get(url.clone()).send().await {
                Ok(resp) => return Ok(resp),
                Err(err) => {
                    error!("got error {err}");
                    sleep(Duration::from_millis((timeout * 1000.0) as u64)).await;
                    timeout *= 4.0 * f64::from(range.sample(&mut thread_rng())) / 1000.0;
                    if timeout >= 64.0 {
                        return Err(err.into());
                    }
                }
            }
        }
    }
}

#[derive(FromSqlRow, Clone, Debug)]
pub struct AuthorizedUsers {
    pub email: StackString,
    pub created_at: OffsetDateTime,
}

impl AuthorizedUsers {
    /// # Errors
    /// Return error if db query fails
    pub async fn get_authorized_users(
        pool: &PgPool,
    ) -> Result<impl Stream<Item = Result<Self, PqError>>, Error> {
        let query = query!("SELECT * FROM authorized_users WHERE deleted_at IS NULL");
        let conn = pool.get().await?;
        query.fetch_streaming(&conn).await.map_err(Into::into)
    }

    /// # Errors
    /// Returns error if db query fails
    pub async fn get_most_recent(
        pool: &PgPool,
    ) -> Result<(Option<OffsetDateTime>, Option<OffsetDateTime>), Error> {
        #[derive(FromSqlRow)]
        struct CreatedDeleted {
            created_at: Option<OffsetDateTime>,
            deleted_at: Option<OffsetDateTime>,
        }

        let query = query!(
            "SELECT max(created_at) as created_at, max(deleted_at) as deleted_at FROM \
             authorized_users"
        );
        let conn = pool.get().await?;
        let result: Option<CreatedDeleted> = query.fetch_opt(&conn).await?;
        match result {
            Some(result) => Ok((result.created_at, result.deleted_at)),
            None => Ok((None, None)),
        }
    }
}
