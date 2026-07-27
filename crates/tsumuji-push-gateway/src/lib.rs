use axum::{
    extract::{Path, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post, put},
    Json, Router,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use rand::{rngs::OsRng, Rng, RngCore};
use reqwest::Client;
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fmt, fs, io,
    net::SocketAddr,
    path::{Path as FilePath, PathBuf},
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::sync::Mutex;

const PAIRING_ALPHABET: &[u8] = b"23456789ABCDEFGHJKMNPQRSTUVWXYZ";
const PAIRING_CODE_LENGTH: usize = 10;
const DEVICE_ID_BYTES: usize = 16;
const DEVICE_CREDENTIAL_BYTES: usize = 32;
const CALL_TOKEN_BYTES: usize = 32;
const MAX_FID_LENGTH: usize = 256;
const OAUTH_SCOPE: &str = "https://www.googleapis.com/auth/firebase.messaging";

#[derive(Clone)]
pub struct GatewayState {
    database: PathBuf,
    fcm: Option<Arc<FcmClient>>,
}

impl GatewayState {
    /// Opens the gateway database and optionally configures FCM.
    ///
    /// # Errors
    ///
    /// Returns an error when the database or service-account file cannot be opened.
    pub fn new(
        database: PathBuf,
        service_account: Option<&FilePath>,
    ) -> Result<Self, GatewayError> {
        initialize_database(&database)?;
        let fcm = service_account
            .map(FcmClient::from_file)
            .transpose()?
            .map(Arc::new);
        Ok(Self { database, fcm })
    }
}

#[derive(Debug)]
pub struct GatewayError {
    public_message: &'static str,
    detail: String,
}

impl GatewayError {
    fn new(public_message: &'static str, detail: impl Into<String>) -> Self {
        Self {
            public_message,
            detail: detail.into(),
        }
    }
}

impl fmt::Display for GatewayError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.detail)
    }
}

impl std::error::Error for GatewayError {}

#[derive(Serialize)]
struct ErrorBody {
    error: &'static str,
}

