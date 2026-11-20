//! Segway Navimow robotic mowers, through Navimow's cloud: what each mower
//! does and its battery, read every `poll_secs`; start, stop, pause, resume
//! and back to its dock. The smart-home API of Navimow's official Home
//! Assistant integration (`segwaynavimow/NavimowHA`), signed in once from the
//! dashboard ([`auth`]); its live MQTT push is not used yet (the polling is
//! what that integration falls back to).
mod auth;
mod encode;
mod model;

use std::time::Duration;

use anyhow::Context as _;
use auth::{API_HOST, Failure, Session};
use http::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE, HOST, USER_AGENT};
use model::{Answer, Mower, Order, State, Status};
use moli_core::{Access, Device, DeviceId, Kind, PointSpec, Semantic, Unit, Value};
use moli_runtime::{BoxFuture, Driver, DriverCtx};
use serde::Deserialize;
use serde_json::Value as Json;

/// The tokens, one JSON secret ([`Session`]).
const SESSION: &str = "session";
const ACCOUNT: &str = "account";
const STATUSES: [&str; 4] = ["connecting", "sign_in", "connected", "error"];
const LIMIT: Duration = Duration::from_secs(20);
const MAX_BODY: usize = 256 * 1024;
const RETRY_MIN: Duration = Duration::from_secs(30);
const RETRY_MAX: Duration = Duration::from_secs(900);
/// After an order, the state is read again this much later.
const AFTER_ORDER: Duration = Duration::from_secs(4);

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// How often the mowers are read (seconds).
    #[serde(default = "default_poll")]
    pub poll_secs: u64,
}

fn default_poll() -> u64 {
    30
}

#[derive(Debug)]
pub struct Navimow {
    config: Config,
}

impl Navimow {
    pub fn new(config: Config) -> anyhow::Result<Self> {
        anyhow::ensure!(
            (15..=3600).contains(&config.poll_secs),
            moli_i18n::tr!("pilotes.navimow.config_invalide")
        );
        Ok(Self { config })
    }
}

impl Driver for Navimow {
    fn kind(&self) -> &'static str {
        "navimow"
    }
    fn run<'a>(&'a self, ctx: &'a mut DriverCtx) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(run(&self.config, ctx))
    }
}

fn point(
    key: &str,
    label: String,
    kind: Kind,
    write: bool,
    unit: Option<Unit>,
    semantic: Semantic,
) -> PointSpec {
    PointSpec {
        key: key.into(),
        label: label.into(),
        kind,
        access: Access { read: true, write },
        unit,
        semantic,
    }
}

fn set_text(ctx: &DriverCtx, id: &DeviceId, key: &str, text: &str) {
    ctx.set_state(id, key, Value::Text(text.into()));
}

/// The account: its state, and where the dashboard signs it in.
fn account_device(ctx: &DriverCtx, id: &DeviceId) -> Device {
    let mut sign_in = point(
        "sign_in",
        moli_i18n::tr!("pilotes.navimow.adresse_collee"),
        Kind::Text,
        true,
        None,
        Semantic::Config,
    );
    sign_in.access.read = true;
    Device {
        id: id.clone(),
        instance: ctx.instance().clone(),
        native_name: moli_i18n::tr!("pilotes.navimow.compte").into(),
        manufacturer: Some("Navimow".into()),
        model: Some(moli_i18n::tr!("pilotes.navimow.modele_compte").into()),
        description: None,
        native_room: None,
        members: Vec::new(),
        points: vec![
            point(
                "status",
                moli_i18n::tr!("pilotes.navimow.etat_compte"),
                Kind::Enum {
                    values: STATUSES.iter().map(|s| (*s).into()).collect(),
                },
                false,
                None,
                Semantic::Other,
            ),
            point(
                "sign_in_url",
                moli_i18n::tr!("pilotes.navimow.page_navimow"),
                Kind::Text,
                false,
                None,
                Semantic::Other,
            ),
            sign_in,
        ],
    }
}

