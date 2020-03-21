use anyhow::Error;
use std::path::PathBuf;
use structopt::StructOpt;

use movie_collection_lib::utils::remcom_single_file;

#[derive(StructOpt)]
/// Create script to copy files, push job to queue
struct RemcomOpts {
    /// Directory
    #[structopt(long, short)]
    directory: Option<PathBuf>,

    #[structopt(long, short)]
    unwatched: bool,

    files: Vec<PathBuf>,
}

fn remcom() -> Result<(), Error> {
    let opts = RemcomOpts::from_args();

    for file in opts.files {
        remcom_single_file(&file, opts.directory.as_deref(), opts.unwatched)?;
    }

    Ok(())
}

fn main() {
    env_logger::init();

    match remcom() {
        Ok(_) => (),
        Err(e) => {
            if e.to_string().contains("Broken pipe") {
            } else {
                panic!("{}", e)
            }
        }
    }
}
