//! Cars by their plate: which of the cars in a zone are the house's own.
//!
//! Frigate reads plates (`lpr`) and tells, for each tracked car, the plate
//! it read (`<base>/events`, `recognized_license_plate`) and, when the plate
//! is one of its `lpr.known_plates`, that car's name
//! (`<base>/tracked_object_update`, `type: lpr`, `name`). Moli compares the
//! plate with the known ones too (letters and digits only, one character
//! of reading error allowed): Frigate does not always put the name.
//!
//! A car counts as unknown in a zone once it has been followed for `wait`
//! without a known plate: reading a plate takes from 2 to 12 s after the
//! car comes into view. A car whose plate is never read (at night, from
//! behind) is unknown after `wait` too: the alarm still hears of it, later.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use serde_json::Value as Json;
use tokio::time::Instant;

/// How long a car is given for its plate to be read, by default.
pub const WAIT: Duration = Duration::from_secs(15);

/// A car Frigate follows.
#[derive(Debug)]
struct Car {
    camera: String,
    /// The zones it stands in now (`current_zones`).
    zones: BTreeSet<String>,
    /// Parked: Frigate does not count it in the zone any more.
    stationary: bool,
    /// When Moli first heard of it.
    since: Instant,
    /// The house's name for it, once its plate is known.
    name: Option<String>,
}

#[derive(Debug, Default)]
pub struct Plates {
    /// Known plate (letters and digits, uppercase) → the car's name.
    known: Vec<(String, String)>,
    wait: Duration,
    cars: BTreeMap<String, Car>,
}

/// Letters and digits only, uppercase: « ab-123-cd » and « AB123CD » are one plate.
fn normalize(plate: &str) -> String {
    plate
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_uppercase())
        .collect()
}

/// Edits between two short strings (Levenshtein).
fn distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut prev = row[0];
        row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let here = row[j + 1];
            row[j + 1] = if ca == *cb {
                prev
            } else {
                1 + prev.min(row[j]).min(row[j + 1])
            };
            prev = here;
        }
    }
    row[b.len()]
}

/// `/api/config` → the known plates (`lpr.known_plates`: name → plates).
/// A pattern (Frigate accepts regular expressions) is left out: Moli only
/// compares plain plates, and keeps Frigate's own match (`name`).
#[must_use]
pub fn known_plates(config: &Json) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let Some(known) = config["lpr"]["known_plates"].as_object() else {
        return out;
    };
    for (name, plates) in known {
        for plate in plates
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Json::as_str)
        {
            let pattern = plate.chars().any(|c| "^$.*+?()[]{}|\\".contains(c));
            let plain = normalize(plate);
            if !pattern && plain.len() >= 4 {
                out.push((plain, name.clone()));
            }
        }
    }
    out
}

impl Plates {
    #[must_use]
    pub fn new(known: Vec<(String, String)>, wait: Duration) -> Self {
        Self {
            known,
            wait,
            cars: BTreeMap::new(),
        }
    }

    pub fn set_known(&mut self, known: Vec<(String, String)>) {
        self.known = known;
    }

    /// The name of the known car this plate is, one reading error allowed.
    fn name_of(&self, plate: &str) -> Option<String> {
        let plate = normalize(plate);
        if plate.len() < 4 {
            return None;
        }
        self.known
            .iter()
            .find(|(k, _)| distance(k, &plate) <= 1)
            .map(|(_, name)| name.clone())
    }

    /// `<base>/events`: a car followed, its zones, its plate; gone at `end`.
    /// The name of a car newly known, if this message made it known.
    pub fn event(&mut self, payload: &str, now: Instant) -> Option<(String, String)> {
        let message: Json = serde_json::from_str(payload).ok()?;
        let after = &message["after"];
        if after["label"].as_str() != Some("car") {
            return None;
        }
        let id = after["id"].as_str()?.to_owned();
        if message["type"].as_str() == Some("end")
            || after["false_positive"].as_bool() == Some(true)
        {
            self.cars.remove(&id);
            return None;
        }
        let plate = match &after["recognized_license_plate"] {
            Json::String(p) => Some(p.clone()),
            // [plate, score] in some versions.
            Json::Array(a) => a.first().and_then(Json::as_str).map(str::to_owned),
            _ => None,
        };
        let named = plate
            .as_deref()
            .and_then(|p| self.name_of(p))
            .or_else(|| after["sub_label"].as_str().and_then(|s| self.known_name(s)))
            .or_else(|| {
                after["sub_label"][0]
                    .as_str()
                    .and_then(|s| self.known_name(s))
            });
        let car = self.cars.entry(id).or_insert_with(|| Car {
            camera: String::new(),
            zones: BTreeSet::new(),
            stationary: false,
            since: now,
            name: None,
        });
        if let Some(camera) = after["camera"].as_str() {
            camera.clone_into(&mut car.camera);
        }
        car.zones = after["current_zones"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Json::as_str)
            .map(str::to_owned)
            .collect();
        car.stationary = after["stationary"].as_bool().unwrap_or(false);
        Self::named(car, named)
    }

