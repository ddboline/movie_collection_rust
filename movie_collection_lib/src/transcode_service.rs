use anyhow::{format_err, Error};
use futures::{future::try_join_all, try_join};
use itertools::Itertools;
use jwalk::WalkDir;
use procfs::process;
use serde::{Deserialize, Serialize};
use smallvec::{smallvec, SmallVec};
use stack_string::{format_sstr, StackString};
use std::{
    collections::HashMap,
    ffi::OsStr,
    fmt,
    fmt::Write,
    future::Future,
    path::{Path, PathBuf},
    process::Stdio,
    str,
};
use stdout_channel::StdoutChannel;
use tokio::{
    fs::{self, File, OpenOptions},
    io::{AsyncBufReadExt, AsyncRead, AsyncWriteExt, BufReader},
    process::Command,
    task::{spawn, spawn_blocking, JoinHandle},
};

use crate::{
    config::Config, make_queue::make_queue_worker, movie_collection::MovieCollection,
    pgpool::PgPool, utils::parse_file_stem,
};

#[derive(Serialize, Deserialize, Debug, PartialEq, Clone, Copy, Eq)]
pub enum JobType {
    Transcode,
    Move,
}

impl JobType {
    fn to_str(self) -> &'static str {
        match self {
            Self::Transcode => "transcode",
            Self::Move => "move",
        }
    }
}

impl fmt::Display for JobType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_str())
    }
}

#[derive(Serialize, Deserialize, Debug, PartialEq, Clone, Eq)]
pub struct TranscodeServiceRequest {
    pub job_type: JobType,
    pub prefix: StackString,
    pub input_path: PathBuf,
    pub output_path: PathBuf,
}

impl fmt::Display for TranscodeServiceRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{job_type}\t{prefix}\t{input_path}\t{output_path}",
            job_type = self.job_type,
            prefix = self.prefix,
            input_path = self
                .input_path
                .file_name()
                .unwrap_or_else(|| OsStr::new(""))
                .to_string_lossy(),
            output_path = self
                .output_path
                .file_name()
                .unwrap_or_else(|| OsStr::new(""))
                .to_string_lossy()
        )
    }
}

impl TranscodeServiceRequest {
    #[must_use]
    pub fn get_header() -> SmallVec<[&'static str; 4]> {
        smallvec!["Job Type", "Prefix", "Input Path", "Output Path"]
    }

    #[must_use]
    pub fn new(job_type: JobType, prefix: &str, input_path: &Path, output_path: &Path) -> Self {
        Self {
            job_type,
            prefix: prefix.into(),
            input_path: input_path.to_path_buf(),
            output_path: output_path.to_path_buf(),
        }
    }

    /// # Errors
    /// Return error if db query fails
    pub fn create_transcode_request(config: &Config, input_path: &Path) -> Result<Self, Error> {
        let input_path = input_path.to_path_buf();
        let fstem = input_path
            .file_stem()
            .ok_or_else(|| format_err!("No stem"))?;
        let output_file = avi_dir(config).join(fstem).with_extension("mp4");
        let prefix = fstem.to_string_lossy().into_owned().into();

        Ok(Self {
            job_type: JobType::Transcode,
            prefix,
            input_path,
            output_path: output_file,
        })
    }

    /// # Errors
    /// Return error if db query fails
    pub async fn create_remcom_request(
        config: &Config,
        path: impl AsRef<Path>,
        directory: Option<impl AsRef<Path>>,
        unwatched: bool,
    ) -> Result<Self, Error> {
        let path = path.as_ref();
        let ext = path
            .extension()
            .ok_or_else(|| format_err!("no extension"))?
            .to_string_lossy();
        let file_stem = path.file_stem().expect("No file stem");
        if ext == "mp4" {
            let prefix = file_stem.to_string_lossy().into_owned().into();
            let output_dir = if let Some(d) = directory {
                let d = config
                    .preferred_dir
                    .join("Documents")
                    .join("movies")
                    .join(d);
                println!("{}", d.to_string_lossy());
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
                let file_stem = file_stem.to_string_lossy();

                let (show, season, episode) = parse_file_stem(&file_stem);

                if season == -1 || episode == -1 {
                    return Err(format_err!(
                        "Failed to parse show season {season} episode {episode}"
                    ));
                }

                let d = config
                    .preferred_dir
                    .join("Documents")
                    .join("television")
                    .join(show.as_str())
                    .join(format_sstr!("season{season}"));
                if !d.exists() {
                    fs::create_dir_all(&d).await?;
                }
                d
            };

            let input_path = path.to_path_buf();
            let output_path = output_dir.join(&format_sstr!("{prefix}.mp4"));

            Ok(Self {
                job_type: JobType::Move,
                prefix,
                input_path,
                output_path,
            })
        } else {
            Self::create_transcode_request(config, path)
        }
    }

