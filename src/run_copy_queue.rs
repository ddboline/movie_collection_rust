use movie_collection_lib::common::utils::read_transcode_jobs_from_queue;

fn main() {
    env_logger::init();

    read_transcode_jobs_from_queue("remcom_work_queue").unwrap();
}