    /// `<base>/tracked_object_update`, `type: lpr`: Frigate's own match
    /// (`name`) or the plate it read.
    pub fn lpr(&mut self, payload: &str, now: Instant) -> Option<(String, String)> {
        let message: Json = serde_json::from_str(payload).ok()?;
        if message["type"].as_str() != Some("lpr") {
            return None;
        }
        let id = message["id"].as_str()?.to_owned();
        let named = message["name"]
            .as_str()
            .and_then(|n| self.known_name(n))
            .or_else(|| message["plate"].as_str().and_then(|p| self.name_of(p)));
        let camera = message["camera"].as_str().unwrap_or_default().to_owned();
        let car = self.cars.entry(id).or_insert_with(|| Car {
            camera,
            zones: BTreeSet::new(),
            stationary: false,
            since: now,
            name: None,
        });
        Self::named(car, named)
    }

    /// A name Frigate gave, if it is one of the known cars' (a sub label
    /// may be something else: a face, a plate read without a match).
    fn known_name(&self, name: &str) -> Option<String> {
        self.known
            .iter()
            .find(|(_, n)| n == name)
            .map(|(_, n)| n.clone())
    }

    /// Names the car; (camera, name) when it was not known before.
    fn named(car: &mut Car, name: Option<String>) -> Option<(String, String)> {
        let name = name?;
        if car.name.is_some() {
            return None;
        }
        car.name = Some(name.clone());
        Some((car.camera.clone(), name))
    }

    /// The cars in `zone` that are not the house's: followed for `wait`
    /// without a known plate, and moving (Frigate leaves parked ones out).
    #[must_use]
    pub fn unknown_in(&self, zone: &str, now: Instant) -> i64 {
        let n = self
            .cars
            .values()
            .filter(|c| {
                c.name.is_none()
                    && !c.stationary
                    && c.zones.contains(zone)
                    && now.duration_since(c.since) >= self.wait
            })
            .count();
        i64::try_from(n).unwrap_or(i64::MAX)
    }

    /// When a car in a zone will turn unknown, if one is still waiting.
    #[must_use]
    pub fn next_deadline(&self, now: Instant) -> Option<Instant> {
        self.cars
            .values()
            .filter(|c| c.name.is_none() && !c.stationary && !c.zones.is_empty())
            .map(|c| c.since + self.wait)
            .filter(|at| *at > now)
            .min()
    }