/// A mower: what it does, its battery, and its controls.
fn mower_device(ctx: &DriverCtx, m: &Mower) -> Device {
    let flag = |key: &str, label: String, write: bool| {
        point(
            key,
            label,
            Kind::Binary,
            write,
            None,
            if write {
                Semantic::Control
            } else {
                Semantic::Other
            },
        )
    };
    Device {
        id: ctx.device_id(&m.id),
        instance: ctx.instance().clone(),
        native_name: m.name.clone().into(),
        manufacturer: Some("Segway Navimow".into()),
        model: m.model.clone().map(Into::into),
        description: None,
        native_room: None,
        members: Vec::new(),
        points: vec![
            point(
                "state",
                moli_i18n::tr!("pilotes.navimow.etat"),
                Kind::Enum {
                    values: State::ALL.iter().map(|s| s.key().into()).collect(),
                },
                false,
                None,
                Semantic::Other,
            ),
            point(
                "battery",
                moli_i18n::tr!("pilotes.navimow.batterie"),
                Kind::Numeric {
                    min: Some(0.0),
                    max: Some(100.0),
                    step: None,
                },
                false,
                Some(Unit::Percent),
                Semantic::Battery,
            ),
            flag("online", moli_i18n::tr!("pilotes.navimow.en_ligne"), false),
            flag("fault", moli_i18n::tr!("pilotes.navimow.probleme"), false),
            point(
                "detail",
                moli_i18n::tr!("pilotes.navimow.detail"),
                Kind::Text,
                false,
                None,
                Semantic::Other,
            ),
            flag("mow", moli_i18n::tr!("pilotes.navimow.tondre"), true),
            flag("pause", moli_i18n::tr!("pilotes.navimow.pause"), true),
            flag("dock", moli_i18n::tr!("pilotes.navimow.retour_base"), true),
        ],
    }
}

fn publish(ctx: &DriverCtx, mower: &DeviceId, s: &Status) {
    ctx.set_state(mower, "state", Value::Text(s.state.key().into()));
    ctx.set_state(
        mower,
        "battery",
        s.battery.map_or(Value::Null, Value::Float),
    );
    ctx.set_state(mower, "online", Value::Bool(s.state != State::Offline));
    ctx.set_state(mower, "fault", Value::Bool(s.state == State::Error));
    set_text(ctx, mower, "detail", s.raw.as_deref().unwrap_or(""));
    ctx.set_state(
        mower,
        "mow",
        Value::Bool(matches!(s.state, State::Mowing | State::Paused)),
    );
    ctx.set_state(mower, "pause", Value::Bool(s.state == State::Paused));
    ctx.set_state(mower, "dock", Value::Bool(s.state == State::Returning));
    ctx.set_availability(mower, s.state != State::Offline);
}

async fn run(config: &Config, ctx: &mut DriverCtx) -> anyhow::Result<()> {
    let account = ctx.device_id(ACCOUNT);
    ctx.upsert_device(account_device(ctx, &account));
    ctx.set_availability(&account, true);
    let mut refused = false;
    let mut retry = RETRY_MIN;
    loop {
        let stored = ctx
            .secret(SESSION)
            .filter(|_| !refused)
            .and_then(|s| serde_json::from_str::<Session>(&s).ok());
        let session = match stored {
            Some(s) => s,
            None => match sign_in(ctx, &account).await? {
                Some(s) => s,
                None => return Ok(()),
            },
        };
        refused = false;
        set_text(ctx, &account, "status", "connecting");
        ctx.ready();
        let started = tokio::time::Instant::now();
        let mut api = Api { session };
        let outcome = connected(config, ctx, &account, &mut api).await;
        if started.elapsed() >= RETRY_MAX {
            retry = RETRY_MIN;
        }
        match outcome {
            Ok(()) => return Ok(()),
            Err(Failure::Refused) => {
                tracing::warn!(instance = %ctx.instance(), "navimow session refused: sign in again");
                refused = true;
            }
            Err(Failure::Other(e)) => {
                tracing::warn!(instance = %ctx.instance(), error = %format!("{e:#}"), "navimow unreachable");
                set_text(ctx, &account, "status", "error");
                if pause(ctx, retry).await {
                    return Ok(());
                }
                retry = (retry * 2).min(RETRY_MAX);
            }
        }
    }
}

