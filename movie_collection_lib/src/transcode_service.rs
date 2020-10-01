use anyhow::{format_err, Error};
use deadpool_lapin::Config as LapinConfig;
use futures::{future::try_join_all, stream::StreamExt, try_join};
use itertools::Itertools;
use lapin::{
    options::{
        BasicAckOptions, BasicConsumeOptions, BasicPublishOptions, QueueDeclareOptions,
        QueueDeleteOptions, QueuePurgeOptions,
    },
    types::FieldTable,
    BasicProperties, Channel, Queue,
};
use procfs::process;
use serde::{Deserialize, Serialize};
use stack_string::StackString;
use std::{
    collections::HashMap,
    fmt,
    path::{Path, PathBuf},
    process::Stdio,
    str,
};
use tokio::{
    fs::{self, File, OpenOptions},
    io::{AsyncBufReadExt, AsyncRead, AsyncWriteExt, BufReader},
    process::Command,
    task::{spawn, spawn_blocking, JoinHandle},
};
use walkdir::WalkDir;

use crate::{
    config::Config, make_queue::make_queue_worker, movie_collection::MovieCollection,
    stdout_channel::StdoutChannel, utils::parse_file_stem,
};

#[derive(Serialize, Deserialize, Debug, PartialEq, Clone, Copy)]
pub enum JobType {
    Transcode,
    Move,
}

impl JobType {
    fn get_str(self) -> &'static str {
        match self {
            Self::Transcode => "transcode",
            Self::Move => "move",
        }
    }
}

impl fmt::Display for JobType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.get_str())
    }
}

#[derive(Serialize, Deserialize, Debug, PartialEq, Clone)]
pub struct TranscodeServiceRequest {
    pub job_type: JobType,
    pub prefix: StackString,
    pub input_path: PathBuf,
    pub output_path: PathBuf,
}

impl TranscodeServiceRequest {
    pub fn new(job_type: JobType, prefix: &str, input_path: &Path, output_path: &Path) -> Self {
        Self {
            job_type,
            prefix: prefix.into(),
            input_path: input_path.to_path_buf(),
            output_path: output_path.to_path_buf(),
        }
    }

    pub fn create_transcode_request(config: &Config, input_path: &Path) -> Result<Self, Error> {
        let input_path = input_path.to_path_buf();
        let fstem = input_path
            .file_stem()
            .ok_or_else(|| format_err!("No stem"))?;
        let output_file = avi_dir(config).join(&fstem).with_extension("mp4");
        let prefix = fstem.to_string_lossy().into_owned().into();

        Ok(Self {
            job_type: JobType::Transcode,
            prefix,
            input_path,
            output_path: output_file,
        })
    }

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
            let prefix = file_stem.to_string_lossy().to_string();
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
                    panic!("Failed to parse show season {} episode {}", season, episode);
                }

                let d = config
                    .preferred_dir
                    .join("Documents")
                    .join("television")
                    .join(show.as_str())
                    .join(format!("season{}", season));
                if !d.exists() {
                    fs::create_dir_all(&d).await?;
                }
                d
            };

            let prefix = prefix.into();
            let input_path = path.to_path_buf();
            let output_path = output_dir.join(&format!("{}.mp4", prefix));

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
}

pub struct TranscodeService {
    config: Config,
    queue: StackString,
}

impl TranscodeService {
    pub fn new(config: Config, queue: &str) -> Self {
        Self {
            config,
            queue: queue.into(),
        }
    }

    pub async fn init(&self) -> Result<Queue, Error> {
        let chan = Self::open_transcode_channel().await?;
        let queue = chan
            .queue_declare(
                &self.queue,
                QueueDeclareOptions::default(),
                FieldTable::default(),
            )
            .await?;
        Ok(queue)
    }

    pub async fn cleanup(&self) -> Result<u32, Error> {
        let chan = Self::open_transcode_channel().await?;
        chan.queue_purge(&self.queue, QueuePurgeOptions::default())
            .await?;
        chan.queue_delete(&self.queue, QueueDeleteOptions::default())
            .await
            .map_err(Into::into)
    }

    async fn open_transcode_channel() -> Result<Channel, Error> {
        let cfg = LapinConfig::default();
        let pool = cfg.create_pool();
        let conn = pool.get().await?;
        conn.create_channel().await.map_err(Into::into)
    }

