//! Signing in to a Navimow account without ever seeing the password: Moli
//! makes the address of Navimow's own sign-in page (OAuth 2, authorization
//! code), the person signs in there and pastes the address the page ends on
//! (`https://my.home-assistant.io/redirect/oauth?code=…&state=…`, or the
//! Home Assistant address that page may forward to). Its code gives an access
//! token (about an hour) and, when Navimow gives one, a refresh token; both
//! are filed in the vault.
//!
//! Navimow opens its smart-home API to its official Home Assistant
//! integration only: Moli signs in as that integration (client id, the
//! client secret Segway publishes with it, its redirect). Interface facts
//! from segwaynavimow/NavimowHA (`const.py`, `auth.py`); no code of theirs.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::Context as _;
use http::header::{ACCEPT, CONTENT_TYPE, HOST, USER_AGENT};
use ring::rand::{SecureRandom as _, SystemRandom};
use serde::{Deserialize, Serialize};
use serde_json::Value as Json;

use crate::encode;

pub(crate) const LOGIN_HOST: &str = "navimow-h5-fra.willand.com";
pub(crate) const API_HOST: &str = "navimow-fra.ninebot.com";
const TOKEN_PATH: &str = "/openapi/oauth/getAccessToken";
const CLIENT_ID: &str = "homeassistant";
/// Published by Segway with its Home Assistant integration (a public client).
const CLIENT_SECRET: &str = "57056e15-722e-42be-bbaa-b0cbfb208a52";
pub(crate) const REDIRECT_URI: &str = "https://my.home-assistant.io/redirect/oauth";
pub(crate) const AGENT: &str = "github.com/rudy-io/moli-os";
const LIMIT: Duration = Duration::from_secs(20);
const MAX_BODY: usize = 64 * 1024;
/// A token is renewed this long before it expires.
const MARGIN: u64 = 120;

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// A sign-in in progress: the state the page must bring back.
#[derive(Clone)]
pub(crate) struct SignIn {
    state: String,
    /// Navimow's sign-in page, for the person.
    pub url: String,
}

impl std::fmt::Debug for SignIn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SignIn")
            .field("url", &self.url)
            .finish_non_exhaustive()
    }
}

impl SignIn {
    pub fn new() -> anyhow::Result<Self> {
        let mut bytes = [0u8; 16];
        SystemRandom::new()
            .fill(&mut bytes)
            .map_err(|_| anyhow::anyhow!("no randomness"))?;
        let state = encode::hex(&bytes);
        let query = encode::form(&[
            ("channel", "homeassistant"),
            ("response_type", "code"),
            ("client_id", CLIENT_ID),
            ("redirect_uri", REDIRECT_URI),
            ("state", &state),
        ]);
        Ok(Self {
            state,
            url: format!("https://{LOGIN_HOST}/smartHome/login?{query}"),
        })
    }

    /// The code in the address the person pasted, if it is the end of this
    /// sign-in: Home Assistant's redirect page, or the Home Assistant
    /// address it forwards to, with this sign-in's state.
    pub fn code(&self, pasted: &str) -> Result<String, Pasted> {
        let pasted = pasted.trim();
        let ends_here = pasted.starts_with("https://my.home-assistant.io/")
            || (pasted.starts_with("http") && pasted.contains("/auth/external/callback"));
        if !ends_here {
            return Err(Pasted::NotTheEnd);
        }
        // Without a Home Assistant of its own to send to, the redirect page
        // moves to its settings page (`/redirect/_change/`) and carries the
        // answer inside its `redirect` parameter: `oauth/?code=…&state=…`.
        let inner = encode::query_param(pasted, "redirect").filter(|r| r.contains("state="));
        let answer = inner.as_deref().unwrap_or(pasted);
        if encode::query_param(answer, "state").as_deref() != Some(self.state.as_str()) {
            return Err(Pasted::Stale);
        }
        encode::query_param(answer, "code")
            .filter(|c| !c.is_empty())
            .ok_or(Pasted::NoCode)
    }

