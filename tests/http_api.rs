use agent_marketplace::server::{PlatformApp, http};
use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

fn json_request(method: &str, uri: &str, body: Value) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

fn empty_request(method: &str, uri: &str) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(uri)
        .body(Body::empty())
        .unwrap()
}

async fn response_json(response: axum::response::Response) -> Value {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap_or(Value::Null)
}

async fn register(
    router: Router,
    agent_id: &str,
    registration_token: Option<&str>,
) -> (StatusCode, Value) {
    let mut request = json_request(
        "POST",
        "/agents/register",
        json!({
            "agent_id": agent_id,
            "name": agent_id,
            "endpoint": null,
            "metadata": {}
        }),
    );
    if let Some(token) = registration_token {
        request
            .headers_mut()
            .insert("registration-token", token.parse().unwrap());
    }
    let response = router.oneshot(request).await.unwrap();
    let status = response.status();
    (status, response_json(response).await)
}

async fn declare_capability(router: Router, token: &str, capability: &str) {
    let mut request = json_request(
        "PUT",
        "/agents/capabilities",
        json!({
            "capabilities": [{
                "name": capability,
                "max_concurrency": 1,
                "contract": null
            }]
        }),
    );
    request
        .headers_mut()
        .insert("authorization", format!("Bearer {token}").parse().unwrap());
    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
}

async fn ping(router: Router, token: &str) {
    let mut request = json_request("POST", "/agents/heartbeat", json!({ "busy": false }));
    request
        .headers_mut()
        .insert("authorization", format!("Bearer {token}").parse().unwrap());
    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn register_requires_invite_token_and_owner_proof_for_existing_agent() {
    let app =
        PlatformApp::spawn_with_registration_token(Some("invite-secret".to_string())).unwrap();
    let router = http::router(app.clone());

    let (status, _) = register(router.clone(), "agent-1", None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let (status, first) = register(router.clone(), "agent-1", Some("invite-secret")).await;
    assert_eq!(status, StatusCode::OK);
    let token = first["token"].as_str().unwrap();

    let (status, _) = register(router.clone(), "agent-1", None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let mut request = json_request(
        "POST",
        "/agents/register",
        json!({
            "agent_id": "agent-1",
            "name": "agent-1-updated",
            "endpoint": null,
            "metadata": {}
        }),
    );
    request
        .headers_mut()
        .insert("authorization", format!("Bearer {token}").parse().unwrap());
    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    app.shutdown().await;
}

#[tokio::test]
async fn list_agents_is_registry_directory_and_discover_is_online_capability_filter() {
    let app = PlatformApp::spawn().unwrap();
    let router = http::router(app.clone());

    let (_, executor) = register(router.clone(), "executor", None).await;
    let executor_token = executor["token"].as_str().unwrap();
    let (_, reviewer) = register(router.clone(), "reviewer", None).await;
    let reviewer_token = reviewer["token"].as_str().unwrap();
    declare_capability(router.clone(), executor_token, "execute").await;
    declare_capability(router.clone(), reviewer_token, "review").await;
    ping(router.clone(), executor_token).await;

    let response = router
        .clone()
        .oneshot(empty_request("GET", "/agents"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let agents = response_json(response).await;
    assert_eq!(agents.as_array().unwrap().len(), 2);
    assert_eq!(agents[0]["agent_id"], "executor");
    assert_eq!(agents[0]["alive"], true);
    assert_eq!(agents[1]["agent_id"], "reviewer");
    assert_eq!(agents[1]["alive"], false);

    let response = router
        .clone()
        .oneshot(empty_request("GET", "/agents?alive_only=true"))
        .await
        .unwrap();
    let alive_agents = response_json(response).await;
    assert_eq!(alive_agents.as_array().unwrap().len(), 1);
    assert_eq!(alive_agents[0]["agent_id"], "executor");

    let response = router
        .oneshot(empty_request("GET", "/agents/discover?cap=execute"))
        .await
        .unwrap();
    let candidates = response_json(response).await;
    assert_eq!(candidates.as_array().unwrap().len(), 1);
    assert_eq!(candidates[0]["agent_id"], "executor");

    app.shutdown().await;
}

#[tokio::test]
async fn write_operations_require_idempotency_key() {
    let app = PlatformApp::spawn().unwrap();
    let router = http::router(app.clone());
    let (_, publisher) = register(router.clone(), "publisher", None).await;
    let token = publisher["token"].as_str().unwrap();

    let mut request = empty_request("POST", "/tasks");
    request
        .headers_mut()
        .insert("authorization", format!("Bearer {token}").parse().unwrap());
    let response = router.oneshot(request).await.unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        response_json(response).await["error"],
        "bad request: missing Idempotency-Key header"
    );

    app.shutdown().await;
}
