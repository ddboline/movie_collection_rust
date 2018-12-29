use clap::{App, Arg};

fn get_version_number() -> String {
    format!(
        "{}.{}.{}{}",
        env!("CARGO_PKG_VERSION_MAJOR"),
        env!("CARGO_PKG_VERSION_MINOR"),
        env!("CARGO_PKG_VERSION_PATCH"),
        option_env!("CARGO_PKG_VERSION_PRE").unwrap_or("")
    )
}

fn remcom() {
    let matches = App::new("Remcom")
        .version(get_version_number().as_str())
        .author("Daniel Boline <ddboline@gmail.com>")
        .about("Create script to do stuff")
        .arg(
            Arg::with_name("directory")
                .short("d")
                .long("directory")
                .value_name("DIRECTORY")
                .help("Directory"),
        )
        .arg(
            Arg::with_name("unwatched")
                .short("u")
                .long("unwatched")
                .value_name("UNWATCHED")
                .takes_value(false),
        )
        .arg(Arg::with_name("files").multiple(true))
        .get_matches();

    if matches.is_present("unwatched") {
        println!("unwatched");
    }

    if let Some(directory) = matches.value_of("directory") {
        println!("directory {}", directory);
    }

    if let Some(patterns) = matches.values_of("files") {
        let strings: Vec<String> = patterns.map(|x| x.to_string()).collect();
        println!("strings {:?}", strings);
    }
}

fn main() {
    remcom();
}
