use failure::Error;
use rayon::prelude::*;

use crate::common::config::Config;
use crate::common::movie_collection::MovieCollectionDB;
use crate::common::movie_queue::MovieQueueDB;
use crate::common::pgpool::PgPool;
use crate::common::utils::{get_video_runtime, map_result_vec};

pub fn make_queue_worker(
    add_files: Option<Vec<String>>,
    del_files: Option<Vec<String>>,
    do_time: bool,
    patterns: &[&str],
    do_shows: bool,
) -> Result<(), Error> {
    let config = Config::with_config();
    let pool = PgPool::new(&config.pgurl);
    let mc = MovieCollectionDB::with_pool(pool.clone());
    let mq = MovieQueueDB::with_pool(pool);

    if do_shows {
        for show in mc.print_tv_shows()? {
            println!("{}", show);
        }
    } else if let Some(files) = del_files {
        for file in files {
            if let Ok(idx) = file.parse::<i32>() {
                mq.remove_from_queue_by_idx(idx)?;
            } else {
                mq.remove_from_queue_by_path(&file)?;
            }
        }
    } else if let Some(files) = add_files {
        if files.len() == 1 {
            let max_idx = mq.get_max_queue_index()?;
            mq.insert_into_queue(max_idx + 1, &files[0])?;
        } else if files.len() == 2 {
            if let Ok(idx) = files[0].parse::<i32>() {
                println!("inserting into {}", idx);
                mq.insert_into_queue(idx, &files[1])?;
            } else {
                for file in &files {
                    let max_idx = mq.get_max_queue_index()?;
                    mq.insert_into_queue(max_idx + 1, &file)?;
                }
            }
        } else {
            for file in &files {
                let max_idx = mq.get_max_queue_index()?;
                mq.insert_into_queue(max_idx + 1, &file)?;
            }
        }
    } else {
        let results = mq.print_movie_queue(&patterns)?;
        if do_time {
            let results: Vec<Result<_, Error>> = results
                .into_par_iter()
                .map(|result| {
                    let timeval = get_video_runtime(&result.path)?;
                    Ok((timeval, result))
                })
                .collect();

            let results = map_result_vec(results)?;

            for (timeval, result) in results {
                println!("{} {}", result, timeval);
            }
        } else {
            for result in results {
                println!("{}", result);
            }
        }
    }
    Ok(())
}
