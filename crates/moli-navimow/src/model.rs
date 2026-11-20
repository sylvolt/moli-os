//! Navimow's smart-home API speaks Google Home's language: the mowers as a
//! SYNC answer (`authList`), their states as a QUERY answer
//! (`getVehicleStatus`), orders as EXECUTE commands (`sendCommands`), each
//! inside Navimow's envelope `{code: 1, desc, data: {payload: …}}`. Pure:
//! answers in, devices and states out.

use serde_json::{Value as Json, json};
use std::collections::BTreeMap;

/// A mower of the account.
#[derive(Clone, Debug, PartialEq)]
pub struct Mower {
    pub id: String,
    pub name: String,
    pub model: Option<String>,
}

/// What a mower is doing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Mowing,
    Paused,
    Docked,
    Returning,
    Idle,
    Error,
    Updating,
    Offline,
    Unknown,
}

impl State {
    pub const ALL: [Self; 9] = [
        Self::Mowing,
        Self::Paused,
        Self::Docked,
        Self::Returning,
        Self::Idle,
        Self::Error,
        Self::Updating,
        Self::Offline,
        Self::Unknown,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Self::Mowing => "mowing",
            Self::Paused => "paused",
            Self::Docked => "docked",
            Self::Returning => "returning",
            Self::Idle => "idle",
            Self::Error => "error",
            Self::Updating => "updating",
            Self::Offline => "offline",
            Self::Unknown => "unknown",
        }
    }

    /// Navimow's words (`vehicleState`), as its own SDK reads them.
    fn from_raw(raw: &str) -> Self {
        match raw {
            "isRunning" | "isMapping" | "mowing" => Self::Mowing,
            "isPaused" | "paused" => Self::Paused,
            "isDocked" | "docked" | "charging" => Self::Docked,
            "isDocking" | "returning" => Self::Returning,
            "isIdle" | "isIdel" | "idle" | "Self-Checking" | "Self-checking" => Self::Idle,
            "Error" | "error" | "isLifted" => Self::Error,
            "inSoftwareUpdate" => Self::Updating,
            "Offline" | "offline" => Self::Offline,
            _ => Self::Unknown,
        }
    }
}

/// A mower's state, read.
#[derive(Clone, Debug, PartialEq)]
pub struct Status {
    pub state: State,
    /// Navimow's own word for it (kept for what Moli does not know).
    pub raw: Option<String>,
    pub battery: Option<f64>,
    pub online: Option<bool>,
}

/// Why an answer is not what was asked.
#[derive(Debug, PartialEq, Eq)]
pub enum Answer {
    /// Navimow refused the token (its envelope says so, or HTTP 401).
    Refused,
    /// Something else: Navimow's words, or an unreadable answer.
    Other(String),
}

/// The payload inside Navimow's envelope.
pub fn payload(status: u16, body: &[u8]) -> Result<Json, Answer> {
    if matches!(status, 401 | 403) {
        return Err(Answer::Refused);
    }
    let json: Json = serde_json::from_slice(body)
        .map_err(|_| Answer::Other(format!("HTTP {status}, not JSON")))?;
    if !(200..300).contains(&status) {
        return Err(Answer::Other(format!("HTTP {status}")));
    }
    match json["code"].as_i64() {
        Some(1) => Ok(json["data"]["payload"].clone()),
        Some(c) if (40100..40200).contains(&c) || c == 401 => Err(Answer::Refused),
        _ => Err(Answer::Other(
            json["desc"]
                .as_str()
                .unwrap_or("unexpected answer")
                .chars()
                .take(120)
                .collect(),
        )),
    }
}

/// The devices of a SYNC-like payload: a list, or a map by id.
fn devices(payload: &Json) -> Vec<(String, &Json)> {
    match &payload["devices"] {
        Json::Array(list) => list
            .iter()
            .filter_map(|d| Some((d["id"].as_str()?.to_owned(), d)))
            .collect(),
        Json::Object(map) => map.iter().map(|(id, d)| (id.clone(), d)).collect(),
        _ => Vec::new(),
    }
}

/// The mowers of the account (`authList`).
pub fn mowers(payload: &Json) -> Vec<Mower> {
    devices(payload)
        .into_iter()
        .filter(|(id, _)| !id.is_empty())
        .map(|(id, d)| {
            let name = d["name"]["name"]
                .as_str()
                .or_else(|| d["name"].as_str())
                .or_else(|| d["deviceName"].as_str())
                .unwrap_or("Navimow")
                .to_owned();
            let model = d["deviceInfo"]["model"]
                .as_str()
                .or_else(|| d["model"].as_str())
                .filter(|m| !m.is_empty())
                .map(str::to_owned);
            Mower { id, name, model }
        })
        .collect()
}

fn battery(d: &Json) -> Option<f64> {
    let pct = |v: &Json| v.as_f64().or_else(|| v.as_str()?.parse().ok());
    if let Some(b) = pct(&d["battery"]) {
        return Some(b.clamp(0.0, 100.0));
    }
    let list = d["capacityRemaining"].as_array()?;
    list.iter()
        .find(|c| {
            c["unit"]
                .as_str()
                .is_some_and(|u| u.eq_ignore_ascii_case("PERCENTAGE"))
        })
        .or_else(|| list.first())
        .and_then(|c| pct(&c["rawValue"]))
        .map(|b| b.clamp(0.0, 100.0))
}