    #[must_use]
    pub fn get_html(&self) -> Vec<StackString> {
        vec![
            self.job_type.to_str().into(),
            self.prefix.clone(),
            self.input_path
                .file_name()
                .unwrap_or_else(|| OsStr::new(""))
                .to_string_lossy()
                .into_owned()
                .into(),
            self.output_path
                .file_name()
                .unwrap_or_else(|| OsStr::new(""))
                .to_string_lossy()
                .into_owned()
                .into(),
        ]
    }

    #[must_use]
    pub fn get_cmd_path(&self) -> PathBuf {
        match self.job_type {
            JobType::Transcode => Path::new("/usr/bin/transcode-avi").to_path_buf(),
            JobType::Move => Path::new("/usr/bin/remcom").to_path_buf(),
        }
    }

    #[must_use]
    pub fn get_json_path(&self, config: &Config) -> PathBuf {
        let prefix = &self.prefix;
        match self.job_type {
            JobType::Transcode => job_dir(config).join(prefix).with_extension("json"),
            JobType::Move => job_dir(config)
                .join(&format_sstr!("{prefix}_copy"))
                .with_extension("json"),
        }
    }

    /// # Errors
    /// Return error if db query fails
    pub async fn publish_to_cli(&self, config: &Config) -> Result<StackString, Error> {
        let cmd_path = self.get_cmd_path();
        let json_path = self.get_json_path(config);
        if cmd_path.exists() {
            let output = Command::new(&cmd_path)
                .args(["-f", json_path.to_string_lossy().as_ref()])
                .output()
                .await?;
            StackString::from_utf8_vec(output.stdout).map_err(Into::into)
        } else {
            Err(format_err!("{cmd_path:?} does not exist"))
        }
    }
}

#[derive(Clone)]
pub struct TranscodeService {
    pub config: Config,
    pub pool: PgPool,
    pub stdout: StdoutChannel<StackString>,
    pub queue: StackString,
}

impl TranscodeService {
    #[must_use]
    pub fn new(
        config: &Config,
        queue: &str,
        pool: &PgPool,
        stdout: &StdoutChannel<StackString>,
    ) -> Self {
        Self {
            config: config.clone(),
            pool: pool.clone(),
            stdout: stdout.clone(),
            queue: queue.into(),
        }
    }

    /// # Errors
    /// Return error if db query fails
    pub async fn publish_transcode_job<F, T>(
        &self,
        payload: &TranscodeServiceRequest,
        publish: F,
    ) -> Result<(), Error>
    where
        F: Fn(Vec<u8>) -> T,
        T: Future<Output = Result<(), Error>>,
    {
        fs::write(
            &payload.get_json_path(&self.config),
            &serde_json::to_vec(&payload)?,
        )
        .await?;
        let payload = serde_json::to_vec(&payload)?;
        publish(payload).await
    }

    /// # Errors
    /// Return error if db query fails
    pub async fn process_data(&self, data: &[u8]) -> Result<(), Error> {
        let payload: TranscodeServiceRequest = serde_json::from_slice(data)?;
        match payload.job_type {
            JobType::Transcode => {
                self.run_transcode(&payload.prefix, &payload.input_path, &payload.output_path)
                    .await
            }
            JobType::Move => {
                self.run_move(&payload.prefix, &payload.input_path, &payload.output_path)
                    .await
            }
        }
    }

    async fn output_to_file<T>(
        mut reader: BufReader<T>,
        output_path: &Path,
        eol: u8,
    ) -> Result<(), Error>
    where
        T: AsyncRead + Unpin,
    {
        let mut f = File::create(&output_path).await?;
        let mut buf = Vec::new();
        while let Ok(bytes) = reader.read_until(eol, &mut buf).await {
            if bytes > 0 {
                f.write_all(&buf).await?;
            } else {
                break;
            }
            buf.clear();
        }
        Ok(())
    }

