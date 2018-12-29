extern crate failure;
extern crate rayon;

use failure::Error;
use rayon::prelude::*;
use std::env::var;
use std::io::BufRead;
use std::io::BufReader;
use std::path::Path;
use subprocess::Exec;

pub fn map_result_vec<T, E>(input: Vec<Result<T, E>>) -> Result<Vec<T>, E> {
    let mut output: Vec<T> = Vec::new();
    for item in input {
        output.push(item?);
    }
    Ok(output)
}

fn walk_directory(path: &str, match_strs: &[String]) -> Result<Vec<String>, Error> {
    Ok(Path::new(path)
        .read_dir()?
        .filter_map(|f| match f {
            Ok(fpath) => match fpath.file_type() {
                Ok(ftype) => {
                    let path_name = fpath.path().to_str().unwrap().to_string();

                    if ftype.is_dir() {
                        Some(match walk_directory(&path_name, match_strs) {
                            Ok(v) => v,
                            Err(e) => panic!("{} {}", path_name, e),
                        })
                    } else {
                        let path_names: Vec<_> = match_strs
                            .iter()
                            .filter_map(|m| {
                                if path_name.contains(m) {
                                    Some(path_name.clone())
                                } else {
                                    None
                                }
                            })
                            .collect();
                        if path_names.len() > 0 {
                            Some(path_names)
                        } else {
                            None
                        }
                    }
                }
                Err(_) => None,
            },
            Err(_) => None,
        })
        .flatten()
        .collect())
}

fn make_list(do_here: Option<String>) -> Result<(), Error> {
    let suffixes = vec!["avi", "mp4", "mkv"];

    let movie_dirs: Vec<String> = vec![
        "/media/dileptonnas/Documents/movies",
        "/media/dileptonnas/Documents/television",
        "/media/dileptonnas/television/unwatched",
        "/media/sabrent2000/Documents/movies",
        "/media/sabrent2000/Documents/television",
        "/media/sabrent2000/television/unwatched",
        "/media/western2000/Documents/movies",
        "/media/western2000/Documents/television",
        "/media/western2000/television/unwatched",
        "/media/seagate4000/Documents/movies",
        "/media/seagate4000/Documents/television",
        "/media/seagate4000/television/unwatched",
    ]
    .par_iter()
    .filter_map(|d| {
        if Path::new(d).exists() {
            Some(d.to_string())
        } else {
            None
        }
    })
    .collect();

    let home_dir = var("HOME").unwrap_or_else(|_| "/tmp".to_string());

    let movies_dir = match do_here {
        Some(dir) => dir,
        None => format!("{}/Documents/movies", home_dir),
    };

    let path = Path::new(&movies_dir);

    let local_file_list: Vec<_> = path
        .read_dir()?
        .filter_map(|f| match f {
            Ok(fname) => {
                let file_name = fname.file_name().into_string().unwrap();
                for suffix in &suffixes {
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

    let file_list: Vec<_> = movie_dirs
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
    make_list(None).unwrap();
    // make_list(Some(current_dir().unwrap().to_str().unwrap().to_string())).unwrap();
}
