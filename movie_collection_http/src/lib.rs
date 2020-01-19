#![allow(clippy::must_use_candidate)]
#![allow(clippy::too_many_lines)]
#![allow(clippy::module_name_repetitions)]
#![allow(clippy::cast_precision_loss)]
#![allow(clippy::cast_sign_loss)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_possible_wrap)]
#![allow(clippy::similar_names)]
#![allow(clippy::shadow_unrelated)]

pub mod errors;
pub mod logged_user;
pub mod movie_queue_app;
pub mod movie_queue_requests;
pub mod movie_queue_routes;

pub trait HandleRequest<T> {
    type Result;
    fn handle(&self, req: T) -> Self::Result;
}
