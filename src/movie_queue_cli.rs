use anyhow::Error;
use clap::Parser;
use derive_more::{From, Into};
use futures::{future, future::try_join_all, TryStreamExt};
use log::error;
use refinery::embed_migrations;
use stack_string::StackString;
use std::{path::PathBuf, str::FromStr};
use stdout_channel::StdoutChannel;
use time::{format_description::well_known::Rfc3339, Duration, OffsetDateTime};
use tokio::{
    fs::{read, File},
    io::{self, stdin, AsyncReadExt, AsyncWrite, AsyncWriteExt},
};

use movie_collection_lib::{
    config::Config,
    imdb_episodes::ImdbEpisodes,
    imdb_ratings::ImdbRatings,
    movie_collection::{LastModifiedResponse, MovieCollection, MovieCollectionRow},
    movie_queue::{MovieQueueDB, MovieQueueRow},
    music_collection::MusicCollection,
    pgpool::PgPool,
    plex_events::{PlexEvent, PlexFilename, PlexMetadata},
    transcode_service::transcode_status,
};

embed_migrations!("migrations");

#[derive(Into, From, Debug, PartialEq, Eq, PartialOrd, Ord, Clone, Copy)]
pub struct DateTimeType(OffsetDateTime);

impl FromStr for DateTimeType {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        OffsetDateTime::parse(&s.replace('Z', "+00:00"), &Rfc3339)
            .map(Into::into)
            .map_err(Into::into)
    }
}

#[derive(Parser)]
enum MovieQueueCli {
    Import {
        #[clap(short, long)]
        /// table -- possible values:
        /// [`imdb_ratings`, `imdb_episodes`, `movie_collection`, `movie_queue`]
        table: StackString,
        #[clap(short, long)]
        filepath: Option<PathBuf>,
    },
    Export {
        #[clap(short, long)]
        /// table -- possible values:
        /// [`last_modified`, `imdb_ratings`, `imdb_episodes`,
        /// `movie_collection`, `movie_queue`]
        table: StackString,
        #[clap(short, long)]
        filepath: Option<PathBuf>,
        #[clap(short, long)]
        start_timestamp: Option<DateTimeType>,
    },
    Status,
    /// Run refinery migrations
    RunMigrations,
    FillPlex,
}

