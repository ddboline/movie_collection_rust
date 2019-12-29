pub mod errors;
pub mod logged_user;
pub mod movie_queue_app;
pub mod movie_queue_requests;
pub mod movie_queue_routes;

pub trait HandleRequest<T> {
    type Result;
    fn handle(&self, req: T) -> Self::Result;
}

#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {
        assert_eq!(2 + 2, 4);
    }
}