    async fn run_transcode(
        &self,
        prefix: &str,
        input_file: &Path,
        output_file: &Path,
    ) -> Result<(), Error> {
        let script_file = job_dir(&self.config).join(prefix).with_extension("json");
        if script_file.exists() {
            fs::remove_file(&script_file).await?;
        }

        if !input_file.exists() {
            return Err(format_err!("{input_file:?} does not exist"));
        }
        let output_path = output_file
            .file_name()
            .ok_or_else(|| format_err!("No Output File"))?;
        let output_path = self
            .config
            .home_dir
            .join("Documents")
            .join("movies")
            .join(output_path);
        let debug_output_path = log_dir(&self.config).join(&format_sstr!("{prefix}_mp4"));
        let stdout_path = debug_output_path.with_extension("out");
        let stderr_path = debug_output_path.with_extension("err");

        let mut p = Command::new("HandBrakeCLI")
            .args([
                "-i",
                input_file.to_string_lossy().as_ref(),
                "-o",
                output_file.to_string_lossy().as_ref(),
                "--preset",
                "Android 480p30",
            ])
            .kill_on_drop(true)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let stdout = p.stdout.take().ok_or_else(|| format_err!("No Stdout"))?;
        let stderr = p.stderr.take().ok_or_else(|| format_err!("No Stderr"))?;

        let reader = BufReader::new(stdout);
        let stdout_task: JoinHandle<Result<(), Error>> =
            spawn(async move { Self::output_to_file(reader, &stdout_path, b'\r').await });

        let reader = BufReader::new(stderr);
        let stderr_task: JoinHandle<Result<(), Error>> =
            spawn(async move { Self::output_to_file(reader, &stderr_path, b'\n').await });

        let status = p.wait().await?;
        println!("Handbrake exited with {status}");
        stdout_task.await??;
        stderr_task.await??;

        if output_file.exists() && fs::rename(&output_file, &output_path).await.is_err() {
            fs::copy(&output_file, &output_path).await?;
            fs::remove_file(&output_file).await?;
        }

        let tmp_avi_path = tmp_dir(&self.config);
        let stdout_path = debug_output_path.with_extension("out");
        let stderr_path = debug_output_path.with_extension("err");
        if stdout_path.exists() && stderr_path.exists() {
            if let Ok(mut f) = OpenOptions::new().append(true).open(&stderr_path).await {
                if let Ok(stdout) = fs::read(&stdout_path).await {
                    f.write_all(b"\n").await?;
                    f.write_all(&stdout).await?;
                }
            }
            let new_debug_output_path = tmp_avi_path.join(&format_sstr!("{prefix}_mp4.out"));
            fs::rename(&stderr_path, &new_debug_output_path).await?;
            fs::remove_file(&stdout_path).await?;
        }
        Ok(())
    }

