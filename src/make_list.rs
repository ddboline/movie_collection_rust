extern crate dotenv;
extern crate failure;
extern crate movie_collection_rust;
extern crate rayon;

use failure::Error;
use rayon::prelude::*;
use std::io::BufRead;
use std::io::BufReader;
use std::path::Path;
use subprocess::Exec;

use movie_collection_rust::utils::{walk_directory, Config};

fn make_list() -> Result<(), Error> {
    let config = Config::new().with_config();

    let movies_dir = format!("{}/Documents/movies", config.home_dir);

    let path = Path::new(&movies_dir);

    let local_file_list: Vec<_> = path
        .read_dir()?
        .filter_map(|f| match f {
            Ok(fname) => {
                let file_name = fname.file_name().into_string().unwrap();
                for suffix in &config.suffixes {
                    if file_name.ends_with(suffix) {
                        return Some(file_name);
                    }
                }
                None
            }
            Err(_) => None,
        })
        .collect();

    if local_file_list.is_empty() {
        return Ok(());
    }

    let file_list: Vec<_> = config
        .movie_dirs
        .par_iter()
        .flat_map(|d| walk_directory(&d, &local_file_list))
        .flatten()
        .collect();

    file_list
        .iter()
        .map(|f| {
            let file_name = f.split("/").last().unwrap().to_string();
            println!("{} {}", file_name, f);
        })
        .for_each(drop);

    file_list
        .par_iter()
        .map(|f| {
            let command = if f.ends_with(".avi") {
                format!("aviindex -i {} -o /dev/null", f)
            } else {
                format!("ffprobe {} 2>&1", f)
            };

            let mut timeval = "".to_string();

            let stream = Exec::shell(command).stream_stdout().unwrap();
            BufReader::new(stream)
                .lines()
                .map(|l| {
                    let items: Vec<_> = l
                        .unwrap()
                        .split_whitespace()
                        .map(|s| s.to_string())
                        .collect();
                    if items.len() > 5 {
                        if items[1] == "V:" {
                            let nsecs: u64 = items[5]
                                .trim_start_matches("frames=")
                                .trim_matches(',')
                                .parse()
                                .unwrap();
                            let nmin = (nsecs as f64 / 60.) as u64;
                            let nhour = (nmin as f64 / 60.) as u64;
                            timeval = format!("{:02}:{:02}:{:02}", nhour, nmin, nsecs % 60);
                        }
                    }
                    if items.len() > 1 {
                        if items[0] == "Duration:" {
                            let its: Vec<_> = items[1].trim_matches(',').split(":").collect();
                            let nhour: u64 = its[0].parse().unwrap();
                            let nmin: u64 = its[1].parse().unwrap();
                            let nsecs: f64 = its[2].parse().unwrap();
                            timeval = format!("{:02}:{:02}:{:02}", nhour, nmin, nsecs as u64);
                        }
                    }
                })
                .for_each(drop);

            println!("{} {}", timeval, f);
        })
        .for_each(drop);

    Ok(())
}

fn main() {
    make_list().unwrap();
}