/// Waits `d`, or until Moli stops (true).
async fn pause(ctx: &DriverCtx, d: Duration) -> bool {
    tokio::select! {
        () = tokio::time::sleep(d) => false,
        () = ctx.cancelled() => true,
    }
}

async fn file(ctx: &DriverCtx, session: &Session) {
    let Ok(text) = serde_json::to_string(session) else {
        return;
    };
    if let Err(e) = ctx.store_secret(SESSION, &text).await {
        tracing::warn!(instance = %ctx.instance(), error = %e, "navimow: the session could not be filed: a sign-in will be needed after a restart");
    }
}

/// Waits for the person to sign in from the dashboard; `None` when Moli stops.
async fn sign_in(ctx: &mut DriverCtx, account: &DeviceId) -> anyhow::Result<Option<Session>> {
    let mut pending = auth::SignIn::new()?;
    set_text(ctx, account, "sign_in_url", &pending.url);
    set_text(ctx, account, "status", "sign_in");
    ctx.wait_for(moli_i18n::tr!("pilotes.navimow.connecte_toi"));
    loop {
        let Some(command) = ctx.next_command().await else {
            return Ok(None);
        };
        if command.device.id != *account || &*command.key != "sign_in" {
            command.reply(Err(moli_i18n::tr!("pilotes.navimow.pas_connecte")));
            continue;
        }
        let Value::Text(pasted) = &command.value else {
            command.reply(Err(moli_i18n::tr!("pilotes.navimow.adresse_fausse")));
            continue;
        };
        let code = match pending.code(pasted) {
            Ok(code) => code,
            Err(why) => {
                command.reply(Err(match why {
                    auth::Pasted::NotTheEnd => moli_i18n::tr!("pilotes.navimow.adresse_fausse"),
                    auth::Pasted::Stale => moli_i18n::tr!("pilotes.navimow.adresse_ancienne"),
                    auth::Pasted::NoCode => moli_i18n::tr!("pilotes.navimow.adresse_sans_code"),
                }));
                continue;
            }
        };
        match pending.session(&code).await {
            Ok(session) => {
                let Ok(text) = serde_json::to_string(&session) else {
                    continue;
                };
                if let Err(e) = ctx.store_secret(SESSION, &text).await {
                    command.reply(Err(moli_i18n::tr!("pilotes.navimow.coffre", why = e)));
                    continue;
                }
                set_text(ctx, account, "sign_in_url", "");
                command.reply(Ok(()));
                tracing::info!(instance = %ctx.instance(), renewable = session.refresh.is_some(), "navimow account connected");
                return Ok(Some(session));
            }
            Err(e) => {
                // A code serves once: a new page for the next try.
                pending = auth::SignIn::new()?;
                set_text(ctx, account, "sign_in_url", &pending.url);
                command.reply(Err(moli_i18n::tr!(
                    "pilotes.navimow.connexion_refusee",
                    why = e
                )));
            }
        }
    }
}

/// The API, with a session renewed before it expires.
struct Api {
    session: Session,
}

impl Api {
    async fn fresh(&mut self, ctx: &DriverCtx) -> Result<(), Failure> {
        if self.session.stale() {
            self.session = self.session.renewed().await?;
            file(ctx, &self.session).await;
        }
        Ok(())
    }

    async fn call(
        &mut self,
        ctx: &DriverCtx,
        method: http::Method,
        path: &str,
        body: Option<&Json>,
    ) -> Result<Json, Failure> {
        self.fresh(ctx).await?;
        match self.once(method.clone(), path, body).await {
            // Refused before its time: renewed once, then given up.
            Err(Failure::Refused) if self.session.refresh.is_some() => {
                self.session = self.session.renewed().await?;
                file(ctx, &self.session).await;
                self.once(method, path, body).await
            }
            other => other,
        }
    }