    async fn run_move(
        &self,
        show: &str,
        input_file: &Path,
        output_file: &Path,
    ) -> Result<(), Error> {
        let script_file = job_dir(&self.config)
            .join(&format_sstr!("{show}_copy"))
            .with_extension("json");
        if script_file.exists() {
            fs::remove_file(&script_file).await?;
        }

        let input_file = if input_file.exists() {
            input_file.to_path_buf()
        } else {
            self.config
                .home_dir
                .join("Documents")
                .join("movies")
                .join(input_file)
        };
        if !input_file.exists() {
            return Err(format_err!("{input_file:?} does not exist"));
        }

        let debug_output_path = log_dir(&self.config).join(&format_sstr!("{show}_copy.out"));
        let mut debug_output_file = File::create(&debug_output_path).await?;

        let show_path = self
            .config
            .home_dir
            .join("Documents")
            .join("movies")
            .join(&format_sstr!("{show}.mp4"));
        if !show_path.exists() {
            return Ok(());
        }
        let new_path = output_file.with_extension("new");
        let task0 = spawn({
            let new_path = new_path.clone();
            let buf = format_sstr!(
                "copy {} to {}\n",
                show_path.to_string_lossy(),
                new_path.to_string_lossy()
            );
            debug_output_file.write_all(buf.as_bytes()).await?;
            async move { fs::copy(&show_path, &new_path).await }
        });
        if output_file.exists() {
            let old_path = output_file.with_extension("old");
            let mut buf = StackString::new();
            writeln!(
                buf,
                "copy {} to {}",
                output_file.to_string_lossy(),
                old_path.to_string_lossy()
            )
            .unwrap();
            debug_output_file.write_all(buf.as_bytes()).await?;
            fs::rename(&output_file, &old_path).await?;
        }
        task0.await??;
        let mut buf = StackString::new();
        writeln!(
            buf,
            "copy {} to {}",
            new_path.to_string_lossy(),
            output_file.to_string_lossy()
        )
        .unwrap();
        debug_output_file.write_all(buf.as_bytes()).await?;
        fs::rename(&new_path, &output_file).await?;
        make_queue_worker(
            &self.config,
            &[],
            &[output_file.into()],
            false,
            &[],
            false,
            &self.stdout,
        )
        .await?;
        let mut buf = StackString::new();
        writeln!(buf, "add {} to queue", output_file.to_string_lossy()).unwrap();
        debug_output_file.write_all(buf.as_bytes()).await?;
        make_queue_worker(
            &self.config,
            &[output_file.into()],
            &[],
            false,
            &[],
            false,
            &self.stdout,
        )
        .await?;
        let mc = MovieCollection::new(&self.config, &self.pool, &self.stdout);
        debug_output_file.write_all(b"update collection\n").await?;
        mc.make_collection().await?;
        mc.fix_collection_show_id().await?;

        debug_output_file.flush().await?;

        if debug_output_path.exists() {
            let new_debug_output_path =
                tmp_dir(&self.config).join(&format_sstr!("{show}_copy.out"));
            fs::rename(&debug_output_path, &new_debug_output_path).await?;
        }

        Ok(())
    }
}

#[must_use]
pub fn movie_dir(config: &Config) -> PathBuf {
    config.home_dir.join("Documents").join("movies")
}

#[must_use]
fn dvdrip_dir(config: &Config) -> PathBuf {
    config.home_dir.join("dvdrip")
}

#[must_use]
fn avi_dir(config: &Config) -> PathBuf {
    dvdrip_dir(config).join("avi")
}

#[must_use]
fn log_dir(config: &Config) -> PathBuf {
    dvdrip_dir(config).join("log")
}

#[must_use]
pub fn job_dir(config: &Config) -> PathBuf {
    dvdrip_dir(config).join("jobs")
}

#[must_use]
fn tmp_dir(config: &Config) -> PathBuf {
    config.home_dir.join("tmp_avi")
}

#[derive(Debug, PartialEq, Eq)]
pub struct ProcInfo {
    pub pid: u64,
    pub name: StackString,
    pub exe: PathBuf,
    pub cmdline: Vec<StackString>,
}

impl ProcInfo {
    #[must_use]
    pub fn get_header() -> SmallVec<[&'static str; 4]> {
        smallvec!["Pid", "Name", "Exe Path", "Cmdline Args"]
    }

    #[must_use]
    pub fn get_html(&self) -> Vec<StackString> {
        vec![
            StackString::from_display(self.pid),
            self.name.clone(),
            self.exe.to_string_lossy().into_owned().into(),
            self.cmdline.join(" ").into(),
        ]
    }
}

impl fmt::Display for ProcInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{pid}\t{name}\t{exe}\t{cmdline}",
            pid = self.pid,
            name = self.name,
            exe = self.exe.to_string_lossy(),
            cmdline = self.cmdline.join(" "),
        )
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct TranscodeStatus {
    pub procs: Vec<ProcInfo>,
    pub upcoming_jobs: Vec<TranscodeServiceRequest>,
    pub current_jobs: Vec<(PathBuf, StackString)>,
    pub finished_jobs: Vec<PathBuf>,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ProcStatus {
    Upcoming,
    Current,
    Finished,
}

impl fmt::Display for ProcStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Self::Upcoming => "upcoming",
            Self::Current => "current",
            Self::Finished => "finished",
        };
        write!(f, "{s}")
    }
}

