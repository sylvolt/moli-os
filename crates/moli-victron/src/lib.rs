//! Victron Energy GX devices (Cerbo GX, Venus OS) through the MQTT broker
//! they run on the local network (« MQTT on LAN »). Venus OS publishes every
//! D-Bus value as `N/<portal>/<service>/<instance>/<path>` with a JSON body
//! `{"value": …}`, but only while a client keeps asking: a keepalive on
//! `R/<portal>/keepalive` at least once a minute. Writes go to
//! `W/<portal>/<service>/<instance>/<path>`.
//!
//! One Moli device per GX service seen (`battery/512`, `solarcharger/289`,
//! `vebus/288`, `evcharger/40`…), with the reviewed points of [`TABLE`].
//! Only the few settings listed there are writable, each with its bounds,
//! and all of them as machine commands (`Semantic::Control`): an order from
//! an agent, the assistant or an automation waits for a human.
//!
//! Topic paths, units and state codes from the documented Venus OS D-Bus
//! paths and from victron_mqtt (Apache-2.0, see `integrations/victron`).

use std::collections::{BTreeMap, HashMap};
use std::time::Duration;

use moli_core::{Access, Device, DeviceId, Kind, PointSpec, Semantic, Unit, Value};
use moli_runtime::{BoxFuture, CommandRequest, Driver, DriverCtx};
use rumqttc::{AsyncClient, Event, MqttOptions, Packet, QoS};
use serde::Deserialize;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// The GX device's address (its MQTT broker, « MQTT on LAN » enabled).
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
    /// The installation's portal id (VRM id, the GX's serial). Without it,
    /// the first one the broker announces (`N/+/system/0/Serial`).
    #[serde(default)]
    pub portal_id: Option<String>,
    /// Seconds between keepalives (Venus OS stops publishing after 60 s
    /// without one).
    #[serde(default = "default_keepalive")]
    pub keepalive_secs: u64,
}

fn default_port() -> u16 {
    1883
}

fn default_keepalive() -> u64 {
    30
}

/// How a topic's value reads, and whether Moli may write it.
#[derive(Clone, Copy, Debug)]
pub enum Read {
    /// A number, rounded to `round` decimals.
    Number { unit: Option<UnitKind>, round: u8 },
    /// A state code, named by the table (unknown codes read as missing).
    Code(&'static [(i64, &'static str)]),
    /// 0 / 1.
    OnOff,
    /// A mode the GX shows through several codes: `(name, code written,
    /// codes read as it)`. Venus OS takes the first code and moves on by
    /// itself to the others (ESS: 1 becomes 2 to 8 as the battery goes).
    Group(&'static [(&'static str, i64, &'static [i64])]),
}

/// Units used here (`moli_core::Unit` is not `Copy`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnitKind {
    Watt,
    KiloWattHour,
    Volt,
    Ampere,
    Celsius,
    Percent,
    Second,
}

impl UnitKind {
    fn unit(self) -> Unit {
        match self {
            Self::Watt => Unit::Watt,
            Self::KiloWattHour => Unit::KiloWattHour,
            Self::Volt => Unit::Volt,
            Self::Ampere => Unit::Ampere,
            Self::Celsius => Unit::Celsius,
            Self::Percent => Unit::Percent,
            Self::Second => Unit::Second,
        }
    }
}

/// What a write accepts (the hub checks numbers against these bounds).
#[derive(Clone, Copy, Debug)]
pub struct Bounds {
    pub min: f64,
    pub max: f64,
    pub step: f64,
    /// -1 also means « no limit » (the GX's own word for it), on top of
    /// `min..=max`; the point then declares -1 as its minimum.
    pub no_limit: bool,
}

/// One reviewed topic: `<service>/<instance>/<path>` → a point.
#[derive(Debug)]
pub struct Topic {
    pub service: &'static str,
    pub path: &'static str,
    pub key: &'static str,
    /// Catalogue key of the label (`pilotes.victron.<…>`).
    pub label: &'static str,
    pub read: Read,
    /// Writable: `Some` for numbers (with their bounds) and for codes and
    /// on/off (the bounds are then unused).
    pub write: Option<Bounds>,
}

