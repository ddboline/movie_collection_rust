#![allow(clippy::needless_pass_by_value)]


pub type HttpResult = Result<HttpResponse, Error>;

fn form_http_response(body: String) -> HttpResult {
    Ok(HttpResponse::Ok()
        .content_type("text/html; charset=utf-8")
        .body(body))
}

fn to_json(js: impl Serialize) -> HttpResult {
    Ok(HttpResponse::Ok().json(js))
}