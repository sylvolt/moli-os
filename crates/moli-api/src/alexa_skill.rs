//! A private Alexa skill (`POST /api/alexa/skill`): « Alexa, demande à
//! Moli si la porte de l'atelier est fermée ». Amazon sends the words, Moli's
//! assistant answers (as when asked aloud: short, said), Alexa reads it out;
//! « et la batterie ? » follows in the same session.
//!
//! Anyone on the Internet can post here, so nothing is believed before it is
//! proved to be Amazon's, for this very skill:
//! - the certificate chain comes only from `https://s3.amazonaws.com/echo.api/…`,
//!   chains to a public CA, is valid now and names `echo-api.amazon.com`;
//! - its key signed the exact body (`Signature-256`, RSA SHA-256);
//! - the request is less than 150 s old;
//! - it is for the skill set up here (`[alexa_skill] skill_id`).
//!
//! The assistant then acts as itself (an agent under the guard): a protected
//! device waits for a person, disarming the alarm wants the code — never
//! given by voice.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use axum::Extension;
use axum::body::Bytes;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use serde_json::{Value as Json, json};

use crate::assistant::Moli;

/// Where Amazon keeps its signing certificates.
const CERT_HOST: &str = "s3.amazonaws.com";
const CERT_PATH: &str = "/echo.api/";
/// The name the signing certificate must carry.
const SIGNER: &str = "echo-api.amazon.com";
/// How old a request may be (Amazon's rule).
const MAX_AGE_S: i64 = 150;
/// Alexa waits 8 s for an answer: what is not ready by then is said so.
const ANSWER_WITHIN: Duration = Duration::from_millis(6_500);
/// A certificate chain is fetched again after this long.
const CHAIN_FOR: Duration = Duration::from_secs(3600);
/// What Alexa reads at most (it says up to 8 000 characters: a sentence
/// or three are what a person listens to).
const MAX_SPOKEN: usize = 600;
/// The conversation kept in the session, turns at most.
const MAX_TURNS: usize = 6;

/// Certificate chains already fetched: URL → (when, certificates).
type Chains = Arc<Mutex<HashMap<String, (Instant, Vec<Vec<u8>>)>>>;

/// `[alexa_skill]`: the skill this house answers.
#[derive(Clone, Debug)]
pub struct AlexaSkill {
    pub skill_id: String,
    /// The point to write `off` to when the voice code is right (the
    /// alarm's `mode`); `None`: no disarming by voice.
    pub disarm: Option<String>,
    chains: Chains,
    lock: Arc<Mutex<VoiceLock>>,
}

impl AlexaSkill {
    #[must_use]
    pub fn new(skill_id: String, disarm: Option<String>) -> Self {
        Self {
            skill_id,
            disarm,
            chains: Arc::default(),
            lock: Arc::default(),
        }
    }
}

/// Where the voice code lives in the vault: `alexa_skill/voice_code`, set by
/// a person through a pipe (`moli-os secrets set alexa_skill voice_code`),
/// never the house's code.
const VOICE_INSTANCE: &str = "alexa_skill";
const VOICE_CODE: &str = "voice_code";
/// Wrong codes in a row before the voice code is locked.
const VOICE_TRIES: u32 = 3;
/// The first lock; each one after doubles it (a day at most).
const VOICE_LOCK: Duration = Duration::from_secs(30 * 60);

/// Wrong voice codes, and how long the voice code is locked.
#[derive(Debug, Default)]
pub(crate) struct VoiceLock {
    failures: u32,
    lockouts: u32,
    until: Option<Instant>,
}

impl VoiceLock {
    /// Locked now, and for how many minutes still.
    pub(crate) fn locked(&self, now: Instant) -> Option<u64> {
        self.until
            .filter(|u| *u > now)
            .map(|u| (u - now).as_secs().div_ceil(60))
    }

    /// A wrong code: locked after [`VOICE_TRIES`] in a row. The minutes of
    /// the lock it starts, if it does.
    pub(crate) fn wrong(&mut self, now: Instant) -> Option<u64> {
        self.failures += 1;
        if self.failures < VOICE_TRIES {
            return None;
        }
        self.failures = 0;
        let lock = VOICE_LOCK
            .saturating_mul(1 << self.lockouts.min(6))
            .min(Duration::from_secs(24 * 3600));
        self.lockouts += 1;
        self.until = Some(now + lock);
        Some(lock.as_secs() / 60)
    }