    pub async fn publish_transcode_job(
        &self,
        payload: &TranscodeServiceRequest,
    ) -> Result<(), Error> {
        let chan = Self::open_transcode_channel().await?;
        let payload = serde_json::to_vec(&payload)?;
        chan.basic_publish(
            "",
            &self.queue,
            BasicPublishOptions::default(),
            payload,
            BasicProperties::default(),
        )
        .await?;
        Ok(())
    }

    pub async fn read_transcode_job(&self) -> Result<(), Error> {
        let chan = Self::open_transcode_channel().await?;
        let mut consumer = chan
            .basic_consume(
                &self.queue,
                &self.queue,
                BasicConsumeOptions::default(),
                FieldTable::default(),
            )
            .await?;
        while let Some(delivery) = consumer.next().await {
            let (channel, delivery) = delivery?;
            let payload: TranscodeServiceRequest = serde_json::from_slice(&delivery.data)?;
            match payload.job_type {
                JobType::Transcode => {
                    self.run_transcode(&payload.prefix, &payload.input_path, &payload.output_path)
                        .await?
                }
                JobType::Move => {
                    self.run_move(&payload.prefix, &payload.input_path, &payload.output_path)
                        .await?
                }
            }
            channel
                .basic_ack(delivery.delivery_tag, BasicAckOptions::default())
                .await?;
        }
        Ok(())
    }

    #[allow(dead_code)]
    async fn get_single_job(&self) -> Result<TranscodeServiceRequest, Error> {
        let chan = Self::open_transcode_channel().await?;
        let mut consumer = chan
            .basic_consume(
                &self.queue,
                &self.queue,
                BasicConsumeOptions::default(),
                FieldTable::default(),
            )
            .await?;
        if let Some(delivery) = consumer.next().await {
            let (channel, delivery) = delivery?;
            let payload: TranscodeServiceRequest = serde_json::from_slice(&delivery.data)?;
            channel
                .basic_ack(delivery.delivery_tag, BasicAckOptions::default())
                .await?;
            Ok(payload)
        } else {
            Err(format_err!("No Messages?"))
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
        let script_file = job_dir(&self.config).join(&prefix).with_extension("json");
        if script_file.exists() {
            fs::remove_file(&script_file).await?;
        }

        if !input_file.exists() {
            return Err(format_err!("{:?} does not exist", input_file));
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
        let debug_output_path = log_dir(&self.config).join(&format!("{}_mp4", prefix));
        let stdout_path = debug_output_path.with_extension("out");
        let stderr_path = debug_output_path.with_extension("err");

        let mut p = Command::new("HandBrakeCLI")
            .args(&[
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

        let transcode_task = spawn(async move { p.await });

        let status = transcode_task.await??;
        println!("Handbrake exited with {}", status);
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
            let new_debug_output_path = tmp_avi_path.join(&format!("{}_mp4.out", prefix));
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
            .join(&format!("{}_copy", show))
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
                .join(&input_file)
        };
        if !input_file.exists() {
            return Err(format_err!("{:?} does not exist", input_file));
        }

        let debug_output_path = log_dir(&self.config).join(&format!("{}_copy.out", show));
        let mut debug_output_file = File::create(&debug_output_path).await?;

        let show_path = self
            .config
            .home_dir
            .join("Documents")
            .join("movies")
            .join(&format!("{}.mp4", show));
        if !show_path.exists() {
            return Ok(());
        }
        let new_path = output_file.with_extension("new");
        let task0 = spawn({
            let new_path = new_path.clone();
            debug_output_file
                .write_all(
                    format!(
                        "copy {} to {}\n",
                        show_path.to_string_lossy(),
                        new_path.to_string_lossy()
                    )
                    .as_bytes(),
                )
                .await?;
            async move { fs::copy(&show_path, &new_path).await }
        });
        if output_file.exists() {
            let old_path = output_file.with_extension("old");
            debug_output_file
                .write_all(
                    format!(
                        "copy {} to {}\n",
                        output_file.to_string_lossy(),
                        old_path.to_string_lossy()
                    )
                    .as_bytes(),
                )
                .await?;
            fs::rename(&output_file, &old_path).await?;
        }
        task0.await??;
        debug_output_file
            .write_all(
                format!(
                    "copy {} to {}\n",
                    new_path.to_string_lossy(),
                    output_file.to_string_lossy()
                )
                .as_bytes(),
            )
            .await?;
        fs::rename(&new_path, &output_file).await?;
        let stdout = StdoutChannel::new();
        make_queue_worker(&[], &[output_file.into()], false, &[], false, &stdout).await?;
        debug_output_file
            .write_all(format!("add {} to queue\n", output_file.to_string_lossy()).as_bytes())
            .await?;
        make_queue_worker(&[output_file.into()], &[], false, &[], false, &stdout).await?;
        let mc = MovieCollection::new();
        debug_output_file.write_all(b"update collection\n").await?;
        mc.make_collection().await?;
        mc.fix_collection_show_id().await?;

        debug_output_file.flush().await?;

        if debug_output_path.exists() {
            let new_debug_output_path = tmp_dir(&self.config).join(&format!("{}_copy.out", show));
            fs::rename(&debug_output_path, &new_debug_output_path).await?;
        }

        Ok(())
    }
}

fn movie_dir(config: &Config) -> PathBuf {
    config.home_dir.join("Documents").join("movies")
}
fn dvdrip_dir(config: &Config) -> PathBuf {
    config.home_dir.join("dvdrip")
}

fn avi_dir(config: &Config) -> PathBuf {
    dvdrip_dir(config).join("avi")
}

fn log_dir(config: &Config) -> PathBuf {
    dvdrip_dir(config).join("log")
}

fn job_dir(config: &Config) -> PathBuf {
    dvdrip_dir(config).join("jobs")
}

fn tmp_dir(config: &Config) -> PathBuf {
    config.home_dir.join("tmp_avi")
}

#[derive(Debug)]
pub struct ProcInfo {
    pub pid: u64,
    pub name: StackString,
    pub exe: PathBuf,
    pub cmdline: Vec<StackString>,
}

impl ProcInfo {
    pub fn get_header() -> Vec<&'static str> {
        vec!["Pid", "Name", "Exe Path", "Cmdline Args"]
    }

    pub fn get_html(&self) -> Vec<StackString> {
        let mut output = Vec::new();
        output.push(self.pid.to_string().into());
        output.push(self.name.clone());
        output.push(self.exe.to_string_lossy().to_string().into());
        output.push(self.cmdline.join(" ").into());
        output
    }
}

#[derive(Debug)]
pub struct TranscodeStatus {
    pub procs: Vec<ProcInfo>,
    pub upcoming_jobs: Vec<TranscodeServiceRequest>,
    pub current_jobs: Vec<(PathBuf, StackString)>,
    pub finished_jobs: Vec<PathBuf>,
}

#[derive(Copy, Clone)]
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
        write!(f, "{}", s)
    }
}

