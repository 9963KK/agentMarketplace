use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use tokio::net::TcpListener;

use crate::artifact::ArtifactManifest;
use crate::heartbeat::AgentId;
use crate::registry::{AgentIdentity, DiscoveryQuery};
use crate::review::{ReviewId, Verdict};
use crate::settlement::{Balance, HoldId, HoldRequest};
use crate::storage::IdempotencyKey;
use crate::types::{AssignmentId, TaskId, Timestamp};

use super::app::PlatformApp;
use super::types::{AgentToken, AssignRequest, ServerError, SubmittedArtifact};

const AUTHORIZATION: &str = "authorization";
const IDEMPOTENCY_KEY: &str = "idempotency-key";

pub fn router(app: PlatformApp) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/agents/register", post(register_agent))
        .route("/agents/capabilities", put(declare_capabilities))
        .route("/agents/deregister", post(deregister))
        .route("/agents/discover", get(discover))
        .route("/agents/heartbeat", post(ping))
        .route("/agents/{agent_id}/assignments", get(assignments_by_agent))
        .route("/tasks", post(create_task))
        .route("/tasks/{task_id}/participants", post(add_participant))
        .route("/sessions", post(create_session))
        .route("/assignments", post(assign))
        .route("/assignments/{assignment_id}", get(get_assignment))
        .route(
            "/assignments/{assignment_id}/review-assignments",
            get(review_assignments_for_target),
        )
        .route(
            "/assignments/{assignment_id}/artifact",
            put(submit_artifact),
        )
        .route(
            "/assignments/{assignment_id}/artifact-locator",
            get(get_artifact_locator),
        )
        .route("/reviews", post(request_review))
        .route(
            "/reviews/by-assignment/{assignment_id}",
            get(reviews_by_assignment),
        )
        .route("/reviews/{review_id}/verdict", post(submit_review))
        .route("/settlement/deposit", post(deposit))
        .route("/settlement/hold", post(hold))
        .route(
            "/settlement/release-execute-after-reviews",
            post(release_execute_after_reviews),
        )
        .route(
            "/settlement/release-review-after-submission",
            post(release_review_after_submission),
        )
        .route("/settlement/refund", post(refund))
        .route("/settlement/balance", get(balance))
        .with_state(app)
}

pub async fn serve(app: PlatformApp, addr: SocketAddr) -> Result<(), std::io::Error> {
    let listener = TcpListener::bind(addr).await?;
    axum::serve(listener, router(app)).await
}

async fn health() -> Json<HealthResponse> {
    Json(HealthResponse { ok: true })
}

async fn register_agent(
    State(app): State<PlatformApp>,
    Json(request): Json<RegisterAgentRequest>,
) -> Result<Json<super::types::RegisterAgentResponse>, HttpError> {
    let mut identity = AgentIdentity::new(request.agent_id);
    identity.name = request.name;
    identity.endpoint = request.endpoint;
    identity.metadata = request.metadata;
    Ok(Json(app.register_agent(identity, now()).await?))
}

