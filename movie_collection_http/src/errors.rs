use anyhow::Error as AnyhowError;
use handlebars::RenderError;
use stack_string::StackString;
use std::{fmt::Debug, io::Error as IoError};
use thiserror::Error;

use crate::logged_user::TRIGGER_DB_UPDATE;

#[derive(Error, Debug)]
#[allow(clippy::used_underscore_binding)]
pub enum ServiceError {
    #[error("Internal Server Error")]
    InternalServerError,
    #[error("BadRequest: {0}")]
    BadRequest(StackString),
    #[error("Unauthorized")]
    Unauthorized,
    #[error("Anyhow error {0}")]
    AnyhowError(#[from] AnyhowError),
    #[error("Template Parse Error {0}")]
    RenderError(#[from] RenderError),
    #[error("IoError {0}")]
    IoError(#[from] IoError),
}

impl Reject for ServiceError {}

#[derive(Serialize)]
struct ErrorMessage {
    code: u16,
    message: String,
}

pub async fn error_response(err: Rejection) -> Result<Box<dyn Reply>, Infallible> {
    let code: StatusCode;
    let message: &str;

    if err.is_not_found() {
        code = StatusCode::NOT_FOUND;
        message = "NOT FOUND";
    } else if let Some(missing_cookie) = err.find::<MissingCookie>() {
        if missing_cookie.name() == "jwt" {
            TRIGGER_DB_UPDATE.set();
            return Ok(Box::new(login_html()));
        }
        code = StatusCode::INTERNAL_SERVER_ERROR;
        message = "Internal Server Error";
    } else if let Some(service_err) = err.find::<ServiceError>() {
        match service_err {
            ServiceError::BadRequest(msg) => {
                code = StatusCode::BAD_REQUEST;
                message = msg.as_str();
            }
            ServiceError::Unauthorized => {
                TRIGGER_DB_UPDATE.set();
                return Ok(Box::new(login_html()));
            }
            _ => {
                error!("Other error: {:?}", service_err);
                code = StatusCode::INTERNAL_SERVER_ERROR;
                message = "Internal Server Error, Please try again later";
            }
        }
    } else if err.find::<warp::reject::MethodNotAllowed>().is_some() {
        code = StatusCode::METHOD_NOT_ALLOWED;
        message = "METHOD NOT ALLOWED";
    } else {
        error!("Unknown error: {:?}", err);
        code = StatusCode::INTERNAL_SERVER_ERROR;
        message = "Internal Server Error, Please try again later";
    };

    let reply = warp::reply::json(&ErrorMessage {
        code: code.as_u16(),
        message: message.to_string(),
    });
    let reply = warp::reply::with_status(reply, code);

    Ok(Box::new(reply))
}

fn login_html() -> impl Reply {
    warp::reply::html(
        "
            <script>
                !function() {
                    let final_url = location.href;
                    location.replace('/auth/login.html?final_url=' + final_url);
                }()
            </script>
        ",
    )
}

#[cfg(test)]
mod test {
    use anyhow::Error;
    use warp::Reply;

    use crate::errors::{error_response, ServiceError};

    #[tokio::test]
    async fn test_service_error() -> Result<(), Error> {
        let err = ServiceError::BadRequest("TEST ERROR".into()).into();
        let resp = error_response(err).await?.into_response();
        assert_eq!(resp.status().as_u16(), 400);

        let err = ServiceError::InternalServerError.into();
        let resp = error_response(err).await?.into_response();
        assert_eq!(resp.status().as_u16(), 500);
        Ok(())
    }
}
