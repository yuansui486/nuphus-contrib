//! UA2 product authorization. Secrets never form part of a public status DTO.
use crate::{ApiError, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const BASE_URL: &str = "https://dongdongkc.shierkeji.com:6201/ua2/api/v1";
const PRODUCT: &str = "/auth/products/lingque/sessions";

pub trait Vault: Send + Sync {
    fn load(&self) -> Result<Option<String>>;
    fn save(&self, value: &str) -> Result<()>;
    fn clear(&self) -> Result<()>;
}
pub trait Clock: Send + Sync {
    fn wall(&self) -> i64;
    fn elapsed(&self) -> Duration;
}
pub struct SystemClock(Instant);
impl Default for SystemClock {
    fn default() -> Self {
        Self(Instant::now())
    }
}
impl Clock for SystemClock {
    fn wall(&self) -> i64 {
        chrono::Utc::now().timestamp()
    }
    fn elapsed(&self) -> Duration {
        self.0.elapsed()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Subject {
    pub id: String,
    pub username: String,
    pub display_name: Option<String>,
    pub tenant_id: String,
    pub tenant_code: String,
    pub tenant_name: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Policy {
    pub product_code: String,
    pub module_enabled: bool,
    pub concurrent_device_limit: u32,
    pub active_session_count: u32,
    pub heartbeat_interval_seconds: u64,
    pub offline_grace_seconds: u64,
}
#[derive(Clone, Serialize, Deserialize)]
struct Grant {
    subject: Subject,
    policy: Policy,
    offline_until: chrono::DateTime<chrono::Utc>,
}
#[derive(Deserialize)]
struct SessionResponse {
    session_token: String,
    #[serde(flatten)]
    grant: Grant,
    replaced_session_id: Option<String>,
}
#[derive(Deserialize)]
struct LoginResponse {
    access_token: String,
    actor_type: String,
}
#[derive(Clone, Serialize, Deserialize)]
struct Cached {
    token: String,
    epoch: String,
    grant: Grant,
    high_water: i64,
    #[serde(default)]
    clock_invalid: bool,
    #[serde(default)]
    offline_blocked: bool,
}
struct State {
    cached: Option<Cached>,
    generation: u64,
    mode: &'static str,
    message: String,
    verified: bool,
    anchor_wall: i64,
    anchor_tick: Duration,
    next_attempt: Duration,
    failures: u32,
    last_persist: Duration,
}
#[derive(Debug, Clone, Serialize)]
pub struct Status {
    pub state: String,
    pub authorized: bool,
    pub message: String,
    pub subject: Option<Subject>,
    pub policy: Option<Policy>,
    pub offline_until: Option<String>,
    pub epoch: Option<String>,
}
#[derive(Clone)]
pub struct Identity {
    pub subject: Subject,
    pub epoch: String,
}
pub struct Authority {
    state: Mutex<State>,
    network: tokio::sync::Mutex<()>,
    vault: Arc<dyn Vault>,
    clock: Arc<dyn Clock>,
    http: reqwest::Client,
    base: String,
    pub device_id: String,
    pub device_name: String,
    pub app_version: String,
}

fn error(code: &str, message: &str) -> ApiError {
    ApiError::new(code, message)
}
fn protocol_error() -> ApiError {
    error(
        "auth_protocol_error",
        "鉴权服务响应不符合灵雀协议，请联网重试或联系管理员。",
    )
}
impl Authority {
    pub fn new(
        vault: Arc<dyn Vault>,
        device_id: String,
        device_name: String,
        version: String,
    ) -> Result<Self> {
        Self::build(
            vault,
            Arc::new(SystemClock::default()),
            BASE_URL.into(),
            device_id,
            device_name,
            version,
        )
    }
    fn build(
        vault: Arc<dyn Vault>,
        clock: Arc<dyn Clock>,
        base: String,
        device_id: String,
        device_name: String,
        version: String,
    ) -> Result<Self> {
        let loaded = vault.load();
        let (cached, message) = match loaded {
            Ok(Some(raw)) => match serde_json::from_str::<Cached>(&raw) {
                Ok(value) => (Some(value), "正在验证已有登录…".into()),
                Err(_) => (None, "授权缓存无法读取，请重新登录。".into()),
            },
            Ok(None) => (None, "请登录灵雀。".into()),
            Err(_) => (None, "安全凭据存储不可用，请解锁系统凭据库后重试。".into()),
        };
        Ok(Self {
            state: Mutex::new(State {
                mode: if cached.is_some() {
                    "restoring"
                } else {
                    "signed_out"
                },
                cached,
                generation: 0,
                message,
                verified: false,
                anchor_wall: clock.wall(),
                anchor_tick: clock.elapsed(),
                next_attempt: Duration::ZERO,
                failures: 0,
                last_persist: Duration::ZERO,
            }),
            network: tokio::sync::Mutex::new(()),
            vault,
            clock,
            http: reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(10))
                .timeout(Duration::from_secs(20))
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .map_err(|_| protocol_error())?,
            base,
            device_id,
            device_name,
            app_version: version,
        })
    }
    fn valid(grant: &Grant) -> bool {
        grant.policy.product_code == "lingque"
            && grant.policy.module_enabled
            && !grant.subject.id.is_empty()
            && !grant.subject.tenant_id.is_empty()
            && grant.policy.heartbeat_interval_seconds > 0
            && grant.policy.offline_grace_seconds <= 86400
    }
    fn inspect(&self, state: &mut State) {
        let now = self.clock.wall();
        let tick = self.clock.elapsed();
        let expected = state.anchor_wall + tick.saturating_sub(state.anchor_tick).as_secs() as i64;
        if let Some(cache) = &mut state.cached {
            let was_clock_invalid = cache.clock_invalid;
            if now + 120 < cache.high_water.max(expected) {
                cache.clock_invalid = true;
            }
            if cache.clock_invalid {
                state.verified = false;
                state.mode = "clock_invalid";
                state.message = "检测到系统时间回拨，请联网验证。".into();
            }
            cache.high_water = cache.high_water.max(now);
            if now >= cache.grant.offline_until.timestamp() && state.verified {
                state.verified = false;
                state.mode = "expired";
                state.message = "离线授权已到期，请联网验证。".into();
            }
            if cache.clock_invalid != was_clock_invalid
                || tick.saturating_sub(state.last_persist) >= Duration::from_secs(60)
            {
                if self
                    .vault
                    .save(&serde_json::to_string(cache).unwrap_or_default())
                    .is_err()
                {
                    cache.offline_blocked = true;
                    let _ = self.vault.clear();
                    state.verified = false;
                    state.mode = "storage_error";
                    state.message = "无法安全保存授权，请检查凭据存储。".into();
                }
                state.last_persist = tick;
            }
        }
    }
    pub fn status(&self) -> Status {
        let mut state = self.state.lock().expect("auth state");
        self.inspect(&mut state);
        Status {
            state: state.mode.into(),
            authorized: state.verified && state.cached.is_some(),
            message: state.message.clone(),
            subject: state.cached.as_ref().map(|c| c.grant.subject.clone()),
            policy: state.cached.as_ref().map(|c| c.grant.policy.clone()),
            offline_until: state
                .cached
                .as_ref()
                .map(|c| c.grant.offline_until.to_rfc3339()),
            epoch: state.cached.as_ref().map(|c| c.epoch.clone()),
        }
    }
    pub fn require(&self) -> Result<Identity> {
        let status = self.status();
        if !status.authorized {
            return Err(error("product_auth_required", &status.message));
        }
        Ok(Identity {
            subject: status.subject.ok_or_else(protocol_error)?,
            epoch: status.epoch.ok_or_else(protocol_error)?,
        })
    }
    pub fn require_epoch(&self, epoch: &str) -> Result<Identity> {
        let identity = self.require()?;
        if identity.epoch != epoch {
            return Err(error(
                "product_session_changed",
                "登录身份已变化，请重新连接灵雀。",
            ));
        }
        Ok(identity)
    }
    async fn response(
        &self,
        request: reqwest::RequestBuilder,
    ) -> std::result::Result<Value, Failure> {
        let response = request.send().await.map_err(|_| Failure::Network)?;
        let status = response.status();
        let retry = response
            .headers()
            .get("retry-after")
            .and_then(|h| h.to_str().ok())
            .map(str::to_owned);
        if status.is_server_error() {
            return Err(Failure::Network);
        }
        if status == reqwest::StatusCode::NO_CONTENT {
            return Ok(Value::Null);
        }
        let body: Value = match response.bytes().await {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or(Value::Null),
            // A broken body stream is a network error, but a received denial
            // status must remain a denial even if its optional detail is lost.
            Err(_) if status.is_success() => return Err(Failure::Network),
            Err(_) => Value::Null,
        };
        if !status.is_success() {
            return Err(Failure::Http(
                status.as_u16(),
                public_error(status.as_u16(), &body),
                retry,
            ));
        }
        if body.is_null() {
            return Err(Failure::Protocol);
        }
        Ok(body)
    }
    pub async fn login(&self, tenant: &str, username: &str, password: &str) -> Result<Status> {
        let _network = self
            .network
            .try_lock()
            .map_err(|_| error("auth_busy", "登录或验证正在进行，请稍候。"))?;
        let tenant = tenant.trim();
        let username = username.trim();
        if tenant.is_empty()
            || username.is_empty()
            || password.is_empty()
            || tenant.chars().count() > 64
            || username.chars().count() > 64
            || password.chars().count() > 128
        {
            return Err(error(
                "auth_invalid_input",
                "请填写租户编码、用户名及密码，并检查长度。",
            ));
        }
        let generation = {
            let mut state = self.state.lock().expect("auth state");
            if self.clock.elapsed() < state.next_attempt && state.mode == "rate_limited" {
                return Err(error("auth_rate_limited", "请求过于频繁，请稍后重试。"));
            }
            if state.cached.is_some() {
                return Err(error("auth_already_signed_in", "请先退出当前账号再登录。"));
            }
            state.generation += 1;
            state.mode = "signing_in";
            state.message = "正在登录…".into();
            state.verified = false;
            state.generation
        };
        let result = async {
            let account: LoginResponse = serde_json::from_value(self.response(self.http.post(format!("{}/auth/tenant-user/login", self.base)).json(&json!({"tenant_code":tenant,"username":username,"password":password}))).await?).map_err(|_| Failure::Protocol)?;
            if account.actor_type != "tenant_user" || account.access_token.is_empty() { return Err(Failure::Protocol); }
            let session: SessionResponse = serde_json::from_value(self.response(self.http.post(format!("{}{PRODUCT}", self.base)).bearer_auth(&account.access_token).json(&json!({"device_id":self.device_id,"device_name":self.device_name,"app_version":self.app_version}))).await?).map_err(|_| Failure::Protocol)?;
            if session.session_token.is_empty() || !Self::valid(&session.grant) || session.grant.offline_until.timestamp() <= self.clock.wall() { return Err(Failure::Protocol); }
            Ok(session)
        }.await;
        match result {
            Ok(session) => {
                let mut state = self.state.lock().expect("auth state");
                if state.generation != generation {
                    return Err(error("auth_cancelled", "登录已取消。"));
                }
                let replaced = session.replaced_session_id.is_some();
                let cache = Cached {
                    token: session.session_token,
                    epoch: uuid::Uuid::new_v4().to_string(),
                    grant: session.grant,
                    high_water: self.clock.wall(),
                    clock_invalid: false,
                    offline_blocked: false,
                };
                if let Err(e) = self.vault.save(&serde_json::to_string(&cache)?) {
                    state.mode = "storage_error";
                    state.message = "无法安全保存授权，请重试。".into();
                    return Err(e);
                }
                self.accept(&mut state, cache);
                if replaced {
                    state.message = "已登录，较早的设备登录已被替换。".into();
                }
            }
            Err(failure) => {
                self.reject(generation, failure, false)?;
            }
        }
        Ok(self.status())
    }
    fn accept(&self, state: &mut State, cache: Cached) {
        state.next_attempt = self.clock.elapsed()
            + Duration::from_secs(cache.grant.policy.heartbeat_interval_seconds);
        state.cached = Some(cache);
        state.mode = "online";
        state.message = "已授权".into();
        state.verified = true;
        state.failures = 0;
        state.anchor_wall = self.clock.wall();
        state.anchor_tick = self.clock.elapsed();
        state.last_persist = self.clock.elapsed();
    }
    fn reject(&self, generation: u64, failure: Failure, heartbeat: bool) -> Result<()> {
        let mut state = self.state.lock().expect("auth state");
        if state.generation != generation {
            return Err(error("auth_cancelled", "旧登录请求已取消。"));
        }
        self.inspect(&mut state);
        // Only a successful online validation can remove a non-network denial.
        // Persist this latch so retries/restarts cannot revive an older offline grant.
        if !matches!(failure, Failure::Network) {
            if let Some(cache) = &mut state.cached {
                cache.offline_blocked = true;
                if self.vault.save(&serde_json::to_string(cache)?).is_err() {
                    let _ = self.vault.clear();
                }
            }
        }
        state.failures = state.failures.saturating_add(1);
        state.next_attempt = self.clock.elapsed()
            + Duration::from_secs((30u64.saturating_mul(1 << state.failures.min(5))).min(600));
        match failure {
            Failure::Network => {
                let allowed = heartbeat
                    && !matches!(state.mode, "clock_invalid" | "storage_error")
                    && state.cached.as_ref().is_some_and(|c| {
                        Self::valid(&c.grant)
                            && !c.offline_blocked
                            && !c.clock_invalid
                            && self.clock.wall() < c.grant.offline_until.timestamp()
                    });
                state.verified = allowed;
                state.mode = if allowed { "offline" } else { "unavailable" };
                state.message = if allowed {
                    "鉴权连接暂时不可用，正在使用原离线授权。"
                } else {
                    "无法连接鉴权服务，请检查网络后重试。"
                }
                .into();
            }
            Failure::Http(code, message, retry) => {
                state.verified = false;
                state.message = message;
                if code == 401 || code == 403 {
                    state.cached = None;
                    state.generation += 1;
                    state.mode = "signed_out";
                    self.vault.clear()?;
                } else if code == 429 {
                    state.mode = "rate_limited";
                    let seconds = retry
                        .and_then(|v| {
                            v.parse::<u64>().ok().or_else(|| {
                                chrono::DateTime::parse_from_rfc2822(&v)
                                    .ok()
                                    .map(|t| (t.timestamp() - self.clock.wall()).max(1) as u64)
                            })
                        })
                        .unwrap_or(60);
                    state.next_attempt =
                        self.clock.elapsed() + Duration::from_secs(seconds.min(86400));
                } else {
                    state.mode = "rejected";
                }
            }
            Failure::Protocol => {
                state.verified = false;
                state.mode = "rejected";
                state.message = protocol_error().message;
            }
        }
        if state.verified {
            Ok(())
        } else {
            Err(error("product_auth_required", &state.message))
        }
    }
    pub async fn heartbeat(&self, force: bool) -> Result<Status> {
        let Ok(_network) = self.network.try_lock() else {
            return Ok(self.status());
        };
        let (cached, generation) = {
            let mut state = self.state.lock().expect("auth state");
            self.inspect(&mut state);
            let Some(cache) = &state.cached else {
                drop(state);
                return Ok(self.status());
            };
            if !force && state.mode == "rejected" {
                drop(state);
                return Ok(self.status());
            }
            if (!force || state.mode == "rate_limited") && self.clock.elapsed() < state.next_attempt
            {
                drop(state);
                return Ok(self.status());
            }
            (cache.clone(), state.generation)
        };
        let result = self
            .response(
                self.http
                    .post(format!("{}{PRODUCT}/current/heartbeat", self.base))
                    .bearer_auth(&cached.token),
            )
            .await;
        match result {
            Ok(value) => {
                let grant = serde_json::from_value::<Grant>(value).map_err(|_| protocol_error());
                let grant = match grant {
                    Ok(g)
                        if Self::valid(&g)
                            && g.subject.id == cached.grant.subject.id
                            && g.subject.tenant_id == cached.grant.subject.tenant_id
                            && g.offline_until.timestamp() > self.clock.wall() =>
                    {
                        g
                    }
                    _ => {
                        self.reject(generation, Failure::Protocol, true)?;
                        return Err(protocol_error());
                    }
                };
                let mut state = self.state.lock().expect("auth state");
                if state.generation != generation {
                    return Err(error("auth_cancelled", "旧心跳已取消。"));
                }
                let cache = Cached {
                    grant,
                    high_water: self.clock.wall(),
                    clock_invalid: false,
                    offline_blocked: false,
                    ..cached
                };
                if let Err(e) = self.vault.save(&serde_json::to_string(&cache)?) {
                    if let Some(previous) = &mut state.cached {
                        previous.offline_blocked = true;
                    }
                    let _ = self.vault.clear();
                    state.verified = false;
                    state.mode = "storage_error";
                    state.message = "无法安全保存授权，请重试。".into();
                    return Err(e);
                }
                self.accept(&mut state, cache);
            }
            Err(failure) => self.reject(generation, failure, true)?,
        }
        Ok(self.status())
    }
    pub async fn logout(&self) -> Result<Status> {
        let (token, cleared) = {
            let mut state = self.state.lock().expect("auth state");
            state.generation += 1;
            state.verified = false;
            state.mode = "signed_out";
            state.message = "已退出登录。".into();
            let token = state.cached.take().map(|c| c.token);
            let cleared = self.vault.clear();
            (token, cleared)
        };
        if let Some(token) = token {
            let result = self
                .response(
                    self.http
                        .delete(format!("{}{PRODUCT}/current", self.base))
                        .bearer_auth(token),
                )
                .await;
            if result.is_err() {
                let mut state = self.state.lock().expect("auth state");
                if state.cached.is_none() {
                    state.message =
                        "本机已退出；服务端退出未确认，原设备额度可能保留至离线期限。".into();
                }
            }
        }
        // Local storage failure must not skip remote revocation.
        cleared?;
        Ok(self.status())
    }
}
enum Failure {
    Network,
    Http(u16, String, Option<String>),
    Protocol,
}
fn public_error(status: u16, body: &Value) -> String {
    let detail = &body["detail"];
    let code = detail["code"].as_str().unwrap_or("");
    // Never serialize validation arrays: their `input` fields can contain passwords.
    if status != 422 {
        if let Some(message) = detail["message"].as_str().or_else(|| detail.as_str()) {
            let message = message.trim();
            let translated = match message {
                "Invalid username or password" => {
                    Some("账号或密码错误，请检查租户编码及账号状态。")
                }
                "Too many requests" => Some("请求过于频繁，请稍后重试。"),
                _ => None,
            };
            if let Some(translated) = translated {
                return translated.into();
            }
            // Preserve human-readable Chinese service guidance, never raw JSON or tokens.
            if message
                .chars()
                .any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c))
                && message.chars().count() <= 256
                && !message.chars().any(char::is_control)
                && !message
                    .split_whitespace()
                    .any(|word| word.len() > 80 && word.is_ascii())
            {
                return message.into();
            }
        }
    }
    match code {
        "product_module_not_enabled" => "该租户尚未开通灵雀，请联系管理员。".into(),
        "product_session_revoked" => "设备登录已失效，请重新登录。".into(),
        "product_session_invalid" | "product_session_missing" => {
            "灵雀产品令牌无效，请重新登录。".into()
        }
        _ => match status {
            401 => "账号、密码或租户状态无效，或设备登录已失效，请检查后重新登录。".into(),
            403 => "当前账号或租户无权使用灵雀，请联系管理员。".into(),
            429 => "登录尝试过多，请稍后重试。".into(),
            422 => {
                let fields = detail
                    .as_array()
                    .map(|items| {
                        items
                            .iter()
                            .filter_map(|item| item["loc"].as_array()?.last()?.as_str())
                            .filter_map(|field| match field {
                                "tenant_code" => Some("租户编码"),
                                "username" => Some("用户名"),
                                "password" => Some("密码"),
                                "device_id" => Some("设备标识"),
                                "device_name" => Some("设备名称"),
                                "app_version" => Some("应用版本"),
                                _ => None,
                            })
                            .collect::<Vec<_>>()
                            .join("、")
                    })
                    .unwrap_or_default();
                format!(
                    "请求字段不符合要求：{}，请检查后重试。",
                    if fields.is_empty() {
                        "登录参数"
                    } else {
                        &fields
                    }
                )
            }
            _ => format!("鉴权请求未成功（HTTP {status}），请重试或联系管理员。"),
        },
    }
}

#[cfg(test)]
mod tests;