impl MovieQueueCli {
    #[allow(clippy::too_many_lines)]
    async fn run() -> Result<(), Error> {
        let config = Config::with_config()?;
        let pool = PgPool::new(&config.pgurl);
        let stdout = StdoutChannel::new();

        match Self::parse() {
            Self::Import { table, filepath } => {
                let data = if let Some(filepath) = filepath {
                    read(&filepath).await?
                } else {
                    let mut stdin = stdin();
                    let mut buf = Vec::new();
                    stdin.read_to_end(&mut buf).await?;
                    buf
                };
                match table.as_str() {
                    "imdb_ratings" => {
                        let shows: Vec<ImdbRatings> = serde_json::from_slice(&data)?;
                        let futures = shows.into_iter().map(|show| {
                            let pool = pool.clone();
                            async move {
                                match ImdbRatings::get_show_by_link(show.link.as_ref(), &pool)
                                    .await?
                                {
                                    Some(_) => show.update_show(&pool).await?,
                                    None => show.insert_show(&pool).await?,
                                };
                                Ok(())
                            }
                        });
                        let results: Result<Vec<_>, Error> = try_join_all(futures).await;
                        stdout.send(format!("imdb_ratings {}\n", results?.len()));
                    }
                    "imdb_episodes" => {
                        let episodes: Vec<ImdbEpisodes> = serde_json::from_slice(&data)?;
                        let futures = episodes.into_iter().map(|episode| {
                            let pool = pool.clone();
                            async move {
                                match episode.get_index(&pool).await? {
                                    Some(_) => episode.update_episode(&pool).await?,
                                    None => episode.insert_episode(&pool).await?,
                                };
                                Ok(())
                            }
                        });
                        let results: Result<Vec<_>, Error> = try_join_all(futures).await;
                        stdout.send(format!("imdb_episodes {}\n", results?.len()));
                    }
                    "plex_event" => {
                        let events: Vec<PlexEvent> = serde_json::from_slice(&data)?;
                        let futures = events.into_iter().map(|event| {
                            let pool = pool.clone();
                            async move {
                                event.write_event(&pool).await?;
                                Ok(())
                            }
                        });
                        let results: Result<Vec<_>, Error> = try_join_all(futures).await;
                        stdout.send(format!("plex_event {}\n", results?.len()));
                    }
                    "plex_filename" => {
                        let filenames: Vec<PlexFilename> = serde_json::from_slice(&data)?;
                        let futures = filenames.into_iter().map(|filename| {
                            let pool = pool.clone();
                            async move {
                                if PlexFilename::get_by_key(&pool, &filename.metadata_key)
                                    .await?
                                    .is_none()
                                {
                                    filename.insert(&pool).await?;
                                }
                                Ok(())
                            }
                        });
                        let results: Result<Vec<_>, Error> = try_join_all(futures).await;
                        stdout.send(format!("plex_filename {}\n", results?.len()));
                    }
                    "plex_metadata" => {
                        let metadatas: Vec<PlexMetadata> = serde_json::from_slice(&data)?;
                        let futures = metadatas.into_iter().map(|metadata| {
                            let pool = pool.clone();
                            async move {
                                if PlexMetadata::get_by_key(&pool, &metadata.metadata_key)
                                    .await?
                                    .is_none()
                                {
                                    metadata.insert(&pool).await?;
                                }
                                Ok(())
                            }
                        });
                        let results: Result<Vec<_>, Error> = try_join_all(futures).await;
                        stdout.send(format!("plex_metadata {}\n", results?.len()));
                    }
                    "movie_collection" => {
                        let rows: Vec<MovieCollectionRow> = serde_json::from_slice(&data)?;
                        let mc = MovieCollection::new(&config, &pool, &stdout);
                        let futures = rows.into_iter().map(|entry| {
                            let mc = mc.clone();
                            async move {
                                if mc
                                    .get_collection_index(entry.path.as_ref())
                                    .await?
                                    .is_none()
                                {
                                    mc.insert_into_collection(entry.path.as_ref(), false)
                                        .await?;
                                }
                                Ok(())
                            }
                        });
                        let results: Result<Vec<_>, Error> = try_join_all(futures).await;
                        stdout.send(format!("movie_collection {}\n", results?.len()));
                    }
                    "music_collection" => {
                        let rows: Vec<MusicCollection> = serde_json::from_slice(&data)?;
                        let futures = rows.into_iter().map(|entry| {
                            let pool = pool.clone();
                            async move {
                                if MusicCollection::get_by_id(&pool, entry.id).await?.is_none() {
                                    entry.insert(&pool).await?;
                                }
                                Ok(())
                            }
                        });
                        let results: Result<Vec<_>, Error> = try_join_all(futures).await;
                        stdout.send(format!("music_collection {}\n", results?.len()));
                    }
                    "movie_queue" => {
                        let mq = MovieQueueDB::new(&config, &pool, &stdout);
                        let mc = MovieCollection::new(&config, &pool, &stdout);
                        let entries: Vec<MovieQueueRow> = serde_json::from_slice(&data)?;
                        let futures = entries.into_iter().map(|mut entry| {
                            let mq = mq.clone();
                            let mc = mc.clone();
                            async move {
                                let cidx = if let Some(i) =
                                    mc.get_collection_index(entry.path.as_ref()).await?
                                {
                                    i
                                } else {
                                    mc.insert_into_collection(entry.path.as_ref(), false)
                                        .await?;
                                    entry.collection_idx
                                };
                                entry.collection_idx = cidx;
                                if mq.get_idx_from_collection_idx(cidx).await?.is_none() {
                                    mq.insert_into_queue_by_collection_idx(
                                        entry.idx,
                                        entry.collection_idx,
                                    )
                                    .await?;
                                }
                                Ok(())
                            }
                        });
                        let results: Result<Vec<_>, Error> = try_join_all(futures).await;
                        stdout.send(format!("movie_queue {}\n", results?.len()));
                    }
                    _ => {}
                }
            }
            Self::Export {
                table,
                filepath,
                start_timestamp,
            } => {
                let start_timestamp = start_timestamp
                    .map_or_else(|| OffsetDateTime::now_utc() - Duration::days(7), Into::into);
                let mut file: Box<dyn AsyncWrite + Unpin> = if let Some(filepath) = filepath {
                    Box::new(File::create(&filepath).await?)
                } else {
                    Box::new(io::stdout())
                };
                match table.as_str() {
                    "last_modified" => {
                        let last_modified = LastModifiedResponse::get_last_modified(&pool).await?;
                        file.write_all(&serde_json::to_vec(&last_modified)?).await?;
                    }
                    "imdb_ratings" => {
                        let shows: Vec<_> =
                            ImdbRatings::get_shows_after_timestamp(start_timestamp, &pool)
                                .await?
                                .try_collect()
                                .await?;
                        file.write_all(&serde_json::to_vec(&shows)?).await?;
                    }
                    "imdb_episodes" => {
                        let episodes: Vec<_> =
                            ImdbEpisodes::get_episodes_after_timestamp(start_timestamp, &pool)
                                .await?
                                .try_collect()
                                .await?;
                        file.write_all(&serde_json::to_vec(&episodes)?).await?;
                    }
                    "plex_event" => {
                        let events: Vec<_> =
                            PlexEvent::get_events(&pool, Some(start_timestamp), None, None, None)
                                .await?
                                .try_collect()
                                .await?;
                        file.write_all(&serde_json::to_vec(&events)?).await?;
                    }
                    "plex_filename" => {
                        let filenames: Vec<_> =
                            PlexFilename::get_filenames(&pool, Some(start_timestamp), None, None)
                                .await?
                                .try_collect()
                                .await?;
                        file.write_all(&serde_json::to_vec(&filenames)?).await?;
                    }
                    "plex_metadata" => {
                        let metadatas: Vec<_> =
                            PlexMetadata::get_entries(&pool, Some(start_timestamp), None, None)
                                .await?
                                .try_collect()
                                .await?;
                        file.write_all(&serde_json::to_vec(&metadatas)?).await?;
                    }
                    "movie_collection" => {
                        let mc = MovieCollection::new(&config, &pool, &stdout);
                        let entries = mc.get_collection_after_timestamp(start_timestamp).await?;
                        file.write_all(&serde_json::to_vec(&entries)?).await?;
                    }
                    "music_collection" => {
                        let entries: Vec<_> =
                            MusicCollection::get_entries(&pool, Some(start_timestamp), None, None)
                                .await?
                                .try_collect()
                                .await?;
                        file.write_all(&serde_json::to_vec(&entries)?).await?;
                    }
                    "movie_queue" => {
                        let mq = MovieQueueDB::new(&config, &pool, &stdout);
                        let entries = mq.get_queue_after_timestamp(start_timestamp).await?;
                        file.write_all(&serde_json::to_vec(&entries)?).await?;
                    }
                    _ => {}
                }
            }
            Self::Status => {
                let status = transcode_status(&config).await?;
                println!("{}", status);
            }
            Self::RunMigrations => {
                let mut conn = pool.get().await?;
                migrations::runner().run_async(&mut **conn).await?;
            }
            Self::FillPlex => {
                let events: Vec<_> = PlexEvent::get_events(&pool, None, None, None, None)
                    .await?
                    .try_filter(|event| future::ready(event.metadata_key.is_some()))
                    .try_collect()
                    .await?;
                for event in events {
                    let metadata_key = event.metadata_key.as_ref().expect("Unexpected failure");
                    if PlexFilename::get_by_key(&pool, metadata_key)
                        .await?
                        .is_none()
                    {
                        match event.get_filename(&config).await {
                            Ok(filename) => {
                                filename.insert(&pool).await?;
                            }
                            Err(e) => {
                                error!("Got error {e}");
                            }
                        }
                    }
                }
            }
        }
        stdout.close().await?;
        Ok(())
    }
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    MovieQueueCli::run().await
}
