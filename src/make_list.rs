use movie_collection_rust::common::make_list::make_list;

fn main() {
    match make_list() {
        Ok(_) => {}
        Err(e) => {
            if e.to_string().contains("Broken pipe") {
            } else {
                panic!("{}", e)
            }
        }
    }
}