const VEBUS_STATE: &[(i64, &str)] = &[
    (0, "off"),
    (1, "low_power"),
    (2, "fault"),
    (3, "bulk"),
    (4, "absorption"),
    (5, "float"),
    (6, "storage"),
    (7, "equalize"),
    (8, "passthrough"),
    (9, "inverting"),
    (10, "power_assist"),
    (11, "power_supply"),
    (244, "sustain"),
    (245, "starting_up"),
    (246, "repeated_absorption"),
    (247, "auto_equalize"),
    (248, "battery_safe"),
    (252, "external_control"),
];
const VEBUS_MODE: &[(i64, &str)] = &[
    (1, "charger_only"),
    (2, "inverter_only"),
    (3, "on"),
    (4, "off"),
];
const EV_STATUS: &[(i64, &str)] = &[
    (0, "disconnected"),
    (1, "connected"),
    (2, "charging"),
    (3, "charged"),
    (4, "waiting_for_sun"),
    (5, "waiting_for_rfid"),
    (6, "waiting_for_start"),
    (7, "low_soc"),
    (8, "ground_test_error"),
    (9, "welded_contacts_error"),
    (10, "cp_input_error"),
    (11, "residual_current"),
    (12, "undervoltage"),
    (13, "overvoltage"),
    (14, "overheating"),
    (20, "charging_limit"),
    (21, "start_charging"),
    (22, "switching_to_3_phase"),
    (23, "switching_to_1_phase"),
];
const EV_MODE: &[(i64, &str)] = &[(0, "manual"), (1, "auto"), (2, "scheduled")];
/// ESS modes (`Settings/CGwacs/BatteryLife/State`), as the GX's own menu
/// writes and reads them: 1 « optimized with BatteryLife » (then 2 to 8 as
/// it goes), 9 « keep batteries charged », 10 « optimized without
/// BatteryLife » (then 11, 12). Read on a real GX: 11.
const ESS_MODE: &[(&str, i64, &[i64])] = &[
    ("optimized_batterylife", 1, &[1, 2, 3, 4, 5, 6, 7, 8]),
    ("optimized", 10, &[10, 11, 12]),
    ("keep_charged", 9, &[9]),
];

const fn num(unit: UnitKind, round: u8) -> Read {
    Read::Number {
        unit: Some(unit),
        round,
    }
}

const fn topic(
    service: &'static str,
    path: &'static str,
    key: &'static str,
    label: &'static str,
    read: Read,
) -> Topic {
    Topic {
        service,
        path,
        key,
        label,
        read,
        write: None,
    }
}

const fn writable(
    service: &'static str,
    path: &'static str,
    key: &'static str,
    label: &'static str,
    read: Read,
    bounds: Bounds,
) -> Topic {
    Topic {
        service,
        path,
        key,
        label,
        read,
        write: Some(bounds),
    }
}

/// Bounds unused for codes and on/off.
const ANY: Bounds = Bounds {
    min: 0.0,
    max: 0.0,
    step: 0.0,
    no_limit: false,
};

/// A power limit of the ESS: 0 to 10 kW, or -1 for none.
const POWER_LIMIT: Bounds = Bounds {
    min: 0.0,
    max: 10_000.0,
    step: 100.0,
    no_limit: true,
};

