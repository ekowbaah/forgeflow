use crate::Engine;
use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};

type Shared = Arc<Mutex<Engine>>;
#[derive(Deserialize)]
struct Submit {
    id: String,
    payload: String,
    #[serde(default = "one")]
    max_attempts: u32,
    #[serde(default)]
    available_at_ms: u64,
}
fn one() -> u32 {
    1
}
#[derive(Deserialize)]
struct ClaimRequest {
    worker: String,
    now_ms: u64,
    #[serde(default = "lease")]
    lease_ms: u64,
}
fn lease() -> u64 {
    30_000
}
#[derive(Deserialize)]
struct CompleteRequest {
    token: String,
    success: bool,
    #[serde(default)]
    retry_at_ms: u64,
}
#[derive(Serialize)]
struct Health {
    status: &'static str,
}

pub fn router() -> Router {
    Router::new()
        .route("/health", get(|| async { Json(Health { status: "ok" }) }))
        .route("/ready", get(|| async { Json(Health { status: "ready" }) }))
        .route("/v1/jobs", get(list).post(submit))
        .route("/v1/jobs/claim", post(claim))
        .route("/v1/jobs/{id}/complete", post(complete))
        .route("/v1/jobs/{id}/cancel", post(cancel))
        .route("/v1/audit", get(audit))
        .with_state(Arc::new(Mutex::new(Engine::default())))
}
async fn list(State(engine): State<Shared>) -> Json<Vec<crate::Job>> {
    Json(engine.lock().unwrap().jobs())
}
async fn audit(State(engine): State<Shared>) -> Json<Vec<String>> {
    Json(engine.lock().unwrap().audit().to_vec())
}
async fn submit(State(engine): State<Shared>, Json(body): Json<Submit>) -> StatusCode {
    if engine.lock().unwrap().submit_at(
        body.id,
        body.payload,
        body.max_attempts,
        body.available_at_ms,
    ) {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    }
}
async fn claim(
    State(engine): State<Shared>,
    Json(body): Json<ClaimRequest>,
) -> Result<Json<crate::Claim>, StatusCode> {
    let mut e = engine.lock().unwrap();
    e.promote_due(body.now_ms);
    e.claim_at(&body.worker, body.now_ms, body.lease_ms)
        .map(Json)
        .ok_or(StatusCode::NO_CONTENT)
}
async fn complete(
    Path(id): Path<String>,
    State(engine): State<Shared>,
    Json(body): Json<CompleteRequest>,
) -> StatusCode {
    if engine
        .lock()
        .unwrap()
        .complete_claim(&id, &body.token, body.success, body.retry_at_ms)
    {
        StatusCode::NO_CONTENT
    } else {
        StatusCode::CONFLICT
    }
}
async fn cancel(Path(id): Path<String>, State(engine): State<Shared>) -> StatusCode {
    if engine.lock().unwrap().cancel(&id) {
        StatusCode::NO_CONTENT
    } else {
        StatusCode::CONFLICT
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::{Body, to_bytes},
        http::Request,
    };
    use tower::ServiceExt;
    #[tokio::test]
    async fn submits_idempotently_and_claims() {
        let app = router();
        let payload = r#"{"id":"j","payload":"{}","max_attempts":2}"#;
        for expected in [201, 200] {
            let r = app
                .clone()
                .oneshot(
                    Request::post("/v1/jobs")
                        .header("content-type", "application/json")
                        .body(Body::from(payload))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(r.status(), expected);
        }
        let r = app
            .oneshot(
                Request::post("/v1/jobs/claim")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"worker":"w","now_ms":0}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(r.status(), 200);
        let body = to_bytes(r.into_body(), usize::MAX).await.unwrap();
        assert!(String::from_utf8_lossy(&body).contains("j:w:1"));
    }
}
