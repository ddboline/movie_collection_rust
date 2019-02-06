#![allow(clippy::needless_pass_by_value)]

extern crate actix;
extern crate actix_web;
extern crate movie_collection_rust;
extern crate rust_auth_server;
extern crate subprocess;

use subprocess::Exec;

use movie_collection_rust::common::config::Config;
use movie_collection_rust::http::movie_queue_app::start_app;

fn main() {
    let config = Config::with_config();
    let command = "rm -f /var/www/html/videos/partial/*";
    Exec::shell(command).join().unwrap();

    let sys = actix::System::new("movie_queue");
    start_app(config);
    let _ = sys.run();
}
