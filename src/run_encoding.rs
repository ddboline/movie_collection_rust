use crossbeam_utils::thread;

use movie_collection_lib::{config::Config, utils::read_transcode_jobs_from_queue};

fn main() {
    env_logger::init();
    let config = Config::with_config().unwrap();

    thread::scope(|s| {
        let a = s.spawn(|_| read_transcode_jobs_from_queue(config.transcode_queue.as_str()));
        let b = s.spawn(|_| read_transcode_jobs_from_queue(config.remcom_queue.as_str()));
        a.join().unwrap().unwrap();
        b.join().unwrap().unwrap();
    })
    .unwrap();
}