impl TranscodeStatus {
    #[must_use]
    pub fn get_proc_map(&self) -> HashMap<StackString, Option<ProcStatus>> {
        let upcoming = self.upcoming_jobs.iter().filter_map(|j| {
            j.input_path.file_name().map(|f| {
                let f = f.to_string_lossy().into_owned();
                let mut f_key = f.as_str();
                for k in [".mkv", ".m4v", ".avi", ".mp4"] {
                    if let Some(s) = f_key.strip_suffix(k) {
                        f_key = s;
                    }
                }
                (f_key.into(), Some(ProcStatus::Upcoming))
            })
        });
        let current = self.current_jobs.iter().filter_map(|(p, _)| {
            p.file_name().map(|f| {
                let f = f.to_string_lossy().into_owned();
                let mut f_key = f.as_str();
                if let Some(s) = f_key.strip_suffix("_mp4.out") {
                    f_key = s;
                }
                if let Some(s) = f_key.strip_suffix("_copy.out") {
                    f_key = s;
                }
                (f_key.into(), Some(ProcStatus::Current))
            })
        });
        let finished = self.finished_jobs.iter().filter_map(|p| {
            p.file_name().map(|f| {
                let f = f.to_string_lossy().into_owned();
                let mut f_key = f.as_str();
                if let Some(s) = f_key.strip_suffix("_mp4.out") {
                    f_key = s;
                }
                if let Some(s) = f_key.strip_suffix("_copy.out") {
                    f_key = s;
                }
                (f_key.into(), Some(ProcStatus::Finished))
            })
        });

        finished.chain(upcoming).chain(current).collect()
    }
}

impl fmt::Display for TranscodeStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if !self.procs.is_empty() {
            write!(
                f,
                "Running procs:\n\n{}\n\n",
                self.procs
                    .iter()
                    .sorted_by_key(|p| p.pid)
                    .map(|p| format_sstr!("{p}"))
                    .join("\n")
            )?;
        }
        if !self.upcoming_jobs.is_empty() {
            write!(
                f,
                "Upcoming jobs:\n\n{}\n\n",
                self.upcoming_jobs
                    .iter()
                    .map(StackString::from_display)
                    .join("\n")
            )?;
        }
        if !self.current_jobs.is_empty() {
            write!(
                f,
                "Current jobs:\n\n{}\n\n",
                self.current_jobs.iter().map(|(_, s)| s).join("\n")
            )?;
        }
        if !self.finished_jobs.is_empty() {
            write!(
                f,
                "Finished jobs:\n\n{}\n\n",
                self.finished_jobs
                    .iter()
                    .map(|p| p
                        .file_name()
                        .unwrap_or_else(|| OsStr::new(""))
                        .to_string_lossy())
                    .join("\n")
            )?;
        }
        Ok(())
    }
}

fn get_procs() -> Result<Vec<ProcInfo>, Error> {
    let accept_paths = &[
        Path::new("/usr/bin/run-encoding"),
        Path::new("/usr/bin/HandBrakeCLI"),
    ];
    let procs = process::all_processes()?
        .into_iter()
        .filter_map(|p| {
            let p = p.ok()?;
            let exe = p.exe().ok()?;
            if accept_paths.iter().any(|x| &exe == x) {
                let cmdline: Vec<_> = p
                    .cmdline()
                    .ok()?
                    .get(1..)
                    .unwrap_or(&[])
                    .iter()
                    .map(Into::into)
                    .collect();
                let status = p.status().ok()?;
                return Some(ProcInfo {
                    pid: p.pid as u64,
                    name: status.name.into(),
                    exe,
                    cmdline,
                });
            }
            None
        })
        .sorted_by_key(|p| p.pid)
        .collect();
    Ok(procs)
}

fn get_paths_sync(dir: impl AsRef<Path>, ext: &str) -> Vec<PathBuf> {
    WalkDir::new(dir)
        .into_iter()
        .filter_map(|fpath| {
            let fpath = fpath.ok()?;
            let fpath = fpath.path();
            if fpath.extension() == Some(ext.as_ref()) {
                Some(fpath)
            } else {
                None
            }
        })
        .sorted_by(Ord::cmp)
        .collect()
}