/// The reviewed topics. Writable ones are the user's own settings, never a
/// protection (no charge voltage, no current limit of the battery: the ESS
/// power ceilings sit above what the BMS allows, never in its place).
pub const TABLE: &[Topic] = &[
    // The battery (its BMS or monitor).
    topic(
        "battery",
        "Soc",
        "soc",
        "pilotes.victron.soc",
        num(UnitKind::Percent, 1),
    ),
    topic(
        "battery",
        "Dc/0/Power",
        "power",
        "pilotes.victron.puissance",
        num(UnitKind::Watt, 0),
    ),
    topic(
        "battery",
        "Dc/0/Voltage",
        "voltage",
        "pilotes.victron.tension",
        num(UnitKind::Volt, 2),
    ),
    topic(
        "battery",
        "Dc/0/Current",
        "current",
        "pilotes.victron.courant",
        num(UnitKind::Ampere, 1),
    ),
    topic(
        "battery",
        "Dc/0/Temperature",
        "temperature",
        "pilotes.victron.temperature",
        num(UnitKind::Celsius, 1),
    ),
    topic(
        "battery",
        "History/ChargedEnergy",
        "charged_energy",
        "pilotes.victron.energie_chargee",
        num(UnitKind::KiloWattHour, 2),
    ),
    topic(
        "battery",
        "History/DischargedEnergy",
        "discharged_energy",
        "pilotes.victron.energie_dechargee",
        num(UnitKind::KiloWattHour, 2),
    ),
    // A solar charge controller (MPPT).
    topic(
        "solarcharger",
        "Yield/Power",
        "power",
        "pilotes.victron.puissance_pv",
        num(UnitKind::Watt, 0),
    ),
    topic(
        "solarcharger",
        "Pv/V",
        "pv_voltage",
        "pilotes.victron.tension_pv",
        num(UnitKind::Volt, 1),
    ),
    topic(
        "solarcharger",
        "History/Daily/0/Yield",
        "yield_today",
        "pilotes.victron.production_jour",
        num(UnitKind::KiloWattHour, 2),
    ),
    topic(
        "solarcharger",
        "Yield/User",
        "yield_total",
        "pilotes.victron.production_totale",
        num(UnitKind::KiloWattHour, 2),
    ),
    topic(
        "solarcharger",
        "State",
        "state",
        "pilotes.victron.etat",
        Read::Code(VEBUS_STATE),
    ),
    // The inverter/charger (MultiPlus, Quattro).
    topic(
        "vebus",
        "State",
        "state",
        "pilotes.victron.etat",
        Read::Code(VEBUS_STATE),
    ),
    writable(
        "vebus",
        "Mode",
        "mode",
        "pilotes.victron.mode_onduleur",
        Read::Code(VEBUS_MODE),
        ANY,
    ),
    topic(
        "vebus",
        "Ac/Out/L1/P",
        "ac_out_power",
        "pilotes.victron.puissance_sortie",
        num(UnitKind::Watt, 0),
    ),
    topic(
        "vebus",
        "Dc/0/Power",
        "dc_power",
        "pilotes.victron.puissance_dc",
        num(UnitKind::Watt, 0),
    ),
    // An EV charging station (EVCS).
    topic(
        "evcharger",
        "Status",
        "status",
        "pilotes.victron.etat_borne",
        Read::Code(EV_STATUS),
    ),
    writable(
        "evcharger",
        "Mode",
        "mode",
        "pilotes.victron.mode_borne",
        Read::Code(EV_MODE),
        ANY,
    ),
    writable(
        "evcharger",
        "StartStop",
        "charging_enabled",
        "pilotes.victron.charge_autorisee",
        Read::OnOff,
        ANY,
    ),
    writable(
        "evcharger",
        "SetCurrent",
        "set_current",
        "pilotes.victron.courant_consigne",
        num(UnitKind::Ampere, 0),
        Bounds {
            min: 6.0,
            max: 32.0,
            step: 1.0,
            no_limit: false,
        },
    ),
    topic(
        "evcharger",
        "Current",
        "current",
        "pilotes.victron.courant",
        num(UnitKind::Ampere, 1),
    ),
    topic(
        "evcharger",
        "Ac/Power",
        "power",
        "pilotes.victron.puissance",
        num(UnitKind::Watt, 0),
    ),
    topic(
        "evcharger",
        "Ac/Energy/Forward",
        "energy",
        "pilotes.victron.energie",
        num(UnitKind::KiloWattHour, 2),
    ),
    topic(
        "evcharger",
        "Session/Energy",
        "session_energy",
        "pilotes.victron.energie_session",
        num(UnitKind::KiloWattHour, 2),
    ),
    topic(
        "evcharger",
        "ChargingTime",
        "charging_time",
        "pilotes.victron.duree_charge",
        num(UnitKind::Second, 0),
    ),
    // A grid meter and a PV inverter (any meter the GX knows).
    topic(
        "grid",
        "Ac/Power",
        "power",
        "pilotes.victron.puissance",
        num(UnitKind::Watt, 0),
    ),
    topic(
        "grid",
        "Ac/Energy/Forward",
        "energy",
        "pilotes.victron.energie",
        num(UnitKind::KiloWattHour, 2),
    ),
    topic(
        "grid",
        "Ac/Energy/Reverse",
        "energy_returned",
        "pilotes.victron.energie_renvoyee",
        num(UnitKind::KiloWattHour, 2),
    ),
    topic(
        "pvinverter",
        "Ac/Power",
        "power",
        "pilotes.victron.puissance",
        num(UnitKind::Watt, 0),
    ),
    topic(
        "pvinverter",
        "Ac/Energy/Forward",
        "energy",
        "pilotes.victron.energie",
        num(UnitKind::KiloWattHour, 2),
    ),
    // The system overview.
    topic(
        "system",
        "Ac/Consumption/L1/Power",
        "consumption",
        "pilotes.victron.consommation",
        num(UnitKind::Watt, 0),
    ),
    topic(
        "system",
        "Dc/Pv/Power",
        "pv_dc_power",
        "pilotes.victron.puissance_pv",
        num(UnitKind::Watt, 0),
    ),
    topic(
        "system",
        "Dc/Battery/Soc",
        "battery_soc",
        "pilotes.victron.soc",
        num(UnitKind::Percent, 1),
    ),
    topic(
        "system",
        "SystemState/State",
        "state",
        "pilotes.victron.etat",
        Read::Code(VEBUS_STATE),
    ),
    // ESS (energy storage system) settings.
    writable(
        "settings",
        "Settings/CGwacs/BatteryLife/MinimumSocLimit",
        "minimum_soc",
        "pilotes.victron.soc_minimum",
        num(UnitKind::Percent, 0),
        Bounds {
            min: 10.0,
            max: 100.0,
            step: 5.0,
            no_limit: false,
        },
    ),
    writable(
        "settings",
        "Settings/CGwacs/BatteryLife/State",
        "ess_mode",
        "pilotes.victron.mode_ess",
        Read::Group(ESS_MODE),
        ANY,
    ),
    // What the house draws from the grid when the ESS balances (W, < 0
    // sends to it).
    writable(
        "settings",
        "Settings/CGwacs/AcPowerSetPoint",
        "grid_setpoint",
        "pilotes.victron.consigne_reseau",
        num(UnitKind::Watt, 0),
        Bounds {
            min: -5_000.0,
            max: 5_000.0,
            step: 10.0,
            no_limit: false,
        },
    ),
    // The ESS's own ceilings (the BMS and DVCC still cap below them).
    writable(
        "settings",
        "Settings/CGwacs/MaxChargePower",
        "max_charge_power",
        "pilotes.victron.charge_max",
        num(UnitKind::Watt, 0),
        POWER_LIMIT,
    ),
    writable(
        "settings",
        "Settings/CGwacs/MaxDischargePower",
        "max_discharge_power",
        "pilotes.victron.decharge_max",
        num(UnitKind::Watt, 0),
        POWER_LIMIT,
    ),
];