    pub async fn session(&self, code: &str) -> Result<Session, Failure> {
        token_request(&[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", REDIRECT_URI),
            ("client_id", CLIENT_ID),
            ("client_secret", CLIENT_SECRET),
        ])
        .await
    }
}

/// What is wrong with a pasted address.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Pasted {
    /// Not the address the sign-in ends on.
    NotTheEnd,
    /// The end of an older sign-in (the page was opened before Moli restarted).
    Stale,
    NoCode,
}

/// The tokens, as filed in the vault (one JSON secret).
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Session {
    pub access: String,
    #[serde(default)]
    pub refresh: Option<String>,
    /// When the access token expires (Unix seconds).
    pub expires: u64,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session")
            .field("refresh", &self.refresh.is_some())
            .field("expires", &self.expires)
            .finish_non_exhaustive()
    }
}

impl Session {
    pub fn stale(&self) -> bool {
        now() + MARGIN >= self.expires
    }

    /// A new access token from the refresh token; `Refused` without one.
    pub async fn renewed(&self) -> Result<Self, Failure> {
        let Some(refresh) = self.refresh.as_deref() else {
            return Err(Failure::Refused);
        };
        let mut next = token_request(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh),
            ("client_id", CLIENT_ID),
            ("client_secret", CLIENT_SECRET),
        ])
        .await?;
        // Navimow may not send a new refresh token: the old one stays.
        if next.refresh.is_none() {
            next.refresh.clone_from(&self.refresh);
        }
        Ok(next)
    }
}

/// Why a request failed.
#[derive(Debug)]
pub(crate) enum Failure {
    /// Navimow no longer takes the session: a person must sign in again.
    Refused,
    Other(anyhow::Error),
}

impl From<anyhow::Error> for Failure {
    fn from(e: anyhow::Error) -> Self {
        Self::Other(e)
    }
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Refused => f.write_str("Navimow refused the session"),
            Self::Other(e) => write!(f, "{e:#}"),
        }
    }
}

async fn token_request(pairs: &[(&str, &str)]) -> Result<Session, Failure> {
    let request = http::Request::builder()
        .method(http::Method::POST)
        .uri(TOKEN_PATH)
        .header(HOST, API_HOST)
        .header(USER_AGENT, AGENT)
        .header(ACCEPT, "application/json")
        .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(http_body_util::Full::new(moli_net::Body::from(
            encode::form(pairs),
        )))
        .context("token request")?;
    // The answer may echo what was sent: never in an error.
    let (status, body) = moli_net::web(API_HOST, 443, true, request, LIMIT, MAX_BODY)
        .await
        .map_err(|_| anyhow::anyhow!("{API_HOST} unreachable"))?;
    session_from(status.as_u16(), &body, now())
}