    async fn once(
        &self,
        method: http::Method,
        path: &str,
        body: Option<&Json>,
    ) -> Result<Json, Failure> {
        let mut id = [0u8; 16];
        ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut id)
            .map_err(|_| anyhow::anyhow!("no randomness"))?;
        let builder = http::Request::builder()
            .method(method)
            .uri(path)
            .header(HOST, API_HOST)
            .header(USER_AGENT, auth::AGENT)
            .header(ACCEPT, "application/json")
            .header(AUTHORIZATION, format!("Bearer {}", self.session.access))
            .header("requestId", encode::hex(&id));
        let request = match body {
            Some(b) => builder
                .header(CONTENT_TYPE, "application/json")
                .body(http_body_util::Full::new(moli_net::Body::from(
                    b.to_string(),
                )))
                .context("request")?,
            None => moli_net::empty(builder).context("request")?,
        };
        // Underlying errors are dropped: no token in a log.
        let (status, answer) = moli_net::web(API_HOST, 443, true, request, LIMIT, MAX_BODY)
            .await
            .map_err(|_| anyhow::anyhow!("{API_HOST} unreachable"))?;
        model::payload(status.as_u16(), &answer).map_err(|e| match e {
            Answer::Refused => Failure::Refused,
            Answer::Other(why) => Failure::Other(anyhow::anyhow!("Navimow: {why}")),
        })
    }
}

/// Signed in: the mowers, read every `poll_secs`, and their orders.
async fn connected(
    config: &Config,
    ctx: &mut DriverCtx,
    account: &DeviceId,
    api: &mut Api,
) -> Result<(), Failure> {
    let list = api
        .call(ctx, http::Method::GET, "/openapi/smarthome/authList", None)
        .await?;
    let mowers = model::mowers(&list);
    for m in &mowers {
        ctx.upsert_device(mower_device(ctx, m));
    }
    set_text(ctx, account, "status", "connected");
    ctx.clear_warning();
    tracing::info!(instance = %ctx.instance(), mowers = mowers.len(), "navimow connected");
    let ids: Vec<String> = mowers.iter().map(|m| m.id.clone()).collect();
    let mut poll = tokio::time::interval(Duration::from_secs(config.poll_secs));
    poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            command = ctx.next_command() => {
                let Some(command) = command else { return Ok(()) };
                let Some(m) = mowers.iter().find(|m| ctx.device_id(&m.id) == command.device.id) else {
                    command.reply(Err(moli_i18n::tr!("pilotes.navimow.deja_connecte")));
                    continue;
                };
                let on = matches!(command.value, Value::Bool(true));
                let order = match (&*command.key, on) {
                    ("mow", true) => Order::Start,
                    ("mow", false) => Order::Stop,
                    ("pause", true) => Order::Pause,
                    ("pause", false) => Order::Resume,
                    ("dock", true) => Order::Dock,
                    _ => {
                        command.reply(Err(moli_i18n::tr!("pilotes.navimow.ordre_inconnu")));
                        continue;
                    }
                };
                match api.call(ctx, http::Method::POST, "/openapi/smarthome/sendCommands", Some(&model::order_body(&m.id, order))).await {
                    Ok(answer) => match model::order_result(&answer) {
                        Ok(()) => command.reply(Ok(())),
                        Err(code) => command.reply(Err(moli_i18n::tr!("pilotes.navimow.ordre_refuse", code = code))),
                    },
                    Err(Failure::Refused) => {
                        command.reply(Err(moli_i18n::tr!("pilotes.navimow.pas_connecte")));
                        return Err(Failure::Refused);
                    }
                    Err(e) => command.reply(Err(e.to_string())),
                }
                if pause(ctx, AFTER_ORDER).await {
                    return Ok(());
                }
                read(ctx, api, &ids).await?;
            }
            _ = poll.tick() => read(ctx, api, &ids).await?,
        }
    }
}

async fn read(ctx: &DriverCtx, api: &mut Api, ids: &[String]) -> Result<(), Failure> {
    if ids.is_empty() {
        return Ok(());
    }
    let body = serde_json::json!({"devices": ids.iter().map(|id| serde_json::json!({"id": id})).collect::<Vec<_>>()});
    let answer = api
        .call(
            ctx,
            http::Method::POST,
            "/openapi/smarthome/getVehicleStatus",
            Some(&body),
        )
        .await?;
    for (id, status) in model::statuses(&answer) {
        if ids.contains(&id) {
            publish(ctx, &ctx.device_id(&id), &status);
        }
    }
    Ok(())
}