    pub(crate) fn right(&mut self) {
        *self = Self::default();
    }
}

/// The code as heard: digits only (« 12 34 » → « 1234 »), 4 to 8 of them.
pub(crate) fn heard_code(slot: &str) -> Option<String> {
    let digits: String = slot.chars().filter(char::is_ascii_digit).collect();
    let others = slot
        .chars()
        .any(|c| !c.is_ascii_digit() && !c.is_whitespace());
    (!others && (4..=8).contains(&digits.len())).then_some(digits)
}

/// The same code, compared on the digests of their digits (no early exit on
/// the text; « 20 15 » kept is « 2015 » heard).
pub(crate) fn same_code(heard: &str, kept: &str) -> bool {
    let digits = |s: &str| s.chars().filter(char::is_ascii_digit).collect::<String>();
    let d = |s: &str| ring::digest::digest(&ring::digest::SHA256, digits(s).as_bytes());
    !digits(kept).is_empty() && d(heard).as_ref() == d(kept).as_ref()
}

/// The certificate URL is Amazon's, and only Amazon's.
pub(crate) fn cert_url_ok(url: &str) -> bool {
    let Some(rest) = url
        .get(..8)
        .filter(|s| s.eq_ignore_ascii_case("https://"))
        .map(|_| &url[8..])
    else {
        return false;
    };
    let (authority, path) = rest.split_once('/').map_or((rest, ""), |(a, p)| (a, p));
    let (host, port) = authority
        .split_once(':')
        .map_or((authority, None), |(h, p)| (h, Some(p)));
    if !host.eq_ignore_ascii_case(CERT_HOST) || port.is_some_and(|p| p != "443") {
        return false;
    }
    // `/echo.api/../x` must not count: the path is normalised first.
    let mut parts: Vec<&str> = Vec::new();
    for seg in path.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    let normal = format!("/{}", parts.join("/"));
    normal.starts_with(CERT_PATH) && !path.contains('?') && !path.contains('#')
}

/// Standard base64, padding optional.
pub(crate) fn base64(text: &str) -> Option<Vec<u8>> {
    let table = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    let (mut buf, mut bits) = (0u32, 0);
    for c in text
        .bytes()
        .filter(|&c| c != b'=' && !c.is_ascii_whitespace())
    {
        buf = (buf << 6) | u32::try_from(table.iter().position(|&t| t == c)?).ok()?;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(u8::try_from((buf >> bits) & 0xFF).ok()?);
        }
    }
    Some(out)
}

/// The certificates of a PEM chain, leaf first.
pub(crate) fn pem_chain(pem: &str) -> Vec<Vec<u8>> {
    const BEGIN: &str = "-----BEGIN CERTIFICATE-----";
    const END: &str = "-----END CERTIFICATE-----";
    let mut out = Vec::new();
    let mut rest = pem;
    while let Some(start) = rest.find(BEGIN) {
        let after = &rest[start + BEGIN.len()..];
        let Some(end) = after.find(END) else { break };
        if let Some(der) = base64(&after[..end]) {
            out.push(der);
        }
        rest = &after[end + END.len()..];
    }
    out
}

/// The chain proves it is Amazon's signer, and its key signed `body`.
pub(crate) fn signed_by_amazon(
    chain: &[Vec<u8>],
    body: &[u8],
    signature: &[u8],
    now: UnixTime,
) -> Result<(), String> {
    let (leaf, rest) = chain.split_first().ok_or("empty certificate chain")?;
    let leaf = CertificateDer::from(leaf.as_slice());
    let intermediates: Vec<CertificateDer<'_>> = rest
        .iter()
        .map(|c| CertificateDer::from(c.as_slice()))
        .collect();
    let cert = webpki::EndEntityCert::try_from(&leaf).map_err(|e| format!("certificate: {e}"))?;
    cert.verify_for_usage(
        webpki::ALL_VERIFICATION_ALGS,
        webpki_roots::TLS_SERVER_ROOTS,
        &intermediates,
        now,
        webpki::KeyUsage::server_auth(),
        None,
        None,
    )
    .map_err(|e| format!("certificate chain: {e}"))?;
    let name = ServerName::try_from(SIGNER).map_err(|e| e.to_string())?;
    cert.verify_is_valid_for_subject_name(&name)
        .map_err(|e| format!("certificate name: {e}"))?;
    cert.verify_signature(webpki::ring::RSA_PKCS1_2048_8192_SHA256, body, signature)
        .map_err(|_| "signature does not match the body".to_owned())
}

