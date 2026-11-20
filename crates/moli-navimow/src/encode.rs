//! The few encodings the sign-in needs: hexadecimal, percent-encoding and
//! query parameters.

/// Lowercase hexadecimal.
pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut s, b| {
            s.push(char::from(b"0123456789abcdef"[usize::from(b >> 4)]));
            s.push(char::from(b"0123456789abcdef"[usize::from(b & 15)]));
            s
        })
}

/// Percent-encoding of everything but the unreserved characters.
pub(crate) fn percent(text: &str) -> String {
    text.bytes()
        .fold(String::with_capacity(text.len()), |mut s, b| {
            if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
                s.push(char::from(b));
            } else {
                s.push('%');
                s.push(char::from(b"0123456789ABCDEF"[usize::from(b >> 4)]));
                s.push(char::from(b"0123456789ABCDEF"[usize::from(b & 15)]));
            }
            s
        })
}

/// `a=1&b=2`, each name and value percent-encoded.
pub(crate) fn form(pairs: &[(&str, &str)]) -> String {
    pairs
        .iter()
        .map(|(k, v)| format!("{}={}", percent(k), percent(v)))
        .collect::<Vec<_>>()
        .join("&")
}

fn decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("");
                if let Ok(b) = u8::from_str_radix(hex, 16) {
                    out.push(b);
                    i += 3;
                    continue;
                }
                out.push(b'%');
            }
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The value of `name` in the query of `url` (after `?`, before `#`).
pub(crate) fn query_param(url: &str, name: &str) -> Option<String> {
    let query = url.split_once('?')?.1;
    let query = query.split('#').next().unwrap_or(query);
    query
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .find(|(k, _)| decode(k) == name)
        .map(|(_, v)| decode(v))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queries_are_read_and_written() {
        assert_eq!(form(&[("a b", "c/d")]), "a%20b=c%2Fd");
        let url = "https://my.home-assistant.io/redirect/oauth?code=ab%2Bc&state=xyz#frag";
        assert_eq!(query_param(url, "code").as_deref(), Some("ab+c"));
        assert_eq!(query_param(url, "state").as_deref(), Some("xyz"));
        assert_eq!(query_param(url, "nope"), None);
        assert_eq!(hex(&[0, 255]), "00ff");
    }
}
