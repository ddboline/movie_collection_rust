use failure::Error;

fn trakt_app() -> Result<(), Error> {
    println!("hello world");
    Ok(())
}

fn main() {
    trakt_app().unwrap();
}