/// The request is fresh and for this skill.
pub(crate) fn for_this_skill(request: &Json, skill_id: &str, now_s: i64) -> Result<(), String> {
    let ts = request["request"]["timestamp"]
        .as_str()
        .and_then(|t| t.parse::<jiff::Timestamp>().ok())
        .ok_or("no timestamp")?;
    if (now_s - ts.as_second()).abs() > MAX_AGE_S {
        return Err("request too old".into());
    }
    let app = request["context"]["System"]["application"]["applicationId"]
        .as_str()
        .or_else(|| request["session"]["application"]["applicationId"].as_str())
        .ok_or("no skill id")?;
    if app != skill_id {
        return Err("another skill".into());
    }
    Ok(())
}

/// What the person asked, from the intent's free slot. Alexa keeps the
/// words in the slot only: the word that led them (« mets », « combien »)
/// is the intent's own, named `Q_<words>` (`Q_est_ce_que`), and put back.
pub(crate) fn question(request: &Json) -> Option<String> {
    let intent = &request["request"]["intent"];
    let slots = intent["slots"].as_object()?;
    let asked = slots
        .values()
        .filter_map(|s| s["value"].as_str())
        .map(str::trim)
        .find(|v| !v.is_empty())?;
    let lead = intent["name"]
        .as_str()
        .and_then(|n| n.strip_prefix("Q_"))
        .map(|w| w.replace('_', " "));
    Some(match lead {
        Some(lead) if !lead.is_empty() => format!("{lead} {asked}"),
        _ => asked.to_owned(),
    })
}

/// Words Alexa can read: no markdown, not too long.
pub(crate) fn spoken(text: &str) -> String {
    let plain: String = text
        .chars()
        .filter(|c| !matches!(c, '*' | '#' | '`' | '_'))
        .collect();
    let plain = plain.split_whitespace().collect::<Vec<_>>().join(" ");
    if plain.chars().count() <= MAX_SPOKEN {
        return plain;
    }
    // Cut at the last sentence that fits.
    let cut: String = plain.chars().take(MAX_SPOKEN).collect();
    match cut.rfind(['.', '!', '?']) {
        Some(i) if i > MAX_SPOKEN / 3 => cut[..=i].to_owned(),
        _ => format!("{cut}…"),
    }
}

/// A spoken answer that keeps the session open with these attributes.
fn speech_keeping(text: &str, attributes: &Json) -> Response {
    let out = json!({
        "version": "1.0",
        "response": {
            "outputSpeech": { "type": "PlainText", "text": text },
            "reprompt": { "outputSpeech": { "type": "PlainText", "text": text } },
            "shouldEndSession": false,
        },
        "sessionAttributes": attributes,
    });
    axum::Json(out).into_response()
}

fn speech(text: &str, end: bool, history: Option<&Json>) -> Response {
    let mut out = json!({
        "version": "1.0",
        "response": {
            "outputSpeech": { "type": "PlainText", "text": text },
            "shouldEndSession": end,
        },
    });
    if !end {
        out["response"]["reprompt"] = json!({
            "outputSpeech": { "type": "PlainText", "text": moli_i18n::tr!("serveur.alexa_skill.encore") }
        });
    }
    if let Some(h) = history {
        out["sessionAttributes"] = json!({ "moli": h });
    }
    axum::Json(out).into_response()
}

async fn chain(skill: &AlexaSkill, url: &str) -> Result<Vec<Vec<u8>>, String> {
    if let Some((at, chain)) = skill
        .chains
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(url)
        .filter(|(at, _)| at.elapsed() < CHAIN_FOR)
    {
        let _ = at;
        return Ok(chain.clone());
    }
    let path = &url[8 + CERT_HOST.len()..];
    let request = http::Request::builder().uri(path).header("host", CERT_HOST);
    let request = moli_net::empty(request).map_err(|e| e.to_string())?;
    let (status, body) = moli_net::web(
        CERT_HOST,
        443,
        true,
        request,
        Duration::from_secs(4),
        64 * 1024,
    )
    .await
    .map_err(|e| format!("{e:#}"))?;
    if !status.is_success() {
        return Err(format!("certificate chain: HTTP {}", status.as_u16()));
    }
    let chain = pem_chain(&String::from_utf8_lossy(&body));
    let mut chains = skill.chains.lock().unwrap_or_else(PoisonError::into_inner);
    if chains.len() > 8 {
        chains.clear();
    }
    chains.insert(url.to_owned(), (Instant::now(), chain.clone()));
    Ok(chain)
}

