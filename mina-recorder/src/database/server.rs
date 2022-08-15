use warp::{
    Filter, Rejection, Reply,
    reply::{WithStatus, Json, self},
    http::StatusCode,
};

use super::rocksdb::DbCore;

fn connections(
    db: DbCore,
) -> impl Filter<Extract = (WithStatus<Json>,), Error = Rejection> + Clone + Sync + Send + 'static {
    warp::path!("connections")
        .and(warp::query::query())
        .map(move |params| -> WithStatus<Json> {
            let v = db.fetch_connections(&params);
            reply::with_status(reply::json(&v.collect::<Vec<_>>()), StatusCode::OK)
        })
}

fn streams(
    db: DbCore,
) -> impl Filter<Extract = (WithStatus<Json>,), Error = Rejection> + Clone + Sync + Send + 'static {
    warp::path!("streams")
        .and(warp::query::query())
        .map(move |params| -> WithStatus<Json> {
            let v = db.fetch_streams(&params);
            reply::with_status(reply::json(&v.collect::<Vec<_>>()), StatusCode::OK)
        })
}

fn messages(
    db: DbCore,
) -> impl Filter<Extract = (WithStatus<Json>,), Error = Rejection> + Clone + Sync + Send + 'static {
    warp::path!("messages")
        .and(warp::query::query())
        .map(move |params| -> WithStatus<Json> {
            let v = db.fetch_messages(&params);
            reply::with_status(reply::json(&v.collect::<Vec<_>>()), StatusCode::OK)
        })
}

fn version(
) -> impl Filter<Extract = (WithStatus<Json>,), Error = Rejection> + Clone + Sync + Send + 'static {
    warp::path!("version")
        .and(warp::query::query())
        .map(move |()| -> reply::WithStatus<Json> {
            reply::with_status(reply::json(&env!("GIT_HASH")), StatusCode::OK)
        })
}

fn openapi(
) -> impl Filter<Extract = (WithStatus<Json>,), Error = Rejection> + Clone + Sync + Send + 'static {
    warp::path!("openapi")
        .and(warp::query::query())
        .map(move |()| -> reply::WithStatus<Json> {
            let s = include_str!("openapi.json");
            let d = serde_json::from_str::<serde_json::Value>(s).unwrap();
            reply::with_status(reply::json(&d), StatusCode::OK)
        })
}

#[allow(dead_code)]
pub fn routes(
    db: DbCore,
) -> impl Filter<Extract = impl Reply, Error = Rejection> + Clone + Sync + Send + 'static {
    use warp::reply::with;

    warp::get()
        .and(
            connections(db.clone())
                .or(streams(db.clone()))
                .or(messages(db))
                .or(version().or(openapi())),
        )
        .with(with::header("Content-Type", "application/json"))
        .with(with::header("Access-Control-Allow-Origin", "*"))
}
