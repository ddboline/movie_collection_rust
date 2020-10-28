use anyhow::Error;
use chrono::{DateTime, Duration, Utc};
use futures::future::try_join_all;
use stack_string::StackString;
use std::path::PathBuf;
use structopt::StructOpt;
use tokio::{
    fs::{read_to_string, File},
    io::{stdin, stdout, AsyncReadExt, AsyncWrite, AsyncWriteExt},
};

use movie_collection_lib::{
    config::Config,
    imdb_episodes::ImdbEpisodes,
    imdb_ratings::ImdbRatings,
    movie_collection::{LastModifiedResponse, MovieCollection, MovieCollectionRow},
    movie_queue::{MovieQueueDB, MovieQueueRow},
    pgpool::PgPool,
    transcode_service::transcode_status,
};

#[derive(StructOpt)]
enum MovieQueueCli {
    Import {
        #[structopt(short, long)]
        /// table -- possible values:
        /// ['imdb_ratings', 'imdb_episodes', 'movie_collection', 'movie_queue']
        table: StackString,
        #[structopt(short, long)]
        filepath: Option<PathBuf>,
    },
    Export {
        #[structopt(short, long)]
        /// table -- possible values:
        /// ['last_modified', 'imdb_ratings', 'imdb_episodes',
        /// 'movie_collection', 'movie_queue']
        table: StackString,
        #[structopt(short, long)]
        filepath: Option<PathBuf>,
        #[structopt(short, long)]
        start_timestamp: Option<DateTime<Utc>>,
    },
    Status,
}

impl MovieQueueCli {
    #[allow(clippy::too_many_lines)]
    async fn run() -> Result<(), Error> {
        let config = Config::with_config()?;
        let pool = PgPool::new(&config.pgurl);
        match Self::from_args() {
            Self::Import { table, filepath } => {
                let data = if let Some(filepath) = filepath {
                    read_to_string(&filepath).await?
                } else {
                    let mut stdin = stdin();
                    let mut buf = String::new();
                    stdin.read_to_string(&mut buf).await?;
                    buf
                };
                match table.as_str() {
                    "imdb_ratings" => {
                        let shows: Vec<ImdbRatings> = serde_json::from_str(&data)?;
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
                        stdout()
                            .write_all(format!("imdb_ratings {}\n", results?.len()).as_bytes())
                            .await?;
                    }
                    "imdb_episodes" => {
                        let episodes: Vec<ImdbEpisodes> = serde_json::from_str(&data)?;
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
                        stdout()
                            .write_all(format!("imdb_episodes {}\n", results?.len()).as_bytes())
                            .await?;
                    }
                    "movie_collection" => {
                        let rows: Vec<MovieCollectionRow> = serde_json::from_str(&data)?;
                        let mc = MovieCollection::with_pool(&config, &pool)?;
                        let futures = rows.into_iter().map(|entry| {
                            let mc = mc.clone();
                            async move {
                                if let Some(cidx) =
                                    mc.get_collection_index(entry.path.as_ref()).await?
                                {
                                    if cidx == entry.idx {
                                        return Ok(());
                                    }
                                    mc.remove_from_collection(entry.path.as_ref()).await?;
                                };
                                mc.insert_into_collection_by_idx(entry.idx, entry.path.as_ref())
                                    .await?;
                                Ok(())
                            }
                        });
                        let results: Result<Vec<_>, Error> = try_join_all(futures).await;
                        stdout()
                            .write_all(format!("movie_collection {}\n", results?.len()).as_bytes())
                            .await?;
                    }
                    "movie_queue" => {
                        let mq = MovieQueueDB::with_pool(&config, &pool);
                        let mc = MovieCollection::with_pool(&config, &pool)?;
                        let entries: Vec<MovieQueueRow> = serde_json::from_str(&data)?;
                        let futures = entries.into_iter().map(|entry| {
                            let mq = mq.clone();
                            let mc = mc.clone();
                            async move {
                                let cidx = if let Some(i) =
                                    mc.get_collection_index(entry.path.as_ref()).await?
                                {
                                    i
                                } else {
                                    mc.insert_into_collection_by_idx(
                                        entry.collection_idx,
                                        entry.path.as_ref(),
                                    )
                                    .await?;
                                    entry.collection_idx
                                };
                                assert_eq!(cidx, entry.collection_idx);
                                mq.insert_into_queue_by_collection_idx(
                                    entry.idx,
                                    entry.collection_idx,
                                )
                                .await?;
                                Ok(())
                            }
                        });
                        let results: Result<Vec<_>, Error> = try_join_all(futures).await;
                        stdout()
                            .write_all(format!("movie_queue {}\n", results?.len()).as_bytes())
                            .await?;
                    }
                    _ => {}
                }
            }
            Self::Export {
                table,
                filepath,
                start_timestamp,
            } => {
                let start_timestamp =
                    start_timestamp.unwrap_or_else(|| Utc::now() - Duration::days(7));
                let mut file: Box<dyn AsyncWrite + Unpin> = if let Some(filepath) = filepath {
                    Box::new(File::create(&filepath).await?)
                } else {
                    Box::new(stdout())
                };
                match table.as_str() {
                    "last_modified" => {
                        let last_modified = LastModifiedResponse::get_last_modified(&pool).await?;
                        file.write_all(&serde_json::to_vec(&last_modified)?).await?;
                    }
                    "imdb_ratings" => {
                        let shows =
                            ImdbRatings::get_shows_after_timestamp(start_timestamp, &pool).await?;
                        file.write_all(&serde_json::to_vec(&shows)?).await?;
                    }
                    "imdb_episodes" => {
                        let episodes =
                            ImdbEpisodes::get_episodes_after_timestamp(start_timestamp, &pool)
                                .await?;
                        file.write_all(&serde_json::to_vec(&episodes)?).await?;
                    }
                    "movie_collection" => {
                        let mc = MovieCollection::with_pool(&config, &pool)?;
                        let entries = mc.get_collection_after_timestamp(start_timestamp).await?;
                        file.write_all(&serde_json::to_vec(&entries)?).await?;
                    }
                    "movie_queue" => {
                        let mq = MovieQueueDB::with_pool(&config, &pool);
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
        }

        Ok(())
    }
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    MovieQueueCli::run().await
}