/// Proved to be Amazon's, for this skill: the request, parsed.
async fn proved(skill: &AlexaSkill, headers: &HeaderMap, body: &[u8]) -> Result<Json, String> {
    let header = |name: &str| headers.get(name).and_then(|v| v.to_str().ok());
    let url = header("signaturecertchainurl").ok_or("no certificate url")?;
    if !cert_url_ok(url) {
        return Err("certificate url is not Amazon's".into());
    }
    let signature = header("signature-256")
        .and_then(base64)
        .ok_or("no signature")?;
    let chain = chain(skill, url).await?;
    signed_by_amazon(&chain, body, &signature, UnixTime::now())?;
    let request: Json = serde_json::from_slice(body).map_err(|_| "not JSON")?;
    for_this_skill(
        &request,
        &skill.skill_id,
        jiff::Timestamp::now().as_second(),
    )?;
    Ok(request)
}

/// The conversation so far, kept by Alexa in the session.
fn history(request: &Json) -> Vec<moli_assistant::Message> {
    request["session"]["attributes"]["moli"]
        .as_array()
        .map(|turns| {
            turns
                .iter()
                .filter_map(|m| serde_json::from_value(m.clone()).ok())
                .collect()
        })
        .unwrap_or_default()
}

pub(crate) async fn skill(
    axum::extract::State(hub): axum::extract::State<moli_runtime::Hub>,
    Extension(skill): Extension<Option<AlexaSkill>>,
    Extension(Moli(moli)): Extension<Moli>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let Some(skill) = skill else {
        return StatusCode::NOT_FOUND.into_response();
    };
    tracing::info!(bytes = body.len(), "alexa skill: received");
    let request = match proved(&skill, &headers, &body).await {
        Ok(r) => r,
        Err(why) => {
            tracing::warn!(why, "alexa skill: request refused");
            return (StatusCode::BAD_REQUEST, "not a request of this skill").into_response();
        }
    };
    let Some(moli) = moli else {
        return speech(
            &moli_i18n::tr!("serveur.alexa_skill.pas_d_assistant"),
            true,
            None,
        );
    };
    let kind = request["request"]["type"].as_str().unwrap_or_default();
    let intent = request["request"]["intent"]["name"]
        .as_str()
        .unwrap_or_default();
    tracing::info!(kind, intent, "alexa skill: request");
    match (kind, intent) {
        ("LaunchRequest", _) => speech(
            &moli_i18n::tr!("serveur.alexa_skill.bonjour"),
            false,
            Some(&json!([])),
        ),
        ("IntentRequest", "AMAZON.StopIntent" | "AMAZON.CancelIntent" | "AMAZON.NoIntent") => {
            speech(&moli_i18n::tr!("serveur.alexa_skill.au_revoir"), true, None)
        }
        ("IntentRequest", "ArmIntent") => arm(&hub, &skill, &request).await,
        ("IntentRequest", "DisarmIntent") => ask_code(&hub, &skill),
        ("IntentRequest", "CodeIntent") => disarm(&hub, &skill, &request).await,
        ("IntentRequest", "AMAZON.HelpIntent") => speech(
            &moli_i18n::tr!("serveur.alexa_skill.aide"),
            false,
            Some(&json!(history(&request))),
        ),
        ("IntentRequest", _) => {
            let Some(asked) = question(&request) else {
                return speech(
                    &moli_i18n::tr!("serveur.alexa_skill.pas_compris"),
                    false,
                    None,
                );
            };
            answer(&moli, &request, asked).await
        }
        // The session ended (or a request this skill does not use).
        _ => speech("", true, None),
    }
}