/// The services Moli makes a device of, and the label of their model.
const SERVICES: &[(&str, &str)] = &[
    ("battery", "pilotes.victron.batterie"),
    ("solarcharger", "pilotes.victron.mppt"),
    ("vebus", "pilotes.victron.onduleur"),
    ("evcharger", "pilotes.victron.borne"),
    ("grid", "pilotes.victron.compteur_reseau"),
    ("pvinverter", "pilotes.victron.onduleur_pv"),
    ("system", "pilotes.victron.systeme"),
    ("settings", "pilotes.victron.ess"),
];

#[derive(Debug)]
pub struct Victron {
    config: Config,
}

impl Victron {
    #[must_use]
    pub fn new(config: Config) -> Self {
        Self { config }
    }
}

impl Driver for Victron {
    fn kind(&self) -> &'static str {
        "victron"
    }

    fn run<'a>(&'a self, ctx: &'a mut DriverCtx) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(run(&self.config, ctx))
    }
}

/// `N/<portal>/<service>/<instance>/<path>` → (service, instance, path).
#[must_use]
pub fn split_topic<'a>(portal: &str, topic: &'a str) -> Option<(&'a str, &'a str, &'a str)> {
    let rest = topic
        .strip_prefix("N/")?
        .strip_prefix(portal)?
        .strip_prefix('/')?;
    let (service, rest) = rest.split_once('/')?;
    let (instance, path) = rest.split_once('/')?;
    (!service.is_empty() && !instance.is_empty() && !path.is_empty())
        .then_some((service, instance, path))
}

/// The reviewed topic for this service and path.
#[must_use]
pub fn find(service: &str, path: &str) -> Option<&'static Topic> {
    TABLE
        .iter()
        .find(|t| t.service == service && t.path == path)
}

/// A published body (`{"value": …}`) as this topic's point value: `Null`
/// when the GX has no value, `None` when the body makes no sense.
#[must_use]
pub fn decode(read: Read, body: &[u8]) -> Option<Value> {
    #[derive(Deserialize)]
    struct Body {
        value: serde_json::Value,
    }
    let body: Body = serde_json::from_slice(body).ok()?;
    if body.value.is_null() {
        return Some(Value::Null);
    }
    let number = body.value.as_f64()?;
    if !number.is_finite() {
        return None;
    }
    Some(match read {
        Read::Number { round, .. } => {
            let factor = 10f64.powi(i32::from(round));
            Value::Float((number * factor).round() / factor)
        }
        Read::Code(codes) => {
            #[allow(clippy::cast_possible_truncation)] // codes are small integers
            let code = number.round() as i64;
            match codes.iter().find(|(c, _)| *c == code) {
                Some((_, name)) => Value::Text((*name).into()),
                None => Value::Null,
            }
        }
        Read::OnOff => Value::Bool(number != 0.0),
        Read::Group(modes) => {
            #[allow(clippy::cast_possible_truncation)] // codes are small integers
            let code = number.round() as i64;
            match modes.iter().find(|(_, _, read)| read.contains(&code)) {
                Some((name, _, _)) => Value::Text((*name).into()),
                None => Value::Null,
            }
        }
    })
}