impl TranscodeStatus {
    pub fn get_proc_map(&self) -> HashMap<StackString, Option<ProcStatus>> {
        let upcoming = self.upcoming_jobs.iter().filter_map(|j| {
            j.input_path.file_name().map(|f| {
                (
                    f.to_string_lossy().into_owned().into(),
                    Some(ProcStatus::Upcoming),
                )
            })
        });
        let current = self.current_jobs.iter().filter_map(|(p, _)| {
            p.file_name().map(|f| {
                (
                    f.to_string_lossy().into_owned().into(),
                    Some(ProcStatus::Current),
                )
            })
        });
        let finished = self.finished_jobs.iter().filter_map(|p| {
            p.file_name().map(|f| {
                (
                    f.to_string_lossy().into_owned().into(),
                    Some(ProcStatus::Finished),
                )
            })
        });

        upcoming.chain(current).chain(finished).collect()
    }

    pub fn get_html(&self) -> Vec<StackString> {
        let mut output: Vec<StackString> = Vec::new();
        output.push("Running procs:<br>".into());
        output.push(r#"<table border="1" class="dataframe">"#.into());
        output.push(
            format!(
                r#"<thead><tr><th>{}</th></tr></thead>"#,
                ProcInfo::get_header().join("</th><th>")
            )
            .into(),
        );
        output.push(
            format!(
                r#"<tbody><tr><td>{}</td></tr></tbody>"#,
                self.procs
                    .iter()
                    .map(|p| p.get_html().join("</td><td>"))
                    .join("</tr><tr>")
            )
            .into(),
        );
        output.push("</table>".into());
        output.push("Upcoming jobs:<br>".into());
        output.push(
            self.upcoming_jobs
                .iter()
                .map(|t| {
                    format!(
                        "Type: {}, Name: {}, Input: {}, Output: {}",
                        t.job_type,
                        t.prefix,
                        t.input_path.to_string_lossy(),
                        t.output_path.to_string_lossy(),
                    )
                })
                .join("<br>")
                .into(),
        );
        output.push(
            format!(
                "Current jobs:<br>{}",
                self.current_jobs.iter().map(|(_, s)| s).join("<br>")
            )
            .into(),
        );
        output.push(
            format!(
                "Finished jobs:<br>{}",
                self.finished_jobs
                    .iter()
                    .map(|p| p.to_string_lossy())
                    .join("<br>")
            )
            .into(),
        );
        output
    }
}

impl fmt::Display for TranscodeStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Running procs:\n{}\n",
            self.procs.iter().map(|p| format!("{:?}", p)).join("\n")
        )?;
        write!(
            f,
            "Upcoming jobs:\n{}\n",
            self.upcoming_jobs
                .iter()
                .map(|j| format!("{:?}", j))
                .join("\n")
        )?;
        write!(
            f,
            "Current jobs:\n{}\n",
            self.current_jobs.iter().map(|(_, s)| s).join("\n")
        )?;
        write!(
            f,
            "Finished jobs:\n{}\n",
            self.finished_jobs
                .iter()
                .map(|p| format!("{:?}", p))
                .join("\n")
        )
    }
}

