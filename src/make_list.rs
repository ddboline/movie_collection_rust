use movie_collection_lib::common::make_list::make_list;

fn main() {
    env_logger::init();

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