/// The token endpoint's answer: a session, or why not. Plain OAuth (what
/// Home Assistant reads), or wrapped in Navimow's `{code, data}`.
pub(crate) fn session_from(status: u16, body: &[u8], now: u64) -> Result<Session, Failure> {
    let json: Json = serde_json::from_slice(body).unwrap_or(Json::Null);
    let inner = if json["data"].is_object() && json["access_token"].is_null() {
        &json["data"]
    } else {
        &json
    };
    if matches!(status, 401 | 403) || json["error"] == "invalid_grant" {
        return Err(Failure::Refused);
    }
    if !(200..300).contains(&status) {
        return Err(Failure::Other(anyhow::anyhow!(
            "token endpoint: HTTP {status}"
        )));
    }
    let text = |k: &str| {
        inner[k]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
    };
    let Some(access) = text("access_token") else {
        // Navimow's own envelope says why (a code other than 1).
        if json["code"].as_i64().is_some_and(|c| c != 1) {
            return Err(Failure::Refused);
        }
        return Err(Failure::Other(anyhow::anyhow!(
            "token endpoint: no token in the answer"
        )));
    };
    let lifetime = inner["expires_in"]
        .as_u64()
        .or_else(|| inner["expires_in"].as_str().and_then(|s| s.parse().ok()))
        .unwrap_or(3600)
        .clamp(300, 30 * 86_400);
    Ok(Session {
        access,
        refresh: text("refresh_token"),
        expires: now + lifetime,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sign_in_page_is_navimows_for_home_assistant() {
        let s = SignIn::new().unwrap();
        assert!(
            s.url
                .starts_with("https://navimow-h5-fra.willand.com/smartHome/login?")
        );
        for part in [
            "channel=homeassistant",
            "response_type=code",
            "client_id=homeassistant",
            "redirect_uri=https%3A%2F%2Fmy.home-assistant.io%2Fredirect%2Foauth",
        ] {
            assert!(s.url.contains(part), "{part}");
        }
        assert_eq!(encode::query_param(&s.url, "state").unwrap(), s.state);
    }

    #[test]
    fn only_the_end_of_this_sign_in_gives_a_code() {
        let s = SignIn::new().unwrap();
        let end = format!("{REDIRECT_URI}?code=abc123&state={}", s.state);
        assert_eq!(s.code(&format!("  {end}\n")), Ok("abc123".into()));
        // Forwarded to the house's Home Assistant: the same code.
        let forwarded = format!(
            "http://homeassistant.local:8123/auth/external/callback?code=abc123&state={}",
            s.state
        );
        assert_eq!(s.code(&forwarded), Ok("abc123".into()));
        // Moved to the redirect page's settings: the answer inside `redirect`.
        let moved = format!(
            "https://my.home-assistant.io/redirect/_change/?redirect=oauth%2F%3Fcode%3Dq7-x%26state%3D{}",
            s.state
        );
        assert_eq!(s.code(&moved), Ok("q7-x".into()));
        let moved_other = "https://my.home-assistant.io/redirect/_change/?redirect=oauth%2F%3Fcode%3Dq7%26state%3Dother";
        assert_eq!(s.code(moved_other), Err(Pasted::Stale));
        assert_eq!(
            s.code("https://navimow-h5-fra.willand.com/smartHome/login?x=1"),
            Err(Pasted::NotTheEnd)
        );
        assert_eq!(
            s.code(&format!("{REDIRECT_URI}?code=abc&state=other")),
            Err(Pasted::Stale)
        );
        assert_eq!(
            s.code(&format!("{REDIRECT_URI}?state={}", s.state)),
            Err(Pasted::NoCode)
        );
    }

    #[test]
    fn token_answers_are_read_plain_or_wrapped() {
        let plain = br#"{"access_token":"a","refresh_token":"r","expires_in":3600}"#;
        let s = session_from(200, plain, 1000).unwrap();
        assert_eq!(
            (s.access.as_str(), s.refresh.as_deref(), s.expires),
            ("a", Some("r"), 4600)
        );
        let wrapped = br#"{"code":1,"desc":"ok","data":{"access_token":"b","expires_in":"7200"}}"#;
        let s = session_from(200, wrapped, 0).unwrap();
        assert_eq!((s.access.as_str(), s.refresh, s.expires), ("b", None, 7200));
        assert!(matches!(session_from(401, b"", 0), Err(Failure::Refused)));
        assert!(matches!(
            session_from(400, br#"{"error":"invalid_grant"}"#, 0),
            Err(Failure::Refused)
        ));
        assert!(matches!(
            session_from(200, br#"{"code":4001,"desc":"expired"}"#, 0),
            Err(Failure::Refused)
        ));
        assert!(matches!(session_from(500, b"", 0), Err(Failure::Other(_))));
        assert_eq!(
            format!("{:?}", session_from(200, plain, 0).unwrap()),
            "Session { refresh: true, expires: 3600, .. }"
        );
    }

    #[test]
    fn a_session_is_renewed_before_it_expires() {
        let s = Session {
            access: "a".into(),
            refresh: None,
            expires: now() + 60,
        };
        assert!(s.stale());
        let s = Session {
            access: "a".into(),
            refresh: None,
            expires: now() + 3600,
        };
        assert!(!s.stale());
    }
}