/// A command's value as the body to write, refused when the point is not
/// writable or the value does not fit.
pub fn encode(topic: &Topic, value: &Value) -> Result<String, String> {
    let Some(bounds) = topic.write else {
        return Err(moli_i18n::tr!(
            "pilotes.victron.ne_se_regle_pas",
            point = topic.key
        ));
    };
    let out = match (topic.read, value) {
        (Read::OnOff, Value::Bool(on)) => serde_json::json!(i64::from(*on)),
        (Read::Code(codes), Value::Text(name)) => match codes.iter().find(|(_, n)| *n == &**name) {
            Some((code, _)) => serde_json::json!(code),
            None => {
                return Err(moli_i18n::tr!(
                    "pilotes.victron.valeur_refusee",
                    point = topic.key
                ));
            }
        },
        (Read::Group(modes), Value::Text(name)) => {
            match modes.iter().find(|(n, _, _)| *n == &**name) {
                Some((_, code, _)) => serde_json::json!(code),
                None => {
                    return Err(moli_i18n::tr!(
                        "pilotes.victron.valeur_refusee",
                        point = topic.key
                    ));
                }
            }
        }
        (Read::Number { .. }, Value::Float(_) | Value::Int(_)) => {
            let v = value.as_f64().unwrap_or(f64::NAN);
            #[allow(clippy::float_cmp)] // -1 is written as is, never computed
            let none = bounds.no_limit && v == -1.0;
            if !(none || (v.is_finite() && bounds.min <= v && v <= bounds.max)) {
                return Err(moli_i18n::tr!(
                    "pilotes.victron.valeur_refusee",
                    point = topic.key
                ));
            }
            serde_json::json!(v)
        }
        _ => {
            return Err(moli_i18n::tr!(
                "pilotes.victron.valeur_refusee",
                point = topic.key
            ));
        }
    };
    Ok(serde_json::json!({ "value": out }).to_string())
}

fn point(topic: &Topic) -> PointSpec {
    let (kind, unit) = match topic.read {
        Read::Number { unit, .. } => (
            Kind::Numeric {
                min: topic.write.map(|b| if b.no_limit { -1.0 } else { b.min }),
                max: topic.write.map(|b| b.max),
                step: topic.write.map(|b| b.step),
            },
            unit.map(UnitKind::unit),
        ),
        Read::Code(codes) => (
            Kind::Enum {
                values: codes.iter().map(|(_, n)| (*n).into()).collect(),
            },
            None,
        ),
        Read::OnOff => (Kind::Binary, None),
        Read::Group(modes) => (
            Kind::Enum {
                values: modes.iter().map(|(n, _, _)| (*n).into()).collect(),
            },
            None,
        ),
    };
    // Every write is a machine command: never run unattended.
    let semantic = if topic.write.is_some() {
        Semantic::Control
    } else {
        Semantic::infer(topic.key, unit.as_ref())
    };
    PointSpec {
        key: topic.key.into(),
        label: moli_i18n::tr(topic.label).into(),
        kind,
        access: Access {
            read: true,
            write: topic.write.is_some(),
        },
        unit,
        semantic,
    }
}

/// The Moli device of one GX service (`battery_512`).
fn device(ctx: &DriverCtx, service: &str, instance: &str, name: Option<&str>) -> Device {
    let model = SERVICES
        .iter()
        .find(|(s, _)| *s == service)
        .map_or_else(|| service.to_owned(), |(_, label)| moli_i18n::tr(label));
    Device {
        id: ctx.device_id(&native_id(service, instance)),
        instance: ctx.instance().clone(),
        native_name: name.unwrap_or(&model).into(),
        manufacturer: Some("Victron Energy".into()),
        model: Some(model.into()),
        description: None,
        native_room: None,
        members: Vec::new(),
        points: TABLE
            .iter()
            .filter(|t| t.service == service)
            .map(point)
            .collect(),
    }
}

#[must_use]
pub fn native_id(service: &str, instance: &str) -> String {
    format!("{service}_{instance}")
}

/// The topics to follow: each reviewed path of each service, and the names
/// the GX gives its devices.
#[must_use]
pub fn subscriptions(portal: &str) -> Vec<String> {
    let mut out: Vec<String> = TABLE
        .iter()
        .map(|t| format!("N/{portal}/{}/+/{}", t.service, t.path))
        .collect();
    for (service, _) in SERVICES {
        for name in ["ProductName", "CustomName"] {
            out.push(format!("N/{portal}/{service}/+/{name}"));
        }
    }
    out.sort();
    out.dedup();
    out
}

/// A GX device Moli shows.
#[derive(Debug, Default)]
struct Known {
    product: Option<String>,
    custom: Option<String>,
    shown: bool,
    /// Marked unavailable when the GX went out of reach.
    away: bool,
}

impl Known {
    /// A value came: whether the device must be told available again (first
    /// seen, or back after the GX was out of reach).
    fn back(&mut self) -> bool {
        let back = self.away;
        self.away = false;
        back
    }

    fn name(&self) -> Option<&str> {
        self.custom
            .as_deref()
            .filter(|n| !n.trim().is_empty())
            .or(self.product.as_deref())
    }
}