/// The mode asked for (`ALARM_MODE` slot, its id once Alexa resolved it):
/// night when none is said.
pub(crate) fn mode_asked(request: &Json) -> &'static str {
    let slot = &request["request"]["intent"]["slots"]["mode"];
    let id = slot["resolutions"]["resolutionsPerAuthority"][0]["values"][0]["value"]["id"]
        .as_str()
        .or_else(|| slot["value"].as_str())
        .unwrap_or_default()
        .to_lowercase();
    match id.as_str() {
        "home" | "présence" | "presence" | "jour" | "maison" => "home",
        "away" | "absence" | "absent" | "vacances" => "away",
        _ => "night",
    }
}

/// « Arme l'alarme en nuit »: no code, as an agent the file allows
/// (`[[guard.agents_may]]`); never through the model.
async fn arm(hub: &moli_runtime::Hub, skill: &AlexaSkill, request: &Json) -> Response {
    let say = |text: String| speech(&text, true, None);
    let Some(point) = skill.disarm.as_deref() else {
        return say(moli_i18n::tr!("serveur.alexa_skill.pas_de_desarmement"));
    };
    let mode = mode_asked(request);
    let sent = hub
        .command(
            &moli_core::PointId::from(point.to_owned()),
            moli_core::Value::Text(mode.into()),
            moli_core::Origin::Assistant,
            Some(moli_i18n::tr!("serveur.alexa_skill.acteur_armement")),
        )
        .await;
    let name = moli_i18n::tr(&format!("serveur.alexa_skill.mode_{mode}"));
    say(match sent {
        Ok(()) => moli_i18n::tr!("serveur.alexa_skill.armee", mode = name),
        Err(moli_runtime::CommandError::NeedsApproval { .. }) => {
            moli_i18n::tr!("serveur.alexa_skill.armement_retenu")
        }
        Err(e) => moli_i18n::tr!("serveur.alexa_skill.armement_echoue", raison = e),
    })
}

/// « Désarme l'alarme »: the voice code is asked (never the house's).
fn ask_code(hub: &moli_runtime::Hub, skill: &AlexaSkill) -> Response {
    let say = |key: &str| speech(&moli_i18n::tr(key), true, None);
    if skill.disarm.is_none() {
        return say("serveur.alexa_skill.pas_de_desarmement");
    }
    if hub
        .secret(&moli_core::InstanceId::from(VOICE_INSTANCE), VOICE_CODE)
        .is_none()
    {
        return say("serveur.alexa_skill.pas_de_code_vocal");
    }
    if let Some(minutes) = lock_of(skill).locked(Instant::now()) {
        return speech(
            &moli_i18n::tr!("serveur.alexa_skill.code_bloque", minutes = minutes),
            true,
            None,
        );
    }
    speech_keeping(
        &moli_i18n::tr!("serveur.alexa_skill.quel_code"),
        &json!({ "disarm": true }),
    )
}

fn lock_of(skill: &AlexaSkill) -> std::sync::MutexGuard<'_, VoiceLock> {
    skill.lock.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The code said after « Quel est le code vocal ? »: checked here, never by
/// the model; right, the alarm is disarmed by the house itself.
async fn disarm(hub: &moli_runtime::Hub, skill: &AlexaSkill, request: &Json) -> Response {
    let say = |text: String| speech(&text, true, None);
    let (Some(point), true) = (
        skill.disarm.as_deref(),
        request["session"]["attributes"]["disarm"] == json!(true),
    ) else {
        return speech(
            &moli_i18n::tr!("serveur.alexa_skill.pas_compris"),
            false,
            None,
        );
    };
    let Some(kept) = hub.secret(&moli_core::InstanceId::from(VOICE_INSTANCE), VOICE_CODE) else {
        return say(moli_i18n::tr!("serveur.alexa_skill.pas_de_code_vocal"));
    };
    let now = Instant::now();
    if let Some(minutes) = lock_of(skill).locked(now) {
        return say(moli_i18n::tr!(
            "serveur.alexa_skill.code_bloque",
            minutes = minutes
        ));
    }
    let said = question(request).unwrap_or_default();
    let heard = heard_code(&said).unwrap_or_default();
    if heard.is_empty() || !same_code(&heard, &kept) {
        let locked = lock_of(skill).wrong(now);
        // Never a digit of either code: only their shapes, to tell a code
        // misheard from a code kept badly.
        tracing::warn!(
            locked = locked.is_some(),
            heard_digits = heard.len(),
            said_chars = said.chars().count(),
            kept_digits = kept.chars().filter(char::is_ascii_digit).count(),
            kept_only_digits = kept.trim().chars().all(|c| c.is_ascii_digit()),
            "alexa skill: wrong voice code"
        );
        return say(match locked {
            Some(minutes) => moli_i18n::tr!("serveur.alexa_skill.code_bloque", minutes = minutes),
            None => moli_i18n::tr!("serveur.alexa_skill.code_faux"),
        });
    }
    lock_of(skill).right();
    let sent = hub
        .command(
            &moli_core::PointId::from(point.to_owned()),
            moli_core::Value::Text("off".into()),
            moli_core::Origin::System,
            Some(moli_i18n::tr!("serveur.alexa_skill.acteur")),
        )
        .await;
    say(match sent {
        Ok(()) => moli_i18n::tr!("serveur.alexa_skill.desarmee"),
        Err(e) => moli_i18n::tr!("serveur.alexa_skill.desarmement_echoue", raison = e),
    })
}