    /// Forgets the cars Frigate stopped telling about (its `end` lost):
    /// none is kept for more than an hour.
    pub fn forget_old(&mut self, now: Instant) {
        self.cars
            .retain(|_, c| now.duration_since(c.since) < Duration::from_secs(3600));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn known() -> Vec<(String, String)> {
        vec![
            ("AB123CD".into(), "Twingo".into()),
            ("EF456GH".into(), "Mini".into()),
        ]
    }

    fn event(kind: &str, id: &str, zones: &[&str], plate: Option<&str>) -> String {
        json!({
            "type": kind,
            "after": {
                "id": id, "camera": "eye", "label": "car", "current_zones": zones,
                "stationary": false, "recognized_license_plate": plate, "sub_label": null,
            }
        })
        .to_string()
    }

    #[test]
    fn known_plates_from_frigate_config() {
        let config = json!({"lpr": {"known_plates": {
            "Twingo": ["ab-123-cd"], "Mini": ["EF456GH", "^GH.*$"]
        }}});
        let mut k = known_plates(&config);
        k.sort();
        assert_eq!(
            k,
            vec![
                ("AB123CD".into(), "Twingo".into()),
                ("EF456GH".into(), "Mini".into())
            ]
        );
        assert!(known_plates(&json!({})).is_empty());
    }

    #[test]
    fn a_car_without_a_plate_is_unknown_after_the_wait() {
        let t0 = Instant::now();
        let mut p = Plates::new(known(), WAIT);
        assert_eq!(p.event(&event("new", "1", &["allee"], None), t0), None);
        assert_eq!(p.unknown_in("allee", t0), 0);
        assert_eq!(p.next_deadline(t0), Some(t0 + WAIT));
        assert_eq!(p.unknown_in("allee", t0 + WAIT), 1);
        assert_eq!(p.unknown_in("cour", t0 + WAIT), 0);
        p.event(&event("end", "1", &[], None), t0 + WAIT);
        assert_eq!(p.unknown_in("allee", t0 + WAIT), 0);
    }

    #[test]
    fn a_known_plate_in_time_never_counts() {
        let t0 = Instant::now();
        let mut p = Plates::new(known(), WAIT);
        p.event(&event("new", "1", &["allee"], None), t0);
        // Read 12 s in, one character wrong, with dashes.
        let named = p.event(
            &event("update", "1", &["allee"], Some("AB-123-CO")),
            t0 + Duration::from_secs(12),
        );
        assert_eq!(named, Some(("eye".into(), "Twingo".into())));
        assert_eq!(p.unknown_in("allee", t0 + WAIT * 2), 0);
        assert_eq!(p.next_deadline(t0 + Duration::from_secs(12)), None);
        // Said once only.
        assert_eq!(
            p.event(
                &event("update", "1", &["allee"], Some("AB123CD")),
                t0 + WAIT
            ),
            None
        );
    }

    #[test]
    fn frigate_s_own_match_and_a_late_plate() {
        let t0 = Instant::now();
        let mut p = Plates::new(known(), WAIT);
        p.event(&event("new", "1", &["allee"], None), t0);
        assert_eq!(p.unknown_in("allee", t0 + WAIT), 1);
        let lpr = json!({"type": "lpr", "id": "1", "camera": "eye", "name": "Mini", "plate": "??"})
            .to_string();
        assert_eq!(p.lpr(&lpr, t0 + WAIT), Some(("eye".into(), "Mini".into())));
        assert_eq!(p.unknown_in("allee", t0 + WAIT), 0);
        // Another name (a face, an unmatched plate) is no house car.
        let other =
            json!({"type": "lpr", "id": "2", "camera": "eye", "name": "Bob", "plate": "ZZ999ZZ"})
                .to_string();
        assert_eq!(p.lpr(&other, t0), None);
    }

    #[test]
    fn a_far_plate_and_a_parked_car_and_other_labels() {
        let t0 = Instant::now();
        let mut p = Plates::new(known(), WAIT);
        p.event(&event("new", "1", &["allee"], Some("AB999XY")), t0);
        assert_eq!(p.unknown_in("allee", t0 + WAIT), 1);
        let parked = json!({"type": "update", "after": {"id": "1", "camera": "eye", "label": "car",
            "current_zones": ["allee"], "stationary": true}})
        .to_string();
        p.event(&parked, t0 + WAIT);
        assert_eq!(p.unknown_in("allee", t0 + WAIT), 0);
        let person = json!({"type": "new", "after": {"id": "9", "label": "person", "current_zones": ["allee"]}}).to_string();
        p.event(&person, t0);
        assert_eq!(p.unknown_in("allee", t0 + WAIT * 2), 0);
        assert_eq!(p.event("not json", t0), None);
    }

    #[test]
    fn forgotten_after_an_hour() {
        let t0 = Instant::now();
        let mut p = Plates::new(known(), WAIT);
        p.event(&event("new", "1", &["allee"], None), t0);
        p.forget_old(t0 + Duration::from_secs(3601));
        assert_eq!(p.unknown_in("allee", t0 + Duration::from_secs(3601)), 0);
    }

    #[test]
    fn distances() {
        assert_eq!(distance("AB123CD", "AB123CD"), 0);
        assert_eq!(distance("AB123CD", "AB123C"), 1);
        assert_eq!(distance("AB123CD", "XB123CE"), 2);
    }
}