impl IntoResponse for GatewayError {
    fn into_response(self) -> Response {
        eprintln!("gateway request failed: {}", self.detail);
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorBody {
                error: self.public_message,
            }),
        )
            .into_response()
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaimRequest {
    code: String,
    fid: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaimResponse {
    pub device_id: String,
    pub device_credential: String,
}

#[derive(Debug, Deserialize)]
pub struct FidRequest {
    fid: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WakeRequest {
    device_id: String,
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    expires_in: i64,
}

#[derive(Debug, Serialize)]
struct StatusResponse {
    status: &'static str,
}

pub fn public_router(state: GatewayState) -> Router {
    Router::new()
        .route("/healthz", get(health))
        .route("/v1/pairings/claim", post(claim_pairing))
        .route("/v1/devices/:device_id/fid", put(update_fid))
        .with_state(state)
}

pub fn internal_router(state: GatewayState) -> Router {
    Router::new()
        .route("/healthz", get(health))
        .route("/internal/v1/wake", post(send_wake))
        .with_state(state)
}

async fn health() -> Json<StatusResponse> {
    Json(StatusResponse { status: "ok" })
}

async fn claim_pairing(
    State(state): State<GatewayState>,
    Json(request): Json<ClaimRequest>,
) -> Result<Json<ClaimResponse>, GatewayError> {
    validate_fid(&request.fid)?;
    let code = normalize_pairing_code(&request.code)?;
    let database = state.database.clone();
    let fid = request.fid;
    let response = tokio::task::spawn_blocking(move || claim(&database, &code, &fid))
        .await
        .map_err(|error| GatewayError::new("pairing failed", error.to_string()))??;
    Ok(Json(response))
}

async fn update_fid(
    State(state): State<GatewayState>,
    Path(device_id): Path<String>,
    headers: HeaderMap,
    Json(request): Json<FidRequest>,
) -> Result<Json<StatusResponse>, GatewayError> {
    validate_device_id(&device_id)?;
    validate_fid(&request.fid)?;
    let credential = bearer_credential(&headers)?;
    let database = state.database.clone();
    tokio::task::spawn_blocking(move || {
        update_device_fid(&database, &device_id, &credential, &request.fid)
    })
    .await
    .map_err(|error| GatewayError::new("device update failed", error.to_string()))??;
    Ok(Json(StatusResponse { status: "updated" }))
}

async fn send_wake(
    State(state): State<GatewayState>,
    Json(request): Json<WakeRequest>,
) -> Result<Json<StatusResponse>, GatewayError> {
    validate_device_id(&request.device_id)?;
    let database = state.database.clone();
    let device_id = request.device_id;
    let fid = tokio::task::spawn_blocking(move || load_fid(&database, &device_id))
        .await
        .map_err(|error| GatewayError::new("wake failed", error.to_string()))??;
    let fcm = state
        .fcm
        .ok_or_else(|| GatewayError::new("wake failed", "FCM service account is not configured"))?;
    fcm.send_wake(&fid).await?;
    Ok(Json(StatusResponse { status: "sent" }))
}

/// Creates a short-lived, single-use code bound to one SIP user.
///
/// # Errors
///
/// Returns an error for an invalid SIP user, invalid TTL, or database failure.
pub fn issue_pairing_code(
    database: &FilePath,
    sip_user: &str,
    ttl: Duration,
) -> Result<String, GatewayError> {
    validate_sip_user(sip_user)?;
    if ttl.is_zero() {
        return Err(GatewayError::new(
            "pairing code could not be created",
            "pairing code TTL must be greater than zero",
        ));
    }
    initialize_database(database)?;
    let code = random_pairing_code();
    let code_hash = hash(code.as_bytes());
    let now = unix_time()?;
    let expires_at = now
        .checked_add(i64::try_from(ttl.as_secs()).map_err(|error| {
            GatewayError::new("pairing code could not be created", error.to_string())
        })?)
        .ok_or_else(|| GatewayError::new("pairing code could not be created", "expiry overflow"))?;
    let connection = open_database(database)?;
    connection
        .execute(
            "DELETE FROM pairing_codes WHERE expires_at <= ?1 OR consumed_at IS NOT NULL",
            params![now],
        )
        .map_err(db_error)?;
    connection
        .execute(
            "INSERT INTO pairing_codes(code_hash, sip_user, expires_at) VALUES (?1, ?2, ?3)",
            params![code_hash.as_slice(), sip_user, expires_at],
        )
        .map_err(db_error)?;
    Ok(format!("{}-{}", &code[..5], &code[5..]))
}

fn claim(database: &FilePath, code: &str, fid: &str) -> Result<ClaimResponse, GatewayError> {
    let mut connection = open_database(database)?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(db_error)?;
    let now = unix_time()?;
    let code_hash = hash(code.as_bytes());
    let pairing = transaction
        .query_row(
            "SELECT sip_user, expires_at, consumed_at
             FROM pairing_codes WHERE code_hash = ?1",
            params![code_hash.as_slice()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, Option<i64>>(2)?,
                ))
            },
        )
        .optional()
        .map_err(db_error)?;
    let Some((sip_user, expires_at, consumed_at)) = pairing else {
        return Err(GatewayError::new(
            "pairing code is invalid",
            "unknown pairing code",
        ));
    };
    if consumed_at.is_some() || expires_at <= now {
        return Err(GatewayError::new(
            "pairing code is invalid",
            "expired or consumed pairing code",
        ));
    }

    let device_id = random_urlsafe(DEVICE_ID_BYTES);
    let device_credential = random_urlsafe(DEVICE_CREDENTIAL_BYTES);
    let credential_hash = hash(device_credential.as_bytes());
    transaction
        .execute("DELETE FROM devices WHERE sip_user = ?1", params![sip_user])
        .map_err(db_error)?;
    transaction
        .execute(
            "INSERT INTO devices(device_id, sip_user, credential_hash, fid, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![device_id, sip_user, credential_hash.as_slice(), fid, now],
        )
        .map_err(db_error)?;
    transaction
        .execute(
            "UPDATE pairing_codes SET consumed_at = ?1 WHERE code_hash = ?2",
            params![now, code_hash.as_slice()],
        )
        .map_err(db_error)?;
    transaction.commit().map_err(db_error)?;
    Ok(ClaimResponse {
        device_id,
        device_credential,
    })
}

fn update_device_fid(
    database: &FilePath,
    device_id: &str,
    credential: &str,
    fid: &str,
) -> Result<(), GatewayError> {
    validate_credential(credential)?;
    let connection = open_database(database)?;
    let credential_hash = hash(credential.as_bytes());
    let changed = connection
        .execute(
            "UPDATE devices SET fid = ?1, updated_at = ?2
             WHERE device_id = ?3 AND credential_hash = ?4",
            params![fid, unix_time()?, device_id, credential_hash.as_slice()],
        )
        .map_err(db_error)?;
    if changed != 1 {
        return Err(GatewayError::new(
            "device authentication failed",
            "unknown device or invalid credential",
        ));
    }
    Ok(())
}

