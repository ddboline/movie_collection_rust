extern crate failure;
extern crate rayon;

use failure::Error;
use rayon::prelude::*;
use std::env::{current_dir, var};
use std::path::Path;

pub fn map_result_vec<T, E>(input: Vec<Result<T, E>>) -> Result<Vec<T>, E> {
    let mut output: Vec<T> = Vec::new();
    for item in input {
        output.push(item?);
    }
    Ok(output)
}

fn walk_directory(path: &str, match_str: &str) -> Result<Vec<String>, Error> {
    Ok(Path::new(path)
        .read_dir()?
        .filter_map(|f| match f {
            Ok(fpath) => match fpath.file_type() {
                Ok(ftype) => {
                    let path_name = fpath.path().to_str().unwrap().to_string();

                    if ftype.is_dir() {
                        Some(match walk_directory(&path_name, match_str) {
                            Ok(v) => v,
                            Err(e) => panic!("{} {}", path_name, e),
                        })
                    } else {
                        if path_name.contains(match_str) {
                            Some(vec![path_name])
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
    .iter()
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

    let file_list: Vec<_> = path
        .read_dir()?
        .filter_map(|f| match f {
            Ok(fname) => {
                let file_name = fname.file_name().into_string().unwrap();

                for suffix in &suffixes {
                    if file_name.ends_with(suffix) {
                        let extern_files: Vec<String> = movie_dirs
                            .par_iter()
                            .flat_map(|d| walk_directory(&d, &file_name).unwrap())
                            .collect();
                        return Some((file_name, extern_files));
                    }
                }
                None
            }
            Err(_) => None,
        })
        .collect();

    file_list
        .iter()
        .map(|(f, p)| println!("{} {}", f, p.join(" ")))
        .for_each(drop);
    Ok(())
}

fn main() {
    make_list(None).unwrap();
    make_list(Some(current_dir().unwrap().to_str().unwrap().to_string())).unwrap();
}
