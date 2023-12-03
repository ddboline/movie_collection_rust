use anyhow::Error;
use stack_string::StackString;
use stdout_channel::StdoutChannel;

use movie_collection_lib::make_list::make_list;

#[tokio::main]
async fn main() -> Result<(), Error> {
    env_logger::init();
    let stdout = StdoutChannel::new();

    match make_list(&stdout).await {
        Ok(()) => {}
        Err(e) => {
            let e = StackString::from_display(e);
            if e.contains("Broken pipe") {
            } else {
                panic!("{}", e);
            }
        }
    }
    stdout.close().await.map_err(Into::into)
}
