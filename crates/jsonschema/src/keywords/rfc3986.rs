//! RFC 3986 syntax for the `uri` and `uri-reference` formats, checked without building a URI.

// unreserved / sub-delims: everything a `reg-name` holds besides percent-encodings.
const REG_NAME: u8 = 1;
const COLON: u8 = 2;
const AT: u8 = 4;
const SLASH: u8 = 8;
const QUESTION: u8 = 16;
const PERCENT: u8 = 32;

const PCHAR: u8 = REG_NAME | COLON | AT;
const PATH: u8 = PCHAR | SLASH;
// Query and fragment share one grammar.
const QUERY: u8 = PATH | QUESTION;
const USERINFO: u8 = REG_NAME | COLON;
const AUTHORITY: u8 = USERINFO | AT;

const CLASSES: [u8; 256] = {
    let mut table = [0; 256];
    let mut b: u8 = 0;
    loop {
        table[b as usize] = if b.is_ascii_alphanumeric()
            || matches!(
                b,
                b'-' | b'.'
                    | b'_'
                    | b'~'
                    | b'!'
                    | b'$'
                    | b'&'
                    | b'\''
                    | b'('
                    | b')'
                    | b'*'
                    | b'+'
                    | b','
                    | b';'
                    | b'='
            ) {
            REG_NAME
        } else {
            match b {
                b':' => COLON,
                b'@' => AT,
                b'/' => SLASH,
                b'?' => QUESTION,
                b'%' => PERCENT,
                _ => 0,
            }
        };
        if b == u8::MAX {
            break;
        }
        b += 1;
    }
    table
};

/// Whether `text` is a URI (`absolute` set) or a URI reference. `None` for a bracketed
/// IP literal in the authority, which the full parser checks.
pub(crate) fn check(text: &str, absolute: bool) -> Option<bool> {
    let bytes = text.as_bytes();
    let scheme = scheme_length(bytes);
    if absolute && scheme.is_none() {
        return Some(false);
    }
    let mut at = scheme.map_or(0, |length| length + 1);
    if bytes[at..].starts_with(b"//") {
        let Some(end) = scan(bytes, at + 2, AUTHORITY) else {
            return Some(false);
        };
        if matches!(bytes.get(end), Some(b'[' | b']')) {
            return None;
        }
        if !authority(&bytes[at + 2..end]) {
            return Some(false);
        }
        at = end;
    } else if scheme.is_none() && first_segment_has_colon(&bytes[at..]) {
        // `a:b` with an invalid scheme would read as one.
        return Some(false);
    }
    let Some(end) = scan(bytes, at, PATH) else {
        return Some(false);
    };
    at = end;
    if bytes.get(at) == Some(&b'?') {
        let Some(end) = scan(bytes, at + 1, QUERY) else {
            return Some(false);
        };
        at = end;
    }
    if bytes.get(at) == Some(&b'#') {
        let Some(end) = scan(bytes, at + 1, QUERY) else {
            return Some(false);
        };
        at = end;
    }
    Some(at == bytes.len())
}

/// The length of a leading `scheme` followed by `:`.
fn scheme_length(bytes: &[u8]) -> Option<usize> {
    if !bytes.first()?.is_ascii_alphabetic() {
        return None;
    }
    let length = bytes
        .iter()
        .position(|&b| !(b.is_ascii_alphanumeric() || matches!(b, b'+' | b'-' | b'.')))?;
    (bytes[length] == b':').then_some(length)
}

fn first_segment_has_colon(bytes: &[u8]) -> bool {
    bytes
        .iter()
        .find(|&&b| matches!(b, b':' | b'/' | b'?' | b'#'))
        == Some(&b':')
}

/// The end of the run of bytes in `class`, percent-encodings included; `None` on a broken one.
#[inline]
fn scan(bytes: &[u8], mut at: usize, class: u8) -> Option<usize> {
    while let Some(&b) = bytes.get(at) {
        let found = CLASSES[b as usize];
        if found & class != 0 {
            at += 1;
        } else if found == PERCENT {
            if !bytes.get(at + 1..at + 3)?.iter().all(u8::is_ascii_hexdigit) {
                return None;
            }
            at += 3;
        } else {
            break;
        }
    }
    Some(at)
}

/// `[ userinfo "@" ] host [ ":" port ]`, from bytes the authority scan already limited to
/// those characters with sound percent-encodings, which any userinfo then satisfies.
fn authority(bytes: &[u8]) -> bool {
    let host_port = bytes
        .iter()
        .position(|&b| b == b'@')
        .map_or(bytes, |at| &bytes[at + 1..]);
    let host_end = host_port
        .iter()
        .position(|&b| CLASSES[b as usize] & (COLON | AT) != 0)
        .unwrap_or(host_port.len());
    match host_port.get(host_end) {
        None => true,
        Some(b':') => host_port[host_end + 1..].iter().all(u8::is_ascii_digit),
        Some(_) => false,
    }
}