async fn get_paths(dir: impl AsRef<Path>, ext: &str) -> Result<Vec<PathBuf>, Error> {
    let dir = dir.as_ref().to_owned();
    let ext = ext.to_owned();
    spawn_blocking(move || get_paths_sync(dir, &ext))
        .await
        .map_err(Into::into)
}

async fn get_last_line(fpath: &Path) -> Result<StackString, Error> {
    let mut buf = Vec::new();
    let mut last = Vec::new();
    let f = File::open(&fpath).await?;
    let mut reader = BufReader::new(f);
    loop {
        if let Ok(0) = reader.read_until(b'\n', &mut buf).await {
            break;
        }
        if !buf.is_empty() {
            std::mem::swap(&mut buf, &mut last);
        }
        buf.clear();
    }
    last.rsplit(|b| *b == b'\r')
        .find(|b| !b.is_empty())
        .map_or_else(
            || String::from_utf8(buf).map_err(Into::into).map(Into::into),
            |line| str::from_utf8(line).map_err(Into::into).map(Into::into),
        )
}

async fn get_upcoming_jobs(p: impl AsRef<Path>) -> Result<Vec<TranscodeServiceRequest>, Error> {
    let futures = get_paths(p, "json")
        .await?
        .into_iter()
        .map(|fpath| async move {
            let js: TranscodeServiceRequest = serde_json::from_slice(&fs::read(fpath).await?)?;
            Ok(js)
        });
    let results: Result<Vec<_>, Error> = try_join_all(futures).await;
    let mut upcoming_jobs = results?;
    upcoming_jobs.sort_by(|x, y| x.prefix.cmp(&y.prefix));
    Ok(upcoming_jobs)
}

async fn get_current_jobs(p: impl AsRef<Path>) -> Result<Vec<(PathBuf, StackString)>, Error> {
    let futures = get_paths(p, "out")
        .await?
        .into_iter()
        .map(|fpath| async move { get_last_line(&fpath).await.map(|p| (fpath.clone(), p)) });
    try_join_all(futures).await
}

/// # Errors
/// Return error if db query fails
pub async fn transcode_status(config: &Config) -> Result<TranscodeStatus, Error> {
    let procs = get_procs()?;
    let (upcoming_jobs, current_jobs, finished_jobs) = try_join!(
        get_upcoming_jobs(job_dir(config)),
        get_current_jobs(log_dir(config)),
        get_paths(tmp_dir(config), "out")
    )?;

    Ok(TranscodeStatus {
        procs,
        upcoming_jobs,
        current_jobs,
        finished_jobs,
    })
}