#[allow(clippy::too_many_lines)] // one select loop, like the other MQTT drivers
async fn run(config: &Config, ctx: &mut DriverCtx) -> anyhow::Result<()> {
    let mut options = MqttOptions::new(
        format!("moli-{}", ctx.instance()),
        &config.host,
        config.port,
    );
    options.set_keep_alive(Duration::from_secs(30));
    options.set_max_packet_size(64 * 1024, 16 * 1024);
    let (client, mut events) = AsyncClient::new(options, 64);
    let mut portal = config.portal_id.clone();
    // (service, instance) by Moli device, to route commands.
    let mut known: BTreeMap<(String, String), Known> = BTreeMap::new();
    let mut routes: HashMap<DeviceId, (String, String)> = HashMap::new();
    let mut connected = false;
    let mut first_keepalive = true;
    let every = Duration::from_secs(config.keepalive_secs.clamp(5, 55));
    let mut keepalive = tokio::time::interval(every);
    let mut paused_until: Option<tokio::time::Instant> = None;
    ctx.ready();
    loop {
        tokio::select! {
            command = ctx.next_command() => {
                let Some(command) = command else { return Ok(()) };
                let target = routes.get(&command.device.id).cloned();
                match (connected, portal.as_deref(), target) {
                    (true, Some(p), Some((service, instance))) => {
                        write(&client, p, &service, &instance, command);
                    }
                    _ => command.reply(Err(moli_i18n::tr!("pilotes.victron.gx_injoignable"))),
                }
            }
            _ = keepalive.tick(), if connected && portal.is_some() => {
                // The first asks for everything, then only changes.
                let body = if first_keepalive { "" } else { r#"{"keepalive-options": ["suppress-republish"]}"# };
                if let Some(p) = &portal
                    && client.try_publish(format!("R/{p}/keepalive"), QoS::AtMostOnce, false, body).is_ok()
                {
                    first_keepalive = false;
                }
            }
            () = async { tokio::time::sleep_until(paused_until.unwrap_or_else(tokio::time::Instant::now)).await }, if paused_until.is_some() => {
                paused_until = None;
            }
            event = events.poll(), if paused_until.is_none() => match event {
                Ok(Event::Incoming(Packet::ConnAck(_))) => {
                    connected = true;
                    first_keepalive = true;
                    let topics = match &portal {
                        Some(p) => subscriptions(p),
                        None => vec!["N/+/system/0/Serial".to_owned()],
                    };
                    for topic in topics {
                        if let Err(e) = client.try_subscribe(topic, QoS::AtMostOnce) {
                            tracing::warn!(instance = %ctx.instance(), error = %e, "victron subscription not sent");
                        }
                    }
                    keepalive.reset_immediately();
                }
                Ok(Event::Incoming(Packet::Publish(publish))) => {
                    if portal.is_none() {
                        // Discovery: the first GX that says its serial.
                        if let Some(id) = publish.topic.strip_prefix("N/").and_then(|t| t.strip_suffix("/system/0/Serial"))
                            && !id.is_empty()
                            && !id.contains('/')
                        {
                            portal = Some(id.to_owned());
                            let _ = client.try_unsubscribe("N/+/system/0/Serial");
                            for topic in subscriptions(id) {
                                let _ = client.try_subscribe(topic, QoS::AtMostOnce);
                            }
                            keepalive.reset_immediately();
                        }
                        continue;
                    }
                    let Some(p) = portal.as_deref() else { continue };
                    let Some((service, instance, path)) = split_topic(p, &publish.topic) else { continue };
                    let entry = (service.to_owned(), instance.to_owned());
                    if path == "ProductName" || path == "CustomName" {
                        let name = decode_text(&publish.payload);
                        let k = known.entry(entry.clone()).or_default();
                        if path == "ProductName" { k.product = name; } else { k.custom = name; }
                        if k.shown {
                            ctx.upsert_device(device(ctx, service, instance, k.name()));
                        }
                        continue;
                    }
                    let Some(topic) = find(service, path) else { continue };
                    let Some(value) = decode(topic.read, &publish.payload) else {
                        tracing::debug!(instance = %ctx.instance(), topic = %publish.topic, "victron value not understood");
                        continue;
                    };
                    let k = known.entry(entry.clone()).or_default();
                    let id = ctx.device_id(&native_id(service, instance));
                    if !k.shown {
                        k.shown = true;
                        ctx.upsert_device(device(ctx, service, instance, k.name()));
                        ctx.set_availability(&id, true);
                        routes.insert(id.clone(), entry);
                    } else if k.back() {
                        // Back after a cut (the network, the box restarting):
                        // without this, it stayed « unavailable » with fresh values.
                        ctx.set_availability(&id, true);
                    }
                    ctx.set_state(&id, topic.key, value);
                }
                Ok(_) => {}
                Err(e) => {
                    connected = false;
                    tracing::debug!(instance = %ctx.instance(), error = %e, "victron GX unreachable");
                    for id in routes.keys() {
                        ctx.set_availability(id, false);
                    }
                    for k in known.values_mut() {
                        k.away = k.shown;
                    }
                    paused_until = Some(tokio::time::Instant::now() + Duration::from_secs(5));
                }
            },
        }
    }
}

fn decode_text(body: &[u8]) -> Option<String> {
    #[derive(Deserialize)]
    struct Body {
        value: Option<String>,
    }
    serde_json::from_slice::<Body>(body)
        .ok()?
        .value
        .filter(|s| !s.is_empty() && s.len() <= 64)
}

fn write(
    client: &AsyncClient,
    portal: &str,
    service: &str,
    instance: &str,
    command: CommandRequest,
) {
    if command.abandoned() {
        // Too late: never act on an order nobody waits for.
        return;
    }
    let result = TABLE
        .iter()
        .find(|t| t.service == service && t.key == &*command.key)
        .ok_or_else(|| moli_i18n::tr!("pilotes.victron.ne_se_regle_pas", point = command.key))
        .and_then(|topic| encode(topic, &command.value).map(|body| (topic, body)))
        .and_then(|(topic, body)| {
            client
                .try_publish(
                    format!("W/{portal}/{service}/{instance}/{}", topic.path),
                    QoS::AtLeastOnce,
                    false,
                    body,
                )
                .map_err(|e| e.to_string())
        });
    command.reply(result);
}

#[cfg(test)]
mod tests {
    use super::*;

    const PORTAL: &str = "aabbccddee01";

    #[test]
    fn topics_split_into_service_instance_and_path() {
        assert_eq!(
            split_topic(PORTAL, "N/aabbccddee01/battery/512/Dc/0/Power"),
            Some(("battery", "512", "Dc/0/Power"))
        );
        assert_eq!(split_topic(PORTAL, "N/aabbccddee02/battery/512/Soc"), None);
        assert_eq!(split_topic(PORTAL, "N/aabbccddee01/battery"), None);
        assert_eq!(split_topic(PORTAL, "W/aabbccddee01/battery/512/Soc"), None);
    }

    #[test]
    fn bodies_become_values() {
        let soc = find("battery", "Soc").unwrap();
        assert_eq!(
            decode(soc.read, br#"{"value": 51.04}"#),
            Some(Value::Float(51.0))
        );
        let volts = find("battery", "Dc/0/Voltage").unwrap();
        assert_eq!(
            decode(volts.read, br#"{"value":52.790000915527344}"#),
            Some(Value::Float(52.79))
        );
        assert_eq!(decode(soc.read, br#"{"value": null}"#), Some(Value::Null));
        assert_eq!(decode(soc.read, b"not json"), None);
        assert_eq!(decode(soc.read, br#"{"value": "x"}"#), None);
        let status = find("evcharger", "Status").unwrap();
        assert_eq!(
            decode(status.read, br#"{"value": 7}"#),
            Some(Value::Text("low_soc".into()))
        );
        // A code this table does not know: no value, not a wrong one.
        assert_eq!(decode(status.read, br#"{"value": 99}"#), Some(Value::Null));
        let enabled = find("evcharger", "StartStop").unwrap();
        assert_eq!(
            decode(enabled.read, br#"{"value": 1}"#),
            Some(Value::Bool(true))
        );
        // Settings carry their bounds next to the value.
        let min = find("settings", "Settings/CGwacs/BatteryLife/MinimumSocLimit").unwrap();
        assert_eq!(
            decode(min.read, br#"{"max":100.0,"min":0.0,"value":50.0}"#),
            Some(Value::Float(50.0))
        );
    }

    #[test]
    fn only_reviewed_settings_are_written_within_bounds() {
        let current = find("evcharger", "SetCurrent").unwrap();
        assert_eq!(
            encode(current, &Value::Float(10.0)).unwrap(),
            r#"{"value":10.0}"#
        );
        assert!(encode(current, &Value::Float(40.0)).is_err());
        assert!(encode(current, &Value::Float(2.0)).is_err());
        let enabled = find("evcharger", "StartStop").unwrap();
        assert_eq!(
            encode(enabled, &Value::Bool(false)).unwrap(),
            r#"{"value":0}"#
        );
        let mode = find("vebus", "Mode").unwrap();
        assert_eq!(
            encode(mode, &Value::Text("charger_only".into())).unwrap(),
            r#"{"value":1}"#
        );
        assert!(encode(mode, &Value::Text("turbo".into())).is_err());
        let soc = find("battery", "Soc").unwrap();
        assert!(
            encode(soc, &Value::Float(50.0)).is_err(),
            "a measure is never written"
        );
        let min = find("settings", "Settings/CGwacs/BatteryLife/MinimumSocLimit").unwrap();
        assert!(
            encode(min, &Value::Float(5.0)).is_err(),
            "below 10 % protects nothing"
        );
        let setpoint = find("settings", "Settings/CGwacs/AcPowerSetPoint").unwrap();
        assert_eq!(
            encode(setpoint, &Value::Float(-50.0)).unwrap(),
            r#"{"value":-50.0}"#
        );
        assert!(encode(setpoint, &Value::Float(20_000.0)).is_err());
    }

    #[test]
    fn ess_modes_write_the_first_code_and_read_the_others() {
        let mode = find("settings", "Settings/CGwacs/BatteryLife/State").unwrap();
        for (code, name) in [
            (1, "optimized_batterylife"),
            (5, "optimized_batterylife"),
            (9, "keep_charged"),
            (10, "optimized"),
            (11, "optimized"),
        ] {
            assert_eq!(
                decode(mode.read, format!(r#"{{"value":{code}}}"#).as_bytes()),
                Some(Value::Text(name.into())),
                "{code}"
            );
        }
        assert_eq!(decode(mode.read, br#"{"value":0}"#), Some(Value::Null));
        assert_eq!(
            encode(mode, &Value::Text("optimized_batterylife".into())).unwrap(),
            r#"{"value":1}"#
        );
        assert_eq!(
            encode(mode, &Value::Text("optimized".into())).unwrap(),
            r#"{"value":10}"#
        );
        assert_eq!(
            encode(mode, &Value::Text("keep_charged".into())).unwrap(),
            r#"{"value":9}"#
        );
        assert!(encode(mode, &Value::Text("external".into())).is_err());
    }

    #[test]
    fn power_limits_take_minus_one_for_none() {
        let charge = find("settings", "Settings/CGwacs/MaxChargePower").unwrap();
        assert_eq!(
            encode(charge, &Value::Float(-1.0)).unwrap(),
            r#"{"value":-1.0}"#
        );
        assert_eq!(
            encode(charge, &Value::Float(2500.0)).unwrap(),
            r#"{"value":2500.0}"#
        );
        assert!(encode(charge, &Value::Float(-0.5)).is_err());
        assert!(encode(charge, &Value::Float(-2.0)).is_err());
        assert!(encode(charge, &Value::Float(20_000.0)).is_err());
        match point(charge).kind {
            Kind::Numeric { min, max, .. } => {
                assert_eq!((min, max), (Some(-1.0), Some(10_000.0)));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn every_write_is_a_machine_command() {
        for t in TABLE {
            let spec = point(t);
            assert_eq!(spec.access.write, t.write.is_some(), "{}", t.key);
            if spec.access.write {
                assert_eq!(spec.semantic, Semantic::Control, "{}", t.key);
            }
        }
        // Keys are unique within each service.
        for (service, _) in SERVICES {
            let mut keys: Vec<_> = TABLE
                .iter()
                .filter(|t| t.service == *service)
                .map(|t| t.key)
                .collect();
            let n = keys.len();
            keys.sort_unstable();
            keys.dedup();
            assert_eq!(keys.len(), n, "{service}");
        }
    }

    #[test]
    fn subscriptions_follow_the_table_and_the_names() {
        let subs = subscriptions(PORTAL);
        assert!(subs.contains(&"N/aabbccddee01/battery/+/Soc".to_owned()));
        assert!(subs.contains(&"N/aabbccddee01/evcharger/+/ProductName".to_owned()));
        assert!(subs.iter().all(|s| s.starts_with("N/aabbccddee01/")));
    }

    #[test]
    fn a_gx_capture_reads_through() {
        // Captured from a real GX (portal and names replaced).
        let capture = include_str!("../../../integrations/victron/fixtures/topics.txt");
        let mut read = 0;
        for line in capture.lines().filter(|l| !l.trim().is_empty()) {
            let (topic, body) = line.split_once(' ').unwrap();
            let (service, _, path) = split_topic(PORTAL, topic).unwrap();
            if let Some(t) = find(service, path) {
                let value = decode(t.read, body.as_bytes());
                assert!(matches!(value, Some(v) if v != Value::Null), "{topic}");
                read += 1;
            }
        }
        assert_eq!(read, TABLE.len());
    }

    #[test]
    fn a_device_is_available_again_after_a_cut() {
        let mut k = Known {
            shown: true,
            ..Known::default()
        };
        assert!(!k.back(), "nothing to tell while connected");
        k.away = true; // the GX went out of reach
        assert!(k.back(), "the first value after the cut");
        assert!(!k.back(), "once only");
    }
}