fn load_fid(database: &FilePath, device_id: &str) -> Result<String, GatewayError> {
    let connection = open_database(database)?;
    connection
        .query_row(
            "SELECT fid FROM devices WHERE device_id = ?1",
            params![device_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(db_error)?
        .ok_or_else(|| GatewayError::new("device not found", "unknown device ID"))
}

fn initialize_database(path: &FilePath) -> Result<(), GatewayError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(io_error)?;
    }
    let connection = open_database(path)?;
    connection
        .execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA foreign_keys = ON;
             CREATE TABLE IF NOT EXISTS pairing_codes (
               code_hash BLOB PRIMARY KEY,
               sip_user TEXT NOT NULL,
               expires_at INTEGER NOT NULL,
               consumed_at INTEGER
             );
             CREATE TABLE IF NOT EXISTS devices (
               device_id TEXT PRIMARY KEY,
               sip_user TEXT NOT NULL UNIQUE,
               credential_hash BLOB NOT NULL,
               fid TEXT NOT NULL,
               updated_at INTEGER NOT NULL
             );",
        )
        .map_err(db_error)?;
    #[cfg(unix)]
    set_private_permissions(path)?;
    Ok(())
}

fn open_database(path: &FilePath) -> Result<Connection, GatewayError> {
    Connection::open(path).map_err(db_error)
}

#[cfg(unix)]
fn set_private_permissions(path: &FilePath) -> Result<(), GatewayError> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).map_err(io_error)
}

fn validate_sip_user(value: &str) -> Result<(), GatewayError> {
    if !(1..=64).contains(&value.len())
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._~-".contains(&byte))
    {
        return Err(GatewayError::new(
            "pairing code could not be created",
            "invalid SIP user",
        ));
    }
    Ok(())
}

fn normalize_pairing_code(value: &str) -> Result<String, GatewayError> {
    let normalized = value
        .bytes()
        .filter(|byte| *byte != b'-' && !byte.is_ascii_whitespace())
        .map(|byte| byte.to_ascii_uppercase())
        .collect::<Vec<_>>();
    if normalized.len() != PAIRING_CODE_LENGTH
        || !normalized
            .iter()
            .all(|byte| PAIRING_ALPHABET.contains(byte))
    {
        return Err(GatewayError::new(
            "pairing code is invalid",
            "malformed pairing code",
        ));
    }
    String::from_utf8(normalized)
        .map_err(|error| GatewayError::new("pairing code is invalid", error.to_string()))
}

fn validate_fid(fid: &str) -> Result<(), GatewayError> {
    if fid.is_empty()
        || fid.len() > MAX_FID_LENGTH
        || !fid
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_:-".contains(&byte))
    {
        return Err(GatewayError::new("invalid FCM identifier", "malformed FID"));
    }
    Ok(())
}

fn validate_device_id(device_id: &str) -> Result<(), GatewayError> {
    let decoded = URL_SAFE_NO_PAD
        .decode(device_id)
        .map_err(|error| GatewayError::new("invalid device ID", error.to_string()))?;
    if decoded.len() != DEVICE_ID_BYTES {
        return Err(GatewayError::new(
            "invalid device ID",
            "invalid device ID length",
        ));
    }
    Ok(())
}

fn validate_credential(credential: &str) -> Result<(), GatewayError> {
    let decoded = URL_SAFE_NO_PAD
        .decode(credential)
        .map_err(|error| GatewayError::new("device authentication failed", error.to_string()))?;
    if decoded.len() != DEVICE_CREDENTIAL_BYTES {
        return Err(GatewayError::new(
            "device authentication failed",
            "invalid credential length",
        ));
    }
    Ok(())
}

fn bearer_credential(headers: &HeaderMap) -> Result<String, GatewayError> {
    let value = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .ok_or_else(|| {
            GatewayError::new("device authentication failed", "missing bearer credential")
        })?;
    validate_credential(value)?;
    Ok(value.to_owned())
}