async fn declare_capabilities(
    State(app): State<PlatformApp>,
    headers: HeaderMap,
    Json(request): Json<DeclareCapabilitiesRequest>,
) -> Result<StatusCode, HttpError> {
    app.declare_capabilities(&auth_token(&headers)?, request.capabilities)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn deregister(
    State(app): State<PlatformApp>,
    headers: HeaderMap,
) -> Result<Json<DeregisterResponse>, HttpError> {
    let deregistered = app.deregister(&auth_token(&headers)?, now()).await?;
    Ok(Json(DeregisterResponse { deregistered }))
}

async fn discover(
    State(app): State<PlatformApp>,
    Query(query): Query<DiscoverParams>,
) -> Result<Json<Vec<crate::registry::AgentCandidate>>, HttpError> {
    let include_busy = query.include_busy;
    let limit = query.limit;
    let mut discovery = DiscoveryQuery::new(query.capability()?);
    if let Some(include_busy) = include_busy {
        discovery = discovery.include_busy(include_busy);
    }
    if let Some(limit) = limit {
        discovery = discovery.limit(limit);
    }
    Ok(Json(app.discover(discovery).await?))
}

async fn ping(
    State(app): State<PlatformApp>,
    headers: HeaderMap,
    Json(request): Json<PingRequest>,
) -> Result<Json<super::types::PingResponse>, HttpError> {
    Ok(Json(app.ping(&auth_token(&headers)?, request.busy).await?))
}

async fn assignments_by_agent(
    State(app): State<PlatformApp>,
    headers: HeaderMap,
    Path(agent_id): Path<AgentId>,
) -> Result<Json<Vec<crate::livesession::Assignment>>, HttpError> {
    let caller = app.authenticate(&auth_token(&headers)?).await?;
    if caller != agent_id {
        return Err(ServerError::Forbidden {
            agent_id: caller,
            action: "query another agent assignments",
        }
        .into());
    }
    Ok(Json(
        app.assignments_for_self(&auth_token(&headers)?).await?,
    ))
}

async fn create_task(
    State(app): State<PlatformApp>,
    headers: HeaderMap,
) -> Result<Json<TaskResponse>, HttpError> {
    let task_id = app
        .create_task(&auth_token(&headers)?, idempotency_key(&headers)?, now())
        .await?;
    Ok(Json(TaskResponse { task_id }))
}

async fn add_participant(
    State(app): State<PlatformApp>,
    headers: HeaderMap,
    Path(task_id): Path<TaskId>,
    Json(request): Json<AddParticipantRequest>,
) -> Result<StatusCode, HttpError> {
    app.add_participant(
        &auth_token(&headers)?,
        idempotency_key(&headers)?,
        task_id,
        request.agent_id,
        now(),
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn create_session(
    State(app): State<PlatformApp>,
    headers: HeaderMap,
    Json(request): Json<CreateSessionRequest>,
) -> Result<Json<super::types::CreatedSession>, HttpError> {
    Ok(Json(
        app.create_session(
            &auth_token(&headers)?,
            idempotency_key(&headers)?,
            request.task_id,
            now(),
        )
        .await?,
    ))
}

async fn assign(
    State(app): State<PlatformApp>,
    headers: HeaderMap,
    Json(request): Json<AssignRequest>,
) -> Result<Json<super::types::CreatedAssignment>, HttpError> {
    Ok(Json(
        app.assign(
            &auth_token(&headers)?,
            idempotency_key(&headers)?,
            request,
            now(),
        )
        .await?,
    ))
}

async fn get_assignment(
    State(app): State<PlatformApp>,
    Path(assignment_id): Path<AssignmentId>,
) -> Result<Json<crate::livesession::Assignment>, HttpError> {
    Ok(Json(app.get_assignment(assignment_id).await?))
}

async fn review_assignments_for_target(
    State(app): State<PlatformApp>,
    Path(assignment_id): Path<AssignmentId>,
) -> Result<Json<Vec<crate::livesession::Assignment>>, HttpError> {
    Ok(Json(
        app.review_assignments_for_target(assignment_id).await?,
    ))
}

async fn submit_artifact(
    State(app): State<PlatformApp>,
    headers: HeaderMap,
    Path(assignment_id): Path<AssignmentId>,
    Json(request): Json<SubmitArtifactRequest>,
) -> Result<Json<SubmittedArtifact>, HttpError> {
    Ok(Json(
        app.submit_artifact(
            &auth_token(&headers)?,
            idempotency_key(&headers)?,
            assignment_id,
            request.manifest,
            request.manifest_uri,
            now(),
        )
        .await?,
    ))
}

async fn get_artifact_locator(
    State(app): State<PlatformApp>,
    Path(assignment_id): Path<AssignmentId>,
) -> Result<Json<crate::storage::ArtifactLocator>, HttpError> {
    Ok(Json(app.get_artifact_locator(assignment_id).await?))
}

async fn request_review(
    State(app): State<PlatformApp>,
    headers: HeaderMap,
    Json(request): Json<super::types::ReviewRequest>,
) -> Result<Json<super::types::RequestedReview>, HttpError> {
    Ok(Json(
        app.request_review(
            &auth_token(&headers)?,
            idempotency_key(&headers)?,
            request,
            now(),
        )
        .await?,
    ))
}

async fn reviews_by_assignment(
    State(app): State<PlatformApp>,
    Path(assignment_id): Path<AssignmentId>,
) -> Result<Json<Vec<crate::review::ReviewSession>>, HttpError> {
    Ok(Json(
        app.collect_reviews_by_assignment(assignment_id).await?,
    ))
}

async fn submit_review(
    State(app): State<PlatformApp>,
    headers: HeaderMap,
    Path(review_id): Path<ReviewId>,
    Json(request): Json<SubmitReviewRequest>,
) -> Result<StatusCode, HttpError> {
    app.submit_review(
        &auth_token(&headers)?,
        idempotency_key(&headers)?,
        review_id,
        request.review_assignment_id,
        request.verdict,
        now(),
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn deposit(
    State(app): State<PlatformApp>,
    headers: HeaderMap,
    Json(request): Json<DepositRequest>,
) -> Result<StatusCode, HttpError> {
    app.deposit(
        &auth_token(&headers)?,
        idempotency_key(&headers)?,
        request.amount,
        now(),
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn hold(
    State(app): State<PlatformApp>,
    headers: HeaderMap,
    Json(request): Json<HoldRequest>,
) -> Result<Json<super::types::CreatedHold>, HttpError> {
    Ok(Json(
        app.hold(
            &auth_token(&headers)?,
            idempotency_key(&headers)?,
            request,
            now(),
        )
        .await?,
    ))
}

async fn release_execute_after_reviews(
    State(app): State<PlatformApp>,
    headers: HeaderMap,
    Json(request): Json<ReleaseExecuteRequest>,
) -> Result<StatusCode, HttpError> {
    app.release_execute_after_reviews(
        &auth_token(&headers)?,
        idempotency_key(&headers)?,
        request.hold_id,
        now(),
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn release_review_after_submission(
    State(app): State<PlatformApp>,
    headers: HeaderMap,
    Json(request): Json<ReleaseReviewRequest>,
) -> Result<StatusCode, HttpError> {
    app.release_review_after_submission(
        &auth_token(&headers)?,
        idempotency_key(&headers)?,
        request.hold_id,
        request.review_id,
        now(),
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn refund(
    State(app): State<PlatformApp>,
    headers: HeaderMap,
    Json(request): Json<RefundRequest>,
) -> Result<StatusCode, HttpError> {
    app.refund(
        &auth_token(&headers)?,
        idempotency_key(&headers)?,
        request.hold_id,
        now(),
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn balance(
    State(app): State<PlatformApp>,
    headers: HeaderMap,
) -> Result<Json<BalanceResponse>, HttpError> {
    Ok(Json(BalanceResponse {
        balance: app.balance(&auth_token(&headers)?).await?,
    }))
}

fn auth_token(headers: &HeaderMap) -> Result<AgentToken, HttpError> {
    let value = headers
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .ok_or(ServerError::Unauthorized)?;
    let Some(token) = value.strip_prefix("Bearer ") else {
        return Err(ServerError::Unauthorized.into());
    };

    Ok(AgentToken::from(token.to_string()))
}

fn idempotency_key(headers: &HeaderMap) -> Result<IdempotencyKey, HttpError> {
    let value = headers
        .get(IDEMPOTENCY_KEY)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| ServerError::component("http", "missing Idempotency-Key header"))?;
    Ok(IdempotencyKey::from(value.to_string()))
}

fn now() -> Timestamp {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u128::from(u64::MAX)) as u64;
    Timestamp(millis)
}

#[derive(Debug)]
struct HttpError(ServerError);

impl From<ServerError> for HttpError {
    fn from(error: ServerError) -> Self {
        Self(error)
    }
}

impl IntoResponse for HttpError {
    fn into_response(self) -> Response {
        let status = match self.0 {
            ServerError::Unauthorized => StatusCode::UNAUTHORIZED,
            ServerError::Forbidden { .. } => StatusCode::FORBIDDEN,
            ServerError::NotFound(_) => StatusCode::NOT_FOUND,
            ServerError::IdempotencyInProgress => StatusCode::CONFLICT,
            ServerError::BadRequest(_)
            | ServerError::InvalidReplay { .. }
            | ServerError::InvalidAssignmentKind { .. }
            | ServerError::MissingAssignmentOutput(_) => StatusCode::BAD_REQUEST,
            ServerError::Startup(_) | ServerError::Component { .. } => {
                StatusCode::INTERNAL_SERVER_ERROR
            }
        };

        (
            status,
            Json(ErrorResponse {
                error: self.0.to_string(),
            }),
        )
            .into_response()
    }
}

#[derive(Deserialize)]
struct AddParticipantRequest {
    agent_id: AgentId,
}

#[derive(Deserialize)]
struct CreateSessionRequest {
    task_id: TaskId,
}

#[derive(Deserialize)]
struct DeclareCapabilitiesRequest {
    capabilities: Vec<crate::registry::Capability>,
}

#[derive(Deserialize)]
struct DepositRequest {
    amount: u64,
}

#[derive(Deserialize)]
struct DiscoverParams {
    cap: Option<String>,
    capability: Option<String>,
    include_busy: Option<bool>,
    limit: Option<usize>,
}

impl DiscoverParams {
    fn capability(self) -> Result<String, ServerError> {
        self.cap
            .or(self.capability)
            .ok_or_else(|| ServerError::BadRequest("missing cap or capability query".to_string()))
    }
}

#[derive(Serialize)]
struct BalanceResponse {
    balance: Balance,
}

#[derive(Serialize)]
struct DeregisterResponse {
    deregistered: bool,
}

#[derive(Serialize)]
struct ErrorResponse {
    error: String,
}

#[derive(Serialize)]
struct HealthResponse {
    ok: bool,
}

#[derive(Deserialize)]
struct PingRequest {
    busy: bool,
}

#[derive(Deserialize)]
struct RefundRequest {
    hold_id: HoldId,
}

#[derive(Deserialize)]
struct RegisterAgentRequest {
    agent_id: AgentId,
    name: Option<String>,
    endpoint: Option<String>,
    #[serde(default)]
    metadata: BTreeMap<String, String>,
}

#[derive(Deserialize)]
struct ReleaseExecuteRequest {
    hold_id: HoldId,
}

#[derive(Deserialize)]
struct ReleaseReviewRequest {
    hold_id: HoldId,
    review_id: ReviewId,
}

#[derive(Deserialize)]
struct SubmitArtifactRequest {
    manifest: ArtifactManifest,
    manifest_uri: String,
}

#[derive(Deserialize)]
struct SubmitReviewRequest {
    review_assignment_id: AssignmentId,
    verdict: Verdict,
}

#[derive(Serialize)]
struct TaskResponse {
    task_id: TaskId,
}