async fn answer(moli: &moli_assistant::Assistant, request: &Json, asked: String) -> Response {
    let mut messages = history(request);
    messages.push(moli_assistant::Message {
        role: moli_assistant::Role::User,
        content: asked.clone(),
    });
    let turn = moli_assistant::Turn {
        messages: messages.clone(),
        surface: Some("bubble".into()),
        spoken: true,
        id: None,
        asker: Some("alexa".into()),
    };
    let reply = match tokio::time::timeout(ANSWER_WITHIN, moli.turn(turn)).await {
        Ok(Ok(reply)) => reply.reply,
        Ok(Err(e)) => {
            tracing::warn!(error = %e, "alexa skill: the assistant failed");
            return speech(&moli_i18n::tr!("serveur.alexa_skill.panne"), true, None);
        }
        Err(_) => {
            return speech(
                &moli_i18n::tr!("serveur.alexa_skill.trop_long"),
                false,
                None,
            );
        }
    };
    let said = spoken(&reply);
    messages.push(moli_assistant::Message {
        role: moli_assistant::Role::Assistant,
        content: said.clone(),
    });
    let keep = messages.len().saturating_sub(MAX_TURNS * 2);
    let kept = json!(messages[keep..]);
    speech(&said, false, Some(&kept))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_amazons_certificate_url() {
        assert!(cert_url_ok(
            "https://s3.amazonaws.com/echo.api/echo-api-cert.pem"
        ));
        assert!(cert_url_ok(
            "HTTPS://s3.amazonaws.com/echo.api/../echo.api/echo-api-cert.pem"
        ));
        assert!(cert_url_ok(
            "https://s3.amazonaws.com:443/echo.api/echo-api-cert.pem"
        ));
        assert!(!cert_url_ok(
            "http://s3.amazonaws.com/echo.api/echo-api-cert.pem"
        ));
        assert!(!cert_url_ok(
            "https://notamazon.com/echo.api/echo-api-cert.pem"
        ));
        assert!(!cert_url_ok(
            "https://s3.amazonaws.com/EcHo.aPi/echo-api-cert.pem"
        ));
        assert!(!cert_url_ok(
            "https://s3.amazonaws.com/invalid.path/echo-api-cert.pem"
        ));
        assert!(!cert_url_ok(
            "https://s3.amazonaws.com:563/echo.api/echo-api-cert.pem"
        ));
        assert!(!cert_url_ok("https://s3.amazonaws.com/echo.api/../x.pem"));
        assert!(!cert_url_ok(
            "https://s3.amazonaws.com.evil.com/echo.api/x.pem"
        ));
    }

    #[test]
    fn a_pem_chain_and_base64() {
        assert_eq!(base64("TW9saQ==").unwrap(), b"Moli");
        assert!(base64("é").is_none());
        let pem = "-----BEGIN CERTIFICATE-----\nTW9s\naQ==\n-----END CERTIFICATE-----\n-----BEGIN CERTIFICATE-----\nQQ==\n-----END CERTIFICATE-----\n";
        assert_eq!(pem_chain(pem), vec![b"Moli".to_vec(), b"A".to_vec()]);
    }

    #[test]
    fn a_garbage_chain_or_signature_is_refused() {
        let now = UnixTime::now();
        assert!(signed_by_amazon(&[], b"{}", b"x", now).is_err());
        assert!(signed_by_amazon(&[b"not a cert".to_vec()], b"{}", b"x", now).is_err());
    }

    #[test]
    fn fresh_and_for_this_skill_only() {
        let now = jiff::Timestamp::now();
        let req = |ts: jiff::Timestamp, app: &str| {
            json!({
                "request": { "timestamp": ts.to_string() },
                "context": { "System": { "application": { "applicationId": app } } }
            })
        };
        let s = now.as_second();
        assert!(
            for_this_skill(&req(now, "amzn1.ask.skill.moli"), "amzn1.ask.skill.moli", s).is_ok()
        );
        assert!(
            for_this_skill(
                &req(now, "amzn1.ask.skill.other"),
                "amzn1.ask.skill.moli",
                s
            )
            .is_err()
        );
        let old = now - jiff::SignedDuration::from_secs(200);
        assert!(
            for_this_skill(&req(old, "amzn1.ask.skill.moli"), "amzn1.ask.skill.moli", s).is_err()
        );
    }

    #[test]
    fn the_voice_code_heard_compared_and_locked() {
        assert_eq!(heard_code("12 34").as_deref(), Some("1234"));
        assert_eq!(heard_code("123456").as_deref(), Some("123456"));
        assert!(heard_code("123").is_none(), "too short");
        assert!(heard_code("123456789").is_none(), "too long");
        assert!(heard_code("douze").is_none());
        assert!(same_code("1234", " 1234\n"));
        assert!(same_code("1234", "12 34"));
        assert!(!same_code("", "  "), "no code kept, nothing matches");
        assert!(!same_code("1235", "1234"));
        let mut lock = VoiceLock::default();
        let t0 = Instant::now();
        assert_eq!(lock.wrong(t0), None);
        assert_eq!(lock.wrong(t0), None);
        assert_eq!(lock.wrong(t0), Some(30), "third wrong code: 30 min");
        assert_eq!(lock.locked(t0), Some(30));
        assert_eq!(lock.locked(t0 + Duration::from_secs(31 * 60)), None);
        for _ in 0..2 {
            lock.wrong(t0);
        }
        assert_eq!(lock.wrong(t0), Some(60), "then twice as long");
        lock.right();
        assert_eq!(lock.locked(t0), None);
    }

    #[test]
    fn the_mode_asked_to_arm() {
        let with = |slot: Json| json!({ "request": { "intent": { "name": "ArmIntent", "slots": { "mode": slot } } } });
        let resolved = json!({ "name": "mode", "value": "la nuit", "resolutions": { "resolutionsPerAuthority": [
            { "values": [ { "value": { "name": "nuit", "id": "night" } } ] } ] } });
        assert_eq!(mode_asked(&with(resolved)), "night");
        assert_eq!(
            mode_asked(&with(json!({ "name": "mode", "value": "présence" }))),
            "home"
        );
        assert_eq!(
            mode_asked(&with(json!({ "name": "mode", "value": "absence" }))),
            "away"
        );
        assert_eq!(mode_asked(&json!({})), "night", "none said: night");
    }

    #[test]
    fn the_question_and_what_alexa_reads() {
        let r = json!({ "request": { "intent": { "name": "Question", "slots": {
            "question": { "name": "question", "value": " la porte est fermée " } } } } });
        assert_eq!(question(&r).as_deref(), Some("la porte est fermée"));
        let r = json!({ "request": { "intent": { "name": "Q_est_ce_que", "slots": {
            "question": { "name": "question", "value": "la porte est fermée" } } } } });
        assert_eq!(
            question(&r).as_deref(),
            Some("est ce que la porte est fermée")
        );
        let r = json!({ "request": { "intent": { "name": "Q_mets", "slots": {
            "question": { "name": "question" } } } } });
        assert!(question(&r).is_none(), "nothing heard");
        assert_eq!(
            spoken("**Oui**, la porte\n est _fermée_."),
            "Oui, la porte est fermée."
        );
        let long = "Une phrase. ".repeat(100);
        let s = spoken(&long);
        assert!(s.chars().count() <= MAX_SPOKEN && s.ends_with('.'));
    }
}