fn random_pairing_code() -> String {
    let mut random = OsRng;
    (0..PAIRING_CODE_LENGTH)
        .map(|_| char::from(PAIRING_ALPHABET[random.gen_range(0..PAIRING_ALPHABET.len())]))
        .collect()
}

fn random_urlsafe(length: usize) -> String {
    let mut bytes = vec![0_u8; length];
    OsRng.fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

fn hash(value: &[u8]) -> [u8; 32] {
    Sha256::digest(value).into()
}

fn unix_time() -> Result<i64, GatewayError> {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| GatewayError::new("clock error", error.to_string()))?
        .as_secs();
    i64::try_from(seconds).map_err(|error| GatewayError::new("clock error", error.to_string()))
}

#[allow(clippy::needless_pass_by_value)]
fn db_error(error: rusqlite::Error) -> GatewayError {
    GatewayError::new("gateway storage failed", error.to_string())
}

#[allow(clippy::needless_pass_by_value)]
fn io_error(error: io::Error) -> GatewayError {
    GatewayError::new("gateway storage failed", error.to_string())
}

#[derive(Debug, Deserialize)]
struct ServiceAccount {
    project_id: String,
    client_email: String,
    private_key: String,
    token_uri: String,
}

#[derive(Debug, Serialize)]
struct JwtClaims<'a> {
    iss: &'a str,
    scope: &'static str,
    aud: &'a str,
    iat: i64,
    exp: i64,
}

struct CachedToken {
    value: String,
    expires_at: i64,
}

struct FcmClient {
    account: ServiceAccount,
    signing_key: EncodingKey,
    http: Client,
    token: Mutex<Option<CachedToken>>,
}

impl FcmClient {
    fn from_file(path: &FilePath) -> Result<Self, GatewayError> {
        let bytes = fs::read(path)
            .map_err(|error| GatewayError::new("FCM configuration failed", error.to_string()))?;
        let account: ServiceAccount = serde_json::from_slice(&bytes)
            .map_err(|error| GatewayError::new("FCM configuration failed", error.to_string()))?;
        if account.project_id.is_empty()
            || account.client_email.is_empty()
            || account.token_uri.is_empty()
        {
            return Err(GatewayError::new(
                "FCM configuration failed",
                "service account is missing required values",
            ));
        }
        let signing_key = EncodingKey::from_rsa_pem(account.private_key.as_bytes())
            .map_err(|error| GatewayError::new("FCM configuration failed", error.to_string()))?;
        Ok(Self {
            account,
            signing_key,
            http: Client::new(),
            token: Mutex::new(None),
        })
    }

    async fn send_wake(&self, fid: &str) -> Result<(), GatewayError> {
        let access_token = self.access_token().await?;
        let call_token = random_urlsafe(CALL_TOKEN_BYTES);
        let response = self
            .http
            .post(format!(
                "https://fcm.googleapis.com/v1/projects/{}/messages:send",
                self.account.project_id
            ))
            .bearer_auth(access_token)
            .json(&serde_json::json!({
                "message": {
                    "fid": fid,
                    "data": {
                        "type": "wake",
                        "callToken": call_token
                    },
                    "android": {
                        "priority": "high",
                        "ttl": "30s"
                    }
                }
            }))
            .send()
            .await
            .map_err(|error| GatewayError::new("wake failed", error.to_string()))?;
        if !response.status().is_success() {
            return Err(GatewayError::new(
                "wake failed",
                format!("FCM returned HTTP {}", response.status()),
            ));
        }
        Ok(())
    }

    async fn access_token(&self) -> Result<String, GatewayError> {
        let now = unix_time()?;
        let mut cache = self.token.lock().await;
        if let Some(token) = cache.as_ref() {
            if token.expires_at > now + 60 {
                return Ok(token.value.clone());
            }
        }
        let claims = JwtClaims {
            iss: &self.account.client_email,
            scope: OAUTH_SCOPE,
            aud: &self.account.token_uri,
            iat: now,
            exp: now + 3600,
        };
        let assertion =
            jsonwebtoken::encode(&Header::new(Algorithm::RS256), &claims, &self.signing_key)
                .map_err(|error| GatewayError::new("wake failed", error.to_string()))?;
        let response = self
            .http
            .post(&self.account.token_uri)
            .form(&[
                ("grant_type", "urn:ietf:params:oauth:grant-type:jwt-bearer"),
                ("assertion", assertion.as_str()),
            ])
            .send()
            .await
            .map_err(|error| GatewayError::new("wake failed", error.to_string()))?;
        if !response.status().is_success() {
            return Err(GatewayError::new(
                "wake failed",
                format!("OAuth token endpoint returned HTTP {}", response.status()),
            ));
        }
        let response: TokenResponse = response
            .json()
            .await
            .map_err(|error| GatewayError::new("wake failed", error.to_string()))?;
        let expires_at = now + response.expires_in;
        *cache = Some(CachedToken {
            value: response.access_token.clone(),
            expires_at,
        });
        Ok(response.access_token)
    }
}

