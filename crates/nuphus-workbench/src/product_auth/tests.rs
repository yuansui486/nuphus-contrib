use super::*;
use axum::{
    extract::State as HttpState,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{delete, post},
    Json, Router,
};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU16, AtomicU64, Ordering};

#[derive(Default)]
struct MemoryVault {
    value: Mutex<Option<String>>,
    broken: AtomicBool,
}
impl Vault for MemoryVault {
    fn load(&self) -> Result<Option<String>> {
        Ok(self.value.lock().unwrap().clone())
    }
    fn save(&self, value: &str) -> Result<()> {
        if self.broken.load(Ordering::SeqCst) {
            return Err(error("storage_error", "test vault unavailable"));
        }
        *self.value.lock().unwrap() = Some(value.into());
        Ok(())
    }
    fn clear(&self) -> Result<()> {
        *self.value.lock().unwrap() = None;
        Ok(())
    }
}
struct TestClock {
    wall: AtomicI64,
    tick: AtomicU64,
}
impl TestClock {
    fn new() -> Self {
        Self {
            wall: AtomicI64::new(1_800_000_000),
            tick: AtomicU64::new(0),
        }
    }
    fn advance(&self, seconds: u64) {
        self.wall.fetch_add(seconds as i64, Ordering::SeqCst);
        self.tick.fetch_add(seconds, Ordering::SeqCst);
    }
}
impl Clock for TestClock {
    fn wall(&self) -> i64 {
        self.wall.load(Ordering::SeqCst)
    }
    fn elapsed(&self) -> Duration {
        Duration::from_secs(self.tick.load(Ordering::SeqCst))
    }
}
struct Mock {
    clock: Arc<TestClock>,
    mode: AtomicU16,
    enabled: AtomicBool,
    records: Mutex<Vec<(String, String)>>,
    limit: AtomicU64,
    count: AtomicU64,
    heartbeat_count: AtomicU64,
    delay: AtomicBool,
    delay_create: AtomicBool,
    arrived: tokio::sync::Notify,
    release: tokio::sync::Notify,
}
impl Mock {
    fn grant(&self) -> Value {
        json!({"subject":{"id":"user-1","username":"demo","display_name":null,"tenant_id":"tenant-1","tenant_code":"demo","tenant_name":"测试企业"},"policy":{"product_code":"lingque","module_enabled":true,"concurrent_device_limit":self.limit.load(Ordering::SeqCst),"active_session_count":self.records.lock().unwrap().len(),"heartbeat_interval_seconds":600,"offline_grace_seconds":86400},"offline_until":chrono::DateTime::from_timestamp(self.clock.wall()+86400,0).unwrap().to_rfc3339()})
    }
    fn token(headers: &HeaderMap) -> &str {
        headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .unwrap_or("")
    }
}
async fn login(HttpState(state): HttpState<Arc<Mock>>, Json(body): Json<Value>) -> Response {
    if body["tenant_code"] != "demo"
        || body["username"] != "demo"
        || body["password"] != " test-only "
    {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"detail":"Invalid username or password"})),
        )
            .into_response();
    }
    if state.mode.load(Ordering::SeqCst) == 429 {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            [("retry-after", "120")],
            Json(json!({"detail":"Too many requests"})),
        )
            .into_response();
    }
    Json(json!({"access_token":"account-jwt-test-only","actor_type":"tenant_user","expires_in":3600})).into_response()
}
async fn create(
    HttpState(state): HttpState<Arc<Mock>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    if Mock::token(&headers) != "account-jwt-test-only" {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if !state.enabled.load(Ordering::SeqCst) {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({"detail":{"code":"product_module_not_enabled","message":"未开通"}})),
        )
            .into_response();
    }
    let count = state.count.fetch_add(1, Ordering::SeqCst);
    let token = format!("lingque-test-{count}");
    let replaced = {
        let mut records = state.records.lock().unwrap();
        let id = body["device_id"].as_str().unwrap().to_string();
        records.retain(|(device, _)| device != &id);
        let replaced = if records.len() >= state.limit.load(Ordering::SeqCst) as usize {
            Some(records.remove(0).0)
        } else {
            None
        };
        records.push((id, token.clone()));
        replaced
    };
    let mut response = state.grant();
    response["session_token"] = json!(token);
    response["replaced_session_id"] = json!(replaced);
    if state.mode.load(Ordering::SeqCst) == 409 {
        response["policy"]["product_code"] = json!("data-desensitization");
    }
    if state.delay_create.load(Ordering::SeqCst) {
        state.arrived.notify_one();
        state.release.notified().await;
    }
    Json(response).into_response()
}
async fn heartbeat(
    HttpState(state): HttpState<Arc<Mock>>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    assert!(body.is_empty(), "heartbeat must not have a JSON body");
    state.heartbeat_count.fetch_add(1, Ordering::SeqCst);
    let valid = state
        .records
        .lock()
        .unwrap()
        .iter()
        .any(|(_, token)| token == Mock::token(&headers));
    if !valid {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"detail":{"code":"product_session_invalid"}})),
        )
            .into_response();
    }
    let grant = state.grant();
    if state.delay.load(Ordering::SeqCst) {
        state.arrived.notify_one();
        state.release.notified().await;
    }
    let mode = state.mode.load(Ordering::SeqCst);
    if (201..=203).contains(&mode) {
        let mut invalid = grant;
        match mode {
            201 => invalid["policy"]["module_enabled"] = json!(false),
            202 => invalid["policy"]["product_code"] = json!("other-product"),
            _ => invalid["subject"]["tenant_id"] = json!("other-tenant"),
        }
        return Json(invalid).into_response();
    }
    if mode != 200 {
        return (
            StatusCode::from_u16(mode).unwrap(),
            Json(json!({"detail":"test error"})),
        )
            .into_response();
    }
    if !state.enabled.load(Ordering::SeqCst) {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({"detail":{"code":"product_module_not_enabled"}})),
        )
            .into_response();
    }
    Json(grant).into_response()
}
async fn logout(
    HttpState(state): HttpState<Arc<Mock>>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    assert!(body.is_empty());
    let token = Mock::token(&headers);
    if state.mode.load(Ordering::SeqCst) == 500 {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }
    state.records.lock().unwrap().retain(|(_, t)| t != token);
    StatusCode::NO_CONTENT.into_response()
}
struct Fixture {
    mock: Arc<Mock>,
    auth: Arc<Authority>,
    vault: Arc<MemoryVault>,
    server: tokio::task::JoinHandle<()>,
    base: String,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.server.abort();
    }
}
async fn fixture() -> Fixture {
    let clock = Arc::new(TestClock::new());
    let mock = Arc::new(Mock {
        clock: clock.clone(),
        mode: AtomicU16::new(200),
        enabled: AtomicBool::new(true),
        records: Mutex::new(Vec::new()),
        limit: AtomicU64::new(2),
        count: AtomicU64::new(0),
        heartbeat_count: AtomicU64::new(0),
        delay: AtomicBool::new(false),
        delay_create: AtomicBool::new(false),
        arrived: tokio::sync::Notify::new(),
        release: tokio::sync::Notify::new(),
    });
    let router = Router::new()
        .route("/auth/tenant-user/login", post(login))
        .route(PRODUCT, post(create))
        .route(&format!("{PRODUCT}/current/heartbeat"), post(heartbeat))
        .route(&format!("{PRODUCT}/current"), delete(logout))
        .with_state(mock.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let vault = Arc::new(MemoryVault::default());
    let auth = Arc::new(
        Authority::build(
            vault.clone(),
            clock,
            base.clone(),
            "device-1".into(),
            "Test desktop".into(),
            "test".into(),
        )
        .unwrap(),
    );
    Fixture {
        mock,
        auth,
        vault,
        server,
        base,
    }
}
async fn sign_in(f: &Fixture) {
    assert!(
        f.auth
            .login(" demo ", " demo ", " test-only ")
            .await
            .unwrap()
            .authorized
    );
}

#[tokio::test]
async fn two_stage_login_heartbeat_and_empty_logout() {
    let f = fixture().await;
    assert!(f.auth.require().is_err());
    sign_in(&f).await;
    let persisted = f.vault.load().unwrap().unwrap();
    assert!(!persisted.contains("account-jwt"));
    assert!(!persisted.contains("test-only"));
    let public = serde_json::to_string(&f.auth.status()).unwrap();
    assert!(!public.contains("lingque-test"));
    assert!(f.auth.heartbeat(true).await.unwrap().authorized);
    assert!(!f.auth.logout().await.unwrap().authorized);
    assert!(f.vault.load().unwrap().is_none());
    assert_eq!(f.auth.device_id, "device-1");
}
#[tokio::test]
async fn module_disabled_account_invalid_and_wrong_product_fail_closed() {
    let f = fixture().await;
    assert!(f.auth.login("wrong", "demo", " test-only ").await.is_err());
    f.mock.enabled.store(false, Ordering::SeqCst);
    assert!(f
        .auth
        .login("demo", "demo", " test-only ")
        .await
        .unwrap_err()
        .message
        .contains("未开通"));
    f.mock.enabled.store(true, Ordering::SeqCst);
    f.mock.mode.store(409, Ordering::SeqCst);
    assert!(f.auth.login("demo", "demo", " test-only ").await.is_err());
    assert!(f.auth.require().is_err());
}
#[tokio::test]
async fn restart_reuses_token_and_network_failure_never_extends_deadline() {
    let f = fixture().await;
    sign_in(&f).await;
    let deadline = f.auth.status().offline_until;
    let restarted = Authority::build(
        f.vault.clone(),
        f.mock.clock.clone(),
        f.base.clone(),
        "device-1".into(),
        "test".into(),
        "test".into(),
    )
    .unwrap();
    assert!(restarted.require().is_err());
    f.mock.mode.store(500, Ordering::SeqCst);
    assert_eq!(restarted.heartbeat(true).await.unwrap().state, "offline");
    f.mock.clock.advance(86000);
    assert!(restarted.heartbeat(true).await.unwrap().authorized);
    assert_eq!(restarted.status().offline_until, deadline);
    f.mock.clock.advance(401);
    assert!(restarted.require().is_err());
    assert!(restarted.heartbeat(true).await.is_err());
    f.mock.mode.store(200, Ordering::SeqCst);
    assert!(restarted.heartbeat(true).await.unwrap().authorized);
    assert_eq!(f.mock.count.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn denied_heartbeat_never_falls_back_to_offline() {
    for code in [401, 403] {
        let f = fixture().await;
        sign_in(&f).await;
        f.mock.mode.store(code, Ordering::SeqCst);
        assert!(f.auth.heartbeat(true).await.is_err());
        assert!(f.auth.require().is_err());
        assert!(f.vault.load().unwrap().is_none());
    }
}

#[tokio::test]
async fn invalid_success_policy_cannot_revive_offline_after_retry_or_restart() {
    for code in [201, 202, 203, 400] {
        let f = fixture().await;
        sign_in(&f).await;
        f.mock.mode.store(code, Ordering::SeqCst);
        assert!(f.auth.heartbeat(true).await.is_err());
        f.mock.mode.store(500, Ordering::SeqCst);
        for _ in 0..2 {
            assert!(f.auth.heartbeat(true).await.is_err());
            assert!(f.auth.require().is_err());
        }
        let restarted = Authority::build(
            f.vault.clone(),
            f.mock.clock.clone(),
            f.base.clone(),
            "device-1".into(),
            "test".into(),
            "test".into(),
        )
        .unwrap();
        assert!(restarted.heartbeat(true).await.is_err());
        f.mock.mode.store(200, Ordering::SeqCst);
        assert!(restarted.heartbeat(true).await.unwrap().authorized);
    }
}

#[tokio::test]
async fn storage_failure_latches_and_real_connection_failure_uses_only_original_deadline() {
    let f = fixture().await;
    sign_in(&f).await;
    let deadline = f.auth.status().offline_until;
    f.server.abort();
    while !f.server.is_finished() {
        tokio::task::yield_now().await;
    }
    // A fresh connection avoids an already accepted HTTP keep-alive connection.
    let auth = Authority::build(
        f.vault.clone(),
        f.mock.clock.clone(),
        f.base.clone(),
        "device-1".into(),
        "test".into(),
        "test".into(),
    )
    .unwrap();
    assert_eq!(auth.heartbeat(true).await.unwrap().state, "offline");
    assert_eq!(auth.status().offline_until, deadline);
    f.vault.broken.store(true, Ordering::SeqCst);
    f.mock.clock.advance(61);
    assert!(auth.require().is_err());
    for _ in 0..2 {
        assert!(auth.heartbeat(true).await.is_err());
    }
    assert!(f.vault.load().unwrap().is_none());
}
#[tokio::test]
async fn clock_rollback_requires_online_validation_even_after_restart() {
    let f = fixture().await;
    sign_in(&f).await;
    f.mock.clock.advance(600);
    let _ = f.auth.status();
    f.mock.clock.wall.fetch_sub(500, Ordering::SeqCst);
    assert_eq!(f.auth.status().state, "clock_invalid");
    f.mock.mode.store(500, Ordering::SeqCst);
    assert!(f.auth.heartbeat(true).await.is_err());
    // A failed retry must not erase the rollback latch.
    assert!(f.auth.heartbeat(true).await.is_err());
    let restarted = Authority::build(
        f.vault.clone(),
        f.mock.clock.clone(),
        f.base.clone(),
        "device-1".into(),
        "test".into(),
        "test".into(),
    )
    .unwrap();
    assert!(restarted.heartbeat(true).await.is_err());
    f.mock.mode.store(200, Ordering::SeqCst);
    assert!(f.auth.heartbeat(true).await.unwrap().authorized);
}
#[tokio::test]
async fn heartbeat_single_flight_and_late_reply_cannot_restore_logout() {
    let f = fixture().await;
    sign_in(&f).await;
    f.mock.delay.store(true, Ordering::SeqCst);
    let auth = f.auth.clone();
    let pending = tokio::spawn(async move { auth.heartbeat(true).await });
    f.mock.arrived.notified().await;
    f.auth.heartbeat(true).await.unwrap();
    assert_eq!(f.mock.heartbeat_count.load(Ordering::SeqCst), 1);
    f.auth.logout().await.unwrap();
    f.mock.release.notify_one();
    assert!(pending.await.unwrap().is_err());
    assert!(f.auth.require().is_err());
    assert!(f.vault.load().unwrap().is_none());
}

#[tokio::test]
async fn logout_during_login_never_restores_a_late_product_session() {
    let f = fixture().await;
    f.mock.delay_create.store(true, Ordering::SeqCst);
    let auth = f.auth.clone();
    let pending = tokio::spawn(async move { auth.login("demo", "demo", " test-only ").await });
    f.mock.arrived.notified().await;
    f.auth.logout().await.unwrap();
    f.mock.release.notify_one();
    assert!(pending.await.unwrap().is_err());
    assert!(f.auth.require().is_err());
    assert!(f.vault.load().unwrap().is_none());
}
#[tokio::test]
async fn same_device_rotation_and_capacity_replacement() {
    let f = fixture().await;
    sign_in(&f).await;
    let old = f
        .auth
        .state
        .lock()
        .unwrap()
        .cached
        .as_ref()
        .unwrap()
        .token
        .clone();
    let second = Authority::build(
        Arc::new(MemoryVault::default()),
        f.mock.clock.clone(),
        f.base.clone(),
        "device-1".into(),
        "test".into(),
        "test".into(),
    )
    .unwrap();
    second.login("demo", "demo", " test-only ").await.unwrap();
    assert_eq!(f.mock.records.lock().unwrap().len(), 1);
    assert!(!f
        .mock
        .records
        .lock()
        .unwrap()
        .iter()
        .any(|(_, t)| t == &old));
    assert!(f.auth.heartbeat(true).await.is_err());
    f.mock.limit.store(1, Ordering::SeqCst);
    let third = Authority::build(
        Arc::new(MemoryVault::default()),
        f.mock.clock.clone(),
        f.base.clone(),
        "device-2".into(),
        "test".into(),
        "test".into(),
    )
    .unwrap();
    assert!(third
        .login("demo", "demo", " test-only ")
        .await
        .unwrap()
        .message
        .contains("替换"));
    assert!(second.heartbeat(true).await.is_err());
}
#[tokio::test]
async fn quota_reduction_and_admin_revocation_require_login() {
    let f = fixture().await;
    sign_in(&f).await;
    f.mock.limit.store(1, Ordering::SeqCst);
    f.mock.records.lock().unwrap().clear();
    assert!(f.auth.heartbeat(true).await.is_err());
    sign_in(&f).await;
    f.mock.enabled.store(false, Ordering::SeqCst);
    assert!(f.auth.heartbeat(true).await.is_err());
    f.mock.enabled.store(true, Ordering::SeqCst);
    assert!(f.auth.require().is_err());
}
#[tokio::test]
async fn account_and_other_product_tokens_are_never_accepted_as_session() {
    for token in ["account-jwt-test-only", "private-product-token"] {
        let f = fixture().await;
        sign_in(&f).await;
        f.auth.state.lock().unwrap().cached.as_mut().unwrap().token = token.into();
        assert!(f.auth.heartbeat(true).await.is_err());
        assert!(f.auth.require().is_err());
    }
}
#[tokio::test]
async fn server_interval_and_rate_limit_are_respected() {
    let f = fixture().await;
    f.mock.mode.store(429, Ordering::SeqCst);
    assert!(f.auth.login("demo", "demo", " test-only ").await.is_err());
    assert_eq!(
        f.auth
            .login("demo", "demo", " test-only ")
            .await
            .unwrap_err()
            .code,
        "auth_rate_limited"
    );
    f.mock.clock.advance(120);
    f.mock.mode.store(200, Ordering::SeqCst);
    sign_in(&f).await;
    f.auth.heartbeat(false).await.unwrap();
    assert_eq!(f.mock.heartbeat_count.load(Ordering::SeqCst), 0);
    f.mock.clock.advance(600);
    f.auth.heartbeat(false).await.unwrap();
    assert_eq!(f.mock.heartbeat_count.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn vault_failure_and_failed_logout_do_not_grant_access() {
    let f = fixture().await;
    f.vault.broken.store(true, Ordering::SeqCst);
    assert!(f.auth.login("demo", "demo", " test-only ").await.is_err());
    assert!(f.auth.require().is_err());
    f.vault.broken.store(false, Ordering::SeqCst);
    sign_in(&f).await;
    f.mock.mode.store(500, Ordering::SeqCst);
    assert!(!f.auth.logout().await.unwrap().authorized);
    assert!(f.vault.load().unwrap().is_none());
}
#[test]
fn validation_errors_do_not_echo_password_inputs() {
    let msg = public_error(
        422,
        &json!({"detail":[{"loc":["body","password"],"msg":"bad secret","input":"sensitive-password"}]}),
    );
    assert!(msg.contains("密码"));
    assert!(!msg.contains("sensitive-password"));
}