/// # Errors
/// Return error if db query fails
pub fn movie_directories(config: &Config) -> Result<Vec<StackString>, Error> {
    let movie_dir = config.preferred_dir.join("Documents").join("movies");
    std::fs::read_dir(movie_dir)?
        .map(|entry| {
            let p = entry?.path();
            let p = p
                .file_name()
                .unwrap_or_else(|| OsStr::new(""))
                .to_string_lossy();
            Ok(p.as_ref().into())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use anyhow::Error;
    use stack_string::{format_sstr, StackString};
    use std::{collections::HashSet, fs::create_dir_all, path::Path};

    use crate::{
        config::Config,
        init_env,
        transcode_service::{
            get_current_jobs, get_last_line, get_paths, get_procs, get_upcoming_jobs,
            transcode_status, JobType, ProcInfo, TranscodeServiceRequest,
        },
    };

    #[tokio::test]
    async fn test_create_move_script() -> Result<(), Error> {
        init_env();
        let config = Config::new()?;
        let job_path = config.home_dir.join("dvdrip").join("jobs");
        create_dir_all(&job_path)?;
        let p = Path::new("mr_robot_s01_ep01.mp4");
        let d: Option<&Path> = None;
        let payload = TranscodeServiceRequest::create_remcom_request(&config, p, d, false).await?;
        println!("{:?}", payload);
        assert_eq!(payload.job_type, JobType::Move);
        assert_eq!(&payload.input_path, p);
        Ok(())
    }

    #[tokio::test]
    async fn test_create_move_script_movie() -> Result<(), Error> {
        init_env();
        let config = Config::new()?;
        let job_path = config.home_dir.join("dvdrip").join("jobs");
        create_dir_all(&job_path)?;
        let drama_dir = config
            .preferred_dir
            .join("Documents")
            .join("movies")
            .join("drama");
        create_dir_all(&drama_dir)?;
        let p = Path::new("a_night_to_remember.mp4");
        let payload = TranscodeServiceRequest::create_remcom_request(
            &config,
            p,
            Some(Path::new("drama")),
            false,
        )
        .await?;
        println!("{:?}", payload);
        assert_eq!(
            payload.output_path,
            config
                .preferred_dir
                .join("Documents/movies/drama/a_night_to_remember.mp4")
        );
        Ok(())
    }

    #[test]
    fn test_create_transcode_script() -> Result<(), Error> {
        init_env();
        let config = Config::new()?;
        let job_path = config.home_dir.join("dvdrip").join("jobs");
        create_dir_all(&job_path)?;
        let p = Path::new("mr_robot_s01_ep01.mkv");
        let payload = TranscodeServiceRequest::create_transcode_request(&config, p)?;
        println!("{:?}", payload);
        assert_eq!(&payload.input_path, p);
        let expected = config
            .home_dir
            .join("dvdrip")
            .join("avi")
            .join(&p.with_extension("mp4"));
        assert_eq!(payload.output_path, expected);
        Ok(())
    }

    #[tokio::test]
    #[ignore]
    async fn test_transcode_status() -> Result<(), Error> {
        let config = Config::with_config()?;
        let status = transcode_status(&config).await?;
        println!("{:?}", status);
        println!("{}", status);
        assert!(status.procs.len() >= 1);
        Ok(())
    }

    #[test]
    #[ignore]
    fn test_get_procs() -> Result<(), Error> {
        let procs = get_procs()?;
        assert!(procs.len() > 0);
        Ok(())
    }

    #[test]
    fn test_procinfo_display() -> Result<(), Error> {
        let cmdline = vec![
            "-i".into(),
            "/home/ddboline/Documents/movies/the_walking_dead_s10_ep02.mkv".into(),
            "-o".into(),
            "/home/ddboline/dvdrip/avi/the_walking_dead_s10_ep02.mp4".into(),
            "--preset".into(),
            "Android 480p30".into(),
        ];
        let p = ProcInfo {
            pid: 25625,
            name: "HandBrakeCLI".into(),
            exe: "/usr/bin/HandBrakeCLI".into(),
            cmdline: cmdline.clone(),
        };
        assert_eq!(
            StackString::from_display(p),
            format_sstr!(
                "25625\tHandBrakeCLI\t/usr/bin/HandBrakeCLI\t{}",
                cmdline.join(" ")
            )
        );
        Ok(())
    }

    #[tokio::test]
    async fn test_get_last_line() -> Result<(), Error> {
        println!("{:?}", std::env::current_dir());
        let p = Path::new("../tests/data/fargo_2014_s04_ep02_mp4.out");
        let output = get_last_line(&p).await?;
        assert_eq!(
            &output,
            "Encoding: task 1 of 1, 22.61 % (76.06 fps, avg 94.82 fps, ETA 00h12m06s)"
        );
        Ok(())
    }

    #[tokio::test]
    async fn test_get_current_jobs() -> Result<(), Error> {
        let result: Vec<_> = get_current_jobs("../tests/data")
            .await?
            .into_iter()
            .filter(|(p, _)| p.to_string_lossy().contains("fargo_2014_s04_ep02_mp4"))
            .collect();
        assert_eq!(
            result[0].1,
            "Encoding: task 1 of 1, 22.61 % (76.06 fps, avg 94.82 fps, ETA 00h12m06s)"
        );
        Ok(())
    }

    #[tokio::test]
    async fn test_get_paths() -> Result<(), Error> {
        let results = get_paths("../tests/data", "out").await?;
        println!("{:?}", results);
        assert!(results.len() > 0);
        Ok(())
    }

    #[tokio::test]
    async fn test_get_upcoming_jobs() -> Result<(), Error> {
        let results = get_upcoming_jobs("../tests/data").await?;
        println!("{:?}", results);
        let prefixes: HashSet<_> = results.iter().map(|r| r.prefix.clone()).collect();
        assert_eq!(results.len(), 2);
        assert!(prefixes.contains("fargo_2014_s04_ep02"));
        Ok(())
    }
}