/// The mowers' states (`getVehicleStatus`), by id.
pub fn statuses(payload: &Json) -> BTreeMap<String, Status> {
    devices(payload)
        .into_iter()
        .map(|(id, d)| {
            let raw = ["vehicleState", "status", "state"]
                .iter()
                .find_map(|k| d[*k].as_str())
                .map(str::to_owned);
            let online = d["online"].as_bool();
            let mut state = raw.as_deref().map_or(State::Unknown, State::from_raw);
            if online == Some(false) {
                state = State::Offline;
            } else if state == State::Unknown {
                // Google's booleans, when no word is given.
                if d["isRunning"] == true && d["isPaused"] != true {
                    state = State::Mowing;
                } else if d["isPaused"] == true {
                    state = State::Paused;
                } else if d["isDocked"] == true {
                    state = State::Docked;
                }
            }
            (
                id,
                Status {
                    state,
                    raw,
                    battery: battery(d),
                    online,
                },
            )
        })
        .collect()
}

/// An order to a mower.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Order {
    Start,
    Stop,
    Pause,
    Resume,
    Dock,
}

/// The `sendCommands` body for one order.
pub fn order_body(id: &str, order: Order) -> Json {
    let execution = match order {
        Order::Start => {
            json!({"command": "action.devices.commands.StartStop", "params": {"on": true}})
        }
        Order::Stop => {
            json!({"command": "action.devices.commands.StartStop", "params": {"on": false}})
        }
        Order::Pause => {
            json!({"command": "action.devices.commands.PauseUnpause", "params": {"on": false}})
        }
        Order::Resume => {
            json!({"command": "action.devices.commands.PauseUnpause", "params": {"on": true}})
        }
        Order::Dock => json!({"command": "action.devices.commands.Dock"}),
    };
    json!({"commands": [{"devices": [{"id": id}], "execution": execution}]})
}

/// Whether the order was taken: an error, unless the mower already is so.
pub fn order_result(payload: &Json) -> Result<(), String> {
    for c in payload["commands"].as_array().into_iter().flatten() {
        if c["status"] == "ERROR" {
            let code = c["errorCode"].as_str().unwrap_or("commandFailed");
            if code != "alreadyInState" {
                return Err(code.chars().take(60).collect());
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const AUTH_LIST: &[u8] =
        include_bytes!("../../../integrations/navimow/fixtures/auth-list.json");
    const STATUS: &[u8] =
        include_bytes!("../../../integrations/navimow/fixtures/vehicle-status.json");
    const STATUS_OFFLINE: &[u8] =
        include_bytes!("../../../integrations/navimow/fixtures/vehicle-status-offline.json");
    const ORDER: &[u8] =
        include_bytes!("../../../integrations/navimow/fixtures/send-commands.json");

    #[test]
    fn the_mowers_of_the_account_are_read() {
        let p = payload(200, AUTH_LIST).unwrap();
        let m = mowers(&p);
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].id, "NV0000000000001");
        assert_eq!(m[0].name, "Tondeuse");
        assert_eq!(m[0].model.as_deref(), Some("i105E"));
    }

    #[test]
    fn states_are_navimows_words_made_moli_ones() {
        let s = statuses(&payload(200, STATUS).unwrap());
        let one = &s["NV0000000000001"];
        assert_eq!(one.state, State::Mowing);
        assert_eq!(one.battery, Some(64.0));
        assert_eq!(one.raw.as_deref(), Some("isRunning"));
        let off = statuses(&payload(200, STATUS_OFFLINE).unwrap());
        assert_eq!(off["NV0000000000001"].state, State::Offline);
        for (raw, state) in [
            ("isDocked", State::Docked),
            ("isDocking", State::Returning),
            ("isPaused", State::Paused),
            ("isLifted", State::Error),
            ("inSoftwareUpdate", State::Updating),
            ("Self-Checking", State::Idle),
            ("something new", State::Unknown),
        ] {
            let p = json!({"devices": [{"id": "x", "vehicleState": raw}]});
            assert_eq!(statuses(&p)["x"].state, state, "{raw}");
        }
        // Google's own form: a map by id, booleans, a battery field.
        let g = json!({"devices": {"x": {"online": true, "isRunning": true, "isPaused": true, "battery": "88"}}});
        assert_eq!(statuses(&g)["x"].state, State::Paused);
        assert_eq!(statuses(&g)["x"].battery, Some(88.0));
    }

    #[test]
    fn envelopes_say_refusals_and_errors() {
        assert_eq!(payload(401, b""), Err(Answer::Refused));
        assert_eq!(
            payload(200, br#"{"code":40101,"desc":"token expired"}"#),
            Err(Answer::Refused)
        );
        assert_eq!(
            payload(200, br#"{"code":5000,"desc":"busy"}"#),
            Err(Answer::Other("busy".into()))
        );
        assert!(matches!(payload(500, b"<html>"), Err(Answer::Other(_))));
    }

    #[test]
    fn orders_are_google_commands_and_already_done_is_done() {
        let b = order_body("NV1", Order::Dock);
        assert_eq!(b["commands"][0]["devices"][0]["id"], "NV1");
        assert_eq!(
            b["commands"][0]["execution"]["command"],
            "action.devices.commands.Dock"
        );
        assert_eq!(
            order_body("NV1", Order::Pause)["commands"][0]["execution"]["params"]["on"],
            false
        );
        assert_eq!(
            order_body("NV1", Order::Start)["commands"][0]["execution"]["params"]["on"],
            true
        );
        assert_eq!(order_result(&payload(200, ORDER).unwrap()), Ok(()));
        let failed = json!({"commands": [{"status": "ERROR", "errorCode": "deviceOffline"}]});
        assert_eq!(order_result(&failed), Err("deviceOffline".into()));
        let already = json!({"commands": [{"status": "ERROR", "errorCode": "alreadyInState"}]});
        assert_eq!(order_result(&already), Ok(()));
    }
}
