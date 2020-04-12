use anyhow::Error;

use movie_collection_lib::{make_list::make_list, stdout_channel::StdoutChannel};

#[tokio::main]
async fn main() -> Result<(), Error> {
    env_logger::init();
    let stdout = StdoutChannel::new();
    let task = stdout.spawn_stdout_task();

    match make_list(&stdout) {
        Ok(_) => {}
        Err(e) => {
            if e.to_string().contains("Broken pipe") {
            } else {
                panic!("{}", e)
            }
        }
    }
    stdout.close().await?;
    task.await?
}