/// Runs separate public-pairing and internal-wake listeners.
///
/// # Errors
///
/// Returns an error unless both listeners are distinct loopback addresses, or when binding or
/// serving either listener fails.
pub async fn serve(
    state: GatewayState,
    public_address: SocketAddr,
    internal_address: SocketAddr,
) -> Result<(), GatewayError> {
    if !public_address.ip().is_loopback() || !internal_address.ip().is_loopback() {
        return Err(GatewayError::new(
            "gateway startup failed",
            "both listeners must use loopback addresses",
        ));
    }
    if public_address == internal_address {
        return Err(GatewayError::new(
            "gateway startup failed",
            "public and internal listeners must be different",
        ));
    }
    let public_listener = tokio::net::TcpListener::bind(public_address)
        .await
        .map_err(|error| GatewayError::new("gateway startup failed", error.to_string()))?;
    let internal_listener = tokio::net::TcpListener::bind(internal_address)
        .await
        .map_err(|error| GatewayError::new("gateway startup failed", error.to_string()))?;
    println!("pairing API listening on {public_address}");
    println!("internal wake API listening on {internal_address}");
    let public_state = state.clone();
    tokio::try_join!(
        async move {
            axum::serve(public_listener, public_router(public_state))
                .await
                .map_err(|error| GatewayError::new("gateway stopped", error.to_string()))
        },
        async move {
            axum::serve(internal_listener, internal_router(state))
                .await
                .map_err(|error| GatewayError::new("gateway stopped", error.to_string()))
        }
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::{to_bytes, Body},
        http::Request,
    };
    use tempfile::tempdir;
    use tower::ServiceExt;

    #[tokio::test]
    async fn pairing_is_one_time_and_updates_fid_with_credential() {
        let directory = tempdir().unwrap();
        let database = directory.path().join("gateway.sqlite3");
        let state = GatewayState::new(database.clone(), None).unwrap();
        let code = issue_pairing_code(&database, "ts-line3", Duration::from_secs(600)).unwrap();
        let request = Request::builder()
            .method("POST")
            .uri("/v1/pairings/claim")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::json!({"code": code, "fid": "test_fid-123"}).to_string(),
            ))
            .unwrap();
        let response = public_router(state.clone()).oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), 4096).await.unwrap();
        let pairing: ClaimResponse = serde_json::from_slice(&body).unwrap();

        let reused = Request::builder()
            .method("POST")
            .uri("/v1/pairings/claim")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::json!({"code": code, "fid": "another_fid"}).to_string(),
            ))
            .unwrap();
        assert_eq!(
            public_router(state.clone())
                .oneshot(reused)
                .await
                .unwrap()
                .status(),
            StatusCode::BAD_REQUEST
        );

        let update = Request::builder()
            .method("PUT")
            .uri(format!("/v1/devices/{}/fid", pairing.device_id))
            .header("content-type", "application/json")
            .header(
                "authorization",
                format!("Bearer {}", pairing.device_credential),
            )
            .body(Body::from(r#"{"fid":"refreshed_fid"}"#))
            .unwrap();
        assert_eq!(
            public_router(state).oneshot(update).await.unwrap().status(),
            StatusCode::OK
        );
        assert_eq!(
            load_fid(&database, &pairing.device_id).unwrap(),
            "refreshed_fid"
        );
    }

    #[test]
    fn pairing_code_is_human_readable() {
        let directory = tempdir().unwrap();
        let database = directory.path().join("gateway.sqlite3");
        let code = issue_pairing_code(&database, "ts-line3", Duration::from_secs(600)).unwrap();
        assert_eq!(code.len(), 11);
        assert_eq!(code.as_bytes()[5], b'-');
        assert!(code
            .bytes()
            .filter(|byte| *byte != b'-')
            .all(|byte| PAIRING_ALPHABET.contains(&byte)));
    }
}