fn get_procs_sync() -> Result<Vec<ProcInfo>, Error> {
    let accept_paths = &[
        Path::new("/usr/bin/run-encoding"),
        Path::new("/usr/bin/HandBrakeCLI"),
    ];
    let procs = process::all_processes()?
        .into_iter()
        .filter_map(|p| {
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
        .collect();
    Ok(procs)
}

async fn get_procs() -> Result<Vec<ProcInfo>, Error> {
    spawn_blocking(get_procs_sync).await?
}

fn get_paths_sync(dir: impl AsRef<Path>, ext: &str) -> Vec<PathBuf> {
    WalkDir::new(dir)
        .into_iter()
        .filter_map(|fpath| {
            let fpath = fpath.ok()?;
            let fpath = fpath.path();
            if fpath.extension() == Some(ext.as_ref()) {
                Some(fpath.to_path_buf())
            } else {
                None
            }
        })
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
    if let Some(buf) = last.rsplit(|b| *b == b'\r').find(|b| !b.is_empty()) {
        str::from_utf8(buf).map(Into::into).map_err(Into::into)
    } else {
        String::from_utf8(buf).map_err(Into::into).map(Into::into)
    }
}

async fn get_upcoming_jobs(p: impl AsRef<Path>) -> Result<Vec<TranscodeServiceRequest>, Error> {
    let futures = get_paths(p, "json")
        .await?
        .into_iter()
        .map(|fpath| async move {
            let js: TranscodeServiceRequest = serde_json::from_slice(&fs::read(fpath).await?)?;
            Ok(js)
        });
    try_join_all(futures).await
}

async fn get_current_jobs(p: impl AsRef<Path>) -> Result<Vec<(PathBuf, StackString)>, Error> {
    let futures = get_paths(p, "out")
        .await?
        .into_iter()
        .map(|fpath| async move {
            get_last_line(&fpath)
                .await
                .map(|p| (fpath.to_path_buf(), p))
        });
    try_join_all(futures).await
}

pub async fn transcode_status(config: &Config) -> Result<TranscodeStatus, Error> {
    let (procs, upcoming_jobs, current_jobs, finished_jobs) = try_join!(
        get_procs(),
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

pub async fn transcode_avi(
    config: &Config,
    stdout: &StdoutChannel,
    files: impl IntoIterator<Item = impl AsRef<Path>>,
) -> Result<(), Error> {
    let transcode_service = TranscodeService::new(config.clone(), &config.transcode_queue);
    transcode_service.init().await?;

    for path in files {
        let path = path.as_ref();
        let movie_path = movie_dir(config);
        let path = if path.exists() {
            path.to_path_buf()
        } else {
            movie_path.join(path)
        }
        .canonicalize()?;

        if !path.exists() {
            panic!("file doesn't exist {}", path.to_string_lossy());
        }
        let payload = TranscodeServiceRequest::create_transcode_request(&config, &path)?;

        let script_file = job_dir(config).join(&payload.prefix).with_extension("json");
        fs::write(&script_file, &serde_json::to_vec(&payload)?).await?;

        transcode_service.publish_transcode_job(&payload).await?;
        stdout.send(format!("script {:?}", payload));
    }
    stdout.close().await
}

pub async fn remcom(
    files: impl IntoIterator<Item = impl AsRef<Path>>,
    directory: Option<impl AsRef<Path>>,
    unwatched: bool,
    config: &Config,
    stdout: &StdoutChannel,
) -> Result<(), Error> {
    let remcom_service = TranscodeService::new(config.clone(), &config.remcom_queue);

    for file in files {
        let payload = TranscodeServiceRequest::create_remcom_request(
            &config,
            file.as_ref(),
            directory.as_ref(),
            unwatched,
        )
        .await?;

        let script_file = job_dir(config)
            .join(&format!("{}_copy", payload.prefix))
            .with_extension("json");
        fs::write(&script_file, &serde_json::to_vec(&payload)?).await?;

        remcom_service.publish_transcode_job(&payload).await?;
        stdout.send(format!("script {:?}", payload));
    }
    stdout.close().await
}

#[cfg(test)]
mod tests {
    use anyhow::Error;
    use std::{env::set_var, fs::create_dir_all, path::Path};
    use tokio::task::spawn;

    use crate::{
        config::Config,
        transcode_service::{
            get_current_jobs, get_last_line, get_paths, get_procs_sync, get_upcoming_jobs,
            transcode_status, JobType, TranscodeService, TranscodeServiceRequest,
        },
    };

    fn init_env() {
        set_var(
            "PGURL",
            "postgresql://USER:PASSWORD@localhost:5432/movie_queue",
        );
        set_var("AUTHDB", "postgresql://USER:PASSWORD@localhost:5432/auth");
        set_var("MOVIE_DIRS", "/tmp");
        set_var("PREFERED_DISK", "/tmp");
        set_var("JWT_SECRET", "JWT_SECRET");
        set_var("SECRET_KEY", "SECRET_KEY");
        set_var("DOMAIN", "DOMAIN");
        set_var("SPARKPOST_API_KEY", "SPARKPOST_API_KEY");
        set_var("SENDING_EMAIL_ADDRESS", "SENDING_EMAIL_ADDRESS");
        set_var("CALLBACK_URL", "https://{DOMAIN}/auth/register.html");
        set_var("TRAKT_CLIENT_ID", "");
        set_var("TRAKT_CLIENT_SECRET", "");
    }

    #[tokio::test]
    async fn test_create_move_script() -> Result<(), Error> {
        init_env();
        let config = Config::new()?;
        let job_path = config.home_dir.join("dvdrip").join("jobs");
        create_dir_all(&job_path)?;
        let p = Path::new("mr_robot_s01_ep01.mp4");
        let payload =
            TranscodeServiceRequest::create_remcom_request(&config, p, None, false).await?;
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
    async fn test_transcode_service() -> Result<(), Error> {
        let config = Config::with_config()?;
        let service = TranscodeService::new(config.clone(), "test_queue");
        let queue = service.init().await?;
        println!("{:?}", queue);
        let task = spawn(async move { service.get_single_job().await });
        let service = TranscodeService::new(config, "test_queue");
        let req = TranscodeServiceRequest::new(
            JobType::Transcode,
            "test_prefix",
            &Path::new("test_input.mkv"),
            &Path::new("test_output.mp4"),
        );
        service.publish_transcode_job(&req).await?;
        let result = task.await??;
        assert_eq!(result, req);
        let result = service.cleanup().await?;
        println!("{}", result);
        Ok(())
    }

    #[tokio::test]
    #[ignore]
    async fn test_transcode_status() -> Result<(), Error> {
        let config = Config::with_config()?;
        let status = transcode_status(&config).await?;
        println!("{:?}", status);
        println!("{}", status);
        assert_eq!(status.procs.len(), 1);
        Ok(())
    }

    #[test]
    #[ignore]
    fn test_get_procs_sync() -> Result<(), Error> {
        let procs = get_procs_sync()?;
        assert!(procs.len() > 0);
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
        let result = get_current_jobs("../tests/data").await?;
        assert_eq!(
            result[0],
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
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].prefix, "fargo_2014_s04_ep02");
        Ok(())
    }
}
