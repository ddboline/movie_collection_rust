extern crate failure;
extern crate movie_collection_rust;

use movie_collection_rust::common::utils::read_transcode_jobs_from_queue;

fn main() {
    read_transcode_jobs_from_queue("transcode_work_queue").unwrap();
}
