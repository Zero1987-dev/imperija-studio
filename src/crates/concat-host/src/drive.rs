// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 TikTok Imperija
//! Google Drive: an export that is already uploaded when it finishes.
//!
//! A finished cut is nearly always going somewhere - a phone, a client, a
//! second editor - and the walk from "export done" to "file shared" is a
//! folder, a browser, a drag and a wait. Connected once, Drive turns that
//! into nothing at all: the export ends and the link is there.
//!
//! **How the connection is made.** The loopback flow, which is what Google
//! prescribes for a program on a person's own machine: the app opens their
//! browser at Google's consent page, listens on a port of its own for the
//! redirect, and trades the code it receives for tokens. No password ever
//! reaches this program, and the browser is the real one, with its
//! password manager and its two-factor prompt - not a window we drew.
//!
//! **PKCE.** The code that comes back on the loopback is a secret, and any
//! other program on the machine could have been listening on that port.
//! So the request carries the hash of a random string, and the exchange
//! carries the string: a code stolen without it is worthless. Required by
//! Google for this flow, and the reason it is safe on a shared machine.
//!
//! **What it may touch.** `drive.file` - the files this program itself
//! created, and nothing else. It cannot read a person's existing Drive,
//! and there is nothing in the flow that asks them to allow it to. The
//! consent screen says so in those words, which is the point.
//!
//! The refresh token is what is kept; the access token lives an hour and
//! is fetched again as needed.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

/// What the program may do on a connected account.
///
/// Only what it made itself. The wider scopes - reading a Drive, listing
/// its folders - are what Google calls sensitive and would have to be
/// reviewed before anyone outside a test list could use them at all.
pub const SCOPE: &str = "https://www.googleapis.com/auth/drive.file";

const AUTH: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const TOKEN: &str = "https://oauth2.googleapis.com/token";
const UPLOAD: &str = "https://www.googleapis.com/upload/drive/v3/files";

/// The app's own identity with Google, as the publisher registered it.
///
/// Not a secret in the usual sense: a program installed on a person's
/// machine cannot keep one, which Google says plainly, and the loopback
/// flow is built on that being true. It identifies the app, and PKCE is
/// what actually protects the exchange.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Client {
    /// The OAuth client id from the Google Cloud console.
    pub id: String,
    /// Its secret. Empty is allowed: a client registered as a Desktop app
    /// without one works the same way.
    pub secret: String,
}

impl Client {
    /// Whether an id has been set at all.
    pub fn is_set(&self) -> bool {
        !self.id.trim().is_empty()
    }
}

/// How far a connection or an upload has got.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Progress {
    /// The browser is open and the person is deciding.
    Waiting,
    /// Bytes sent, of the file's size.
    Sending {
        /// Bytes sent so far.
        sent: u64,
        /// The file's size.
        total: u64,
    },
}

/// What is kept between runs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Tokens {
    /// Traded for access tokens, indefinitely, until the person revokes it.
    pub refresh: String,
    /// The current access token, good for about an hour.
    pub access: String,
    /// When that one stops working, in seconds since the epoch.
    pub expires: u64,
}

/// The random string PKCE is built on, and the challenge sent with the
/// request.
///
/// Both derived here so a test can check the pair without a network: the
/// challenge must be the URL-safe base64 of the verifier's SHA-256, with
/// no padding, which is exactly what Google checks on the exchange.
pub fn pkce(verifier: &str) -> String {
    use sha2::{Digest, Sha256};
    base64url(&Sha256::digest(verifier.as_bytes()))
}

/// URL-safe base64 without padding, which is the only form OAuth uses.
pub fn base64url(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for i in 0..chunk.len() + 1 {
            out.push(ALPHABET[((n >> (18 - i * 6)) & 0x3f) as usize] as char);
        }
    }
    out
}

/// Percent-encoding for a query value: everything but the unreserved set.
pub fn escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(*byte as char)
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

/// The consent page's address.
pub fn auth_url(client: &Client, redirect: &str, challenge: &str, state: &str) -> String {
    format!(
        "{AUTH}?client_id={}&redirect_uri={}&response_type=code&scope={}\
         &code_challenge={}&code_challenge_method=S256&state={}&access_type=offline&prompt=consent",
        escape(&client.id),
        escape(redirect),
        escape(SCOPE),
        escape(challenge),
        escape(state),
    )
}

/// The `code` and `state` out of the browser's redirect, given its request
/// line - `GET /?state=...&code=... HTTP/1.1`.
///
/// None for anything else, a refusal included: Google sends `error=` there
/// when the person says no, and that is not a code.
pub fn code_from(request_line: &str) -> Option<(String, String)> {
    let path = request_line.split_whitespace().nth(1)?;
    let query = path.split_once('?')?.1;
    let mut code = None;
    let mut state = None;
    for pair in query.split('&') {
        match pair.split_once('=') {
            Some(("code", value)) => code = Some(unescape(value)),
            Some(("state", value)) => state = Some(unescape(value)),
            _ => {}
        }
    }
    Some((code?, state?))
}

/// Undoes percent-encoding. Unknown escapes are left as they came.
pub fn unescape(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(byte) = u8::from_str_radix(&value[i + 1..i + 3], 16) {
                out.push(byte);
                i += 3;
                continue;
            }
        }
        out.push(if bytes[i] == b'+' { b' ' } else { bytes[i] });
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Seconds since the epoch.
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Random bytes, from the operating system.
fn random(count: usize) -> Vec<u8> {
    let mut bytes = vec![0u8; count];
    if let Ok(mut file) = std::fs::File::open("/dev/urandom")
        && file.read_exact(&mut bytes).is_ok()
    {
        return bytes;
    }
    // Windows, and any machine without it: the clock and the address of a
    // fresh allocation, hashed. Weaker, and only ever used for a value
    // that lives for the seconds a consent screen is open.
    use sha2::{Digest, Sha256};
    let seed = format!("{:?}-{:p}", SystemTime::now(), &bytes);
    let mut out = Vec::new();
    let mut digest = Sha256::digest(seed.as_bytes()).to_vec();
    while out.len() < count {
        out.extend_from_slice(&digest);
        digest = Sha256::digest(&digest).to_vec();
    }
    out.truncate(count);
    out
}

/// The connection to Drive: what is registered, what is stored, and the
/// two things a person asks for - connect, and upload.
pub struct Drive {
    /// The app's data directory, where the tokens live.
    data: PathBuf,
    /// Loaded once and kept, so an upload does not read the disk.
    tokens: Mutex<Option<Tokens>>,
}

impl Drive {
    /// A connection that reads what an earlier run stored.
    pub fn new(data: &Path) -> Drive {
        Drive {
            data: data.to_path_buf(),
            tokens: Mutex::new(None),
        }
    }

    /// Where the refresh token is kept.
    fn token_file(&self) -> PathBuf {
        self.data.join("drive-token.json")
    }

    /// Where the publisher's client is kept.
    fn client_file(&self) -> PathBuf {
        self.data.join("drive-client.json")
    }

    /// The registered client, from the file the publisher ships or the
    /// person fills in. An unset one is not an error: it is what "not set
    /// up yet" looks like, and the settings page says so.
    pub fn client(&self) -> Client {
        let text = std::fs::read_to_string(self.client_file()).unwrap_or_default();
        let json: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
        Client {
            id: json["client_id"].as_str().unwrap_or_default().to_owned(),
            secret: json["client_secret"]
                .as_str()
                .unwrap_or_default()
                .to_owned(),
        }
    }

    /// Records the client. Written with the tokens' own permissions: it is
    /// not a password, but it is not for sharing either.
    pub fn set_client(&self, client: &Client) -> Result<(), String> {
        let json = serde_json::json!({
            "client_id": client.id.trim(),
            "client_secret": client.secret.trim(),
        });
        write_private(&self.client_file(), &json.to_string())
    }

    /// Whether an account is connected.
    pub fn is_connected(&self) -> bool {
        self.stored()
            .map(|t| !t.refresh.is_empty())
            .unwrap_or(false)
    }

    /// The stored tokens, read once and remembered.
    fn stored(&self) -> Option<Tokens> {
        let mut slot = self.tokens.lock().ok()?;
        if slot.is_none() {
            let text = std::fs::read_to_string(self.token_file()).ok()?;
            let json: serde_json::Value = serde_json::from_str(&text).ok()?;
            *slot = Some(Tokens {
                refresh: json["refresh_token"].as_str()?.to_owned(),
                access: json["access_token"].as_str().unwrap_or_default().to_owned(),
                expires: json["expires"].as_u64().unwrap_or(0),
            });
        }
        slot.clone()
    }

    /// Keeps tokens for the next run.
    fn keep(&self, tokens: &Tokens) -> Result<(), String> {
        let json = serde_json::json!({
            "refresh_token": tokens.refresh,
            "access_token": tokens.access,
            "expires": tokens.expires,
        });
        write_private(&self.token_file(), &json.to_string())?;
        if let Ok(mut slot) = self.tokens.lock() {
            *slot = Some(tokens.clone());
        }
        Ok(())
    }

    /// Forgets the account. The token is not revoked at Google - that is
    /// the person's own page to visit - but nothing of it stays here.
    pub fn disconnect(&self) {
        let _ = std::fs::remove_file(self.token_file());
        if let Ok(mut slot) = self.tokens.lock() {
            *slot = None;
        }
    }

    /// Runs the consent flow. `open` is handed the address to put in front
    /// of the person; the call then waits for their browser to come back.
    ///
    /// The listener is bound before the browser is opened, so the redirect
    /// can never arrive at a port nothing is holding.
    pub fn connect(
        &self,
        open: &mut dyn FnMut(&str),
        progress: &mut dyn FnMut(Progress),
    ) -> Result<(), String> {
        let client = self.client();
        if !client.is_set() {
            return Err("Drive is not set up on this build".to_owned());
        }
        let listener = TcpListener::bind("127.0.0.1:0")
            .map_err(|error| format!("could not listen for the answer: {error}"))?;
        let port = listener
            .local_addr()
            .map_err(|error| format!("could not read the port: {error}"))?
            .port();
        let redirect = format!("http://127.0.0.1:{port}");
        let verifier = base64url(&random(48));
        let state = base64url(&random(16));

        open(&auth_url(&client, &redirect, &pkce(&verifier), &state));
        progress(Progress::Waiting);

        let code = wait_for_code(&listener, &state)?;
        let answer = post_form(
            TOKEN,
            &[
                ("client_id", &client.id),
                ("client_secret", &client.secret),
                ("code", &code),
                ("code_verifier", &verifier),
                ("grant_type", "authorization_code"),
                ("redirect_uri", &redirect),
            ],
        )?;
        let refresh = answer["refresh_token"]
            .as_str()
            .ok_or("Google did not send a lasting token; try again and allow access")?;
        self.keep(&Tokens {
            refresh: refresh.to_owned(),
            access: answer["access_token"]
                .as_str()
                .unwrap_or_default()
                .to_owned(),
            expires: now() + answer["expires_in"].as_u64().unwrap_or(3600),
        })
    }

    /// A working access token, fetched again when the held one has run out.
    fn access(&self) -> Result<String, String> {
        let tokens = self.stored().ok_or("no account is connected")?;
        // A minute of slack: a token that expires mid-upload is a failure
        // three hundred megabytes in.
        if !tokens.access.is_empty() && tokens.expires > now() + 60 {
            return Ok(tokens.access);
        }
        let client = self.client();
        let answer = post_form(
            TOKEN,
            &[
                ("client_id", &client.id),
                ("client_secret", &client.secret),
                ("refresh_token", &tokens.refresh),
                ("grant_type", "refresh_token"),
            ],
        )?;
        let access = answer["access_token"]
            .as_str()
            .ok_or("the connection to Drive has expired; connect again")?
            .to_owned();
        self.keep(&Tokens {
            refresh: tokens.refresh,
            access: access.clone(),
            expires: now() + answer["expires_in"].as_u64().unwrap_or(3600),
        })?;
        Ok(access)
    }

    /// Uploads `file` and answers with the address to open it at.
    ///
    /// Resumable rather than simple: Google's simple upload is capped at
    /// five megabytes, and an export is not.
    pub fn upload(
        &self,
        file: &Path,
        progress: &mut dyn FnMut(Progress),
    ) -> Result<String, String> {
        let access = self.access()?;
        let name = file
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "video.mp4".to_owned());
        let total = std::fs::metadata(file)
            .map_err(|error| format!("could not read {}: {error}", file.display()))?
            .len();

        let session = ureq::post(&format!("{UPLOAD}?uploadType=resumable"))
            .set("Authorization", &format!("Bearer {access}"))
            .set("Content-Type", "application/json; charset=UTF-8")
            .send_string(&serde_json::json!({ "name": name }).to_string())
            .map_err(describe)?
            .header("location")
            .ok_or("Drive did not offer a place to upload to")?
            .to_owned();

        // In pieces, so the bar moves and a stall is visible. A multiple of
        // 256 KiB is what the API requires of every piece but the last.
        const PIECE: u64 = 8 * 256 * 1024;
        let mut handle = std::fs::File::open(file)
            .map_err(|error| format!("could not open {}: {error}", file.display()))?;
        let mut sent = 0u64;
        let mut answer = None;
        while sent < total {
            let size = PIECE.min(total - sent);
            let mut piece = vec![0u8; size as usize];
            handle
                .read_exact(&mut piece)
                .map_err(|error| format!("could not read {}: {error}", file.display()))?;
            let range = format!("bytes {}-{}/{}", sent, sent + size - 1, total);
            let reply = ureq::put(&session)
                .set("Content-Range", &range)
                .send_bytes(&piece);
            match reply {
                // 308: that piece landed, send the next.
                Err(ureq::Error::Status(308, _)) => {}
                Ok(done) => answer = Some(read_json(done)),
                Err(error) => return Err(describe(error)),
            }
            sent += size;
            progress(Progress::Sending { sent, total });
        }

        let id = answer
            .and_then(|json| json["id"].as_str().map(str::to_owned))
            .ok_or("the upload finished but Drive named no file")?;
        Ok(format!("https://drive.google.com/file/d/{id}/view"))
    }
}

/// Writes a file only its owner can read.
fn write_private(path: &Path, text: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("could not create {}: {error}", parent.display()))?;
    }
    std::fs::write(path, text)
        .map_err(|error| format!("could not write {}: {error}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

/// Waits for the browser's redirect and answers the code in it.
///
/// Anything arriving with the wrong `state` is ignored rather than
/// accepted: that is the check that the answer is the one this call asked
/// for and not another page's.
fn wait_for_code(listener: &TcpListener, state: &str) -> Result<String, String> {
    for stream in listener.incoming() {
        let mut stream = stream.map_err(|error| format!("the browser did not arrive: {error}"))?;
        let mut line = String::new();
        BufReader::new(&stream)
            .read_line(&mut line)
            .map_err(|error| format!("could not read the answer: {error}"))?;
        let found = code_from(&line).filter(|(_, sent)| sent == state);
        let body = if found.is_some() {
            "<h2>Spojeno.</h2><p>Možeš zatvoriti ovu karticu i vratiti se u Imperija Studio.</p>"
        } else {
            "<h2>Nije spojeno.</h2><p>Vrati se u Imperija Studio i pokušaj ponovo.</p>"
        };
        let _ = write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\n\
             Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let _ = stream.flush();
        if let Some((code, _)) = found {
            return Ok(code);
        }
        return Err("access was not granted".to_owned());
    }
    Err("the browser never came back".to_owned())
}

/// A form POST that answers JSON.
fn post_form(url: &str, fields: &[(&str, &str)]) -> Result<serde_json::Value, String> {
    Ok(read_json(
        ureq::post(url).send_form(fields).map_err(describe)?,
    ))
}

/// A response's body as JSON. Read as text and parsed here rather than
/// through ureq's own helper, which is behind a feature this build does
/// not carry. A body that is not JSON parses as null, and every field
/// read from it is then absent - which is what the callers already check.
fn read_json(response: ureq::Response) -> serde_json::Value {
    response
        .into_string()
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

/// A network error in words, with Google's own message where it sent one -
/// "invalid_grant" alone tells a person nothing.
fn describe(error: ureq::Error) -> String {
    match error {
        ureq::Error::Status(code, response) => {
            let body = response.into_string().unwrap_or_default();
            let json: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
            let said = json["error_description"]
                .as_str()
                .or_else(|| json["error"]["message"].as_str())
                .or_else(|| json["error"].as_str())
                .unwrap_or("")
                .to_owned();
            if said.is_empty() {
                format!("Drive answered {code}")
            } else {
                format!("Drive answered {code}: {said}")
            }
        }
        other => format!("could not reach Drive: {other}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64url_matches_the_rfc_and_never_pads() {
        assert_eq!(base64url(b""), "");
        assert_eq!(base64url(b"f"), "Zg");
        assert_eq!(base64url(b"fo"), "Zm8");
        assert_eq!(base64url(b"foo"), "Zm9v");
        assert_eq!(base64url(b"foobar"), "Zm9vYmFy");
        assert!(!base64url(b"f").contains('='), "no padding");
        // The two characters that make it URL-safe, where the ordinary
        // alphabet would put + and /.
        let odd = base64url(&[0xfb, 0xff, 0xbe]);
        assert_eq!(odd, "-_--");
        assert!(!odd.contains('+') && !odd.contains('/'), "{odd}");
    }

    #[test]
    fn the_challenge_is_the_verifiers_digest() {
        // The pair from RFC 7636's own example, which is what Google checks.
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        assert_eq!(
            pkce(verifier),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn a_different_verifier_gives_a_different_challenge() {
        assert_ne!(pkce("one"), pkce("two"));
    }

    #[test]
    fn escaping_leaves_the_unreserved_set_alone() {
        assert_eq!(escape("aZ09-._~"), "aZ09-._~");
        assert_eq!(escape("a b"), "a%20b");
        assert_eq!(escape("https://x/y?z=1"), "https%3A%2F%2Fx%2Fy%3Fz%3D1");
    }

    #[test]
    fn escaping_and_unescaping_are_a_round_trip() {
        for text in ["plain", "a b&c=d", "https://127.0.0.1:1/", "šđč"] {
            assert_eq!(unescape(&escape(text)), text, "{text}");
        }
    }

    #[test]
    fn the_consent_address_carries_what_google_requires() {
        let client = Client {
            id: "abc.apps".to_owned(),
            secret: String::new(),
        };
        let url = auth_url(&client, "http://127.0.0.1:5000", "chal", "st");
        for part in [
            "client_id=abc.apps",
            "code_challenge=chal",
            "code_challenge_method=S256",
            "state=st",
            "response_type=code",
            // Without this Google sends no lasting token, and the
            // connection would die in an hour.
            "access_type=offline",
        ] {
            assert!(url.contains(part), "{part} missing from {url}");
        }
        assert!(url.contains("drive.file"), "the narrow scope: {url}");
        assert!(!url.contains("drive.readonly"), "nothing wider: {url}");
    }

    #[test]
    fn the_code_is_read_out_of_the_redirect() {
        let line = "GET /?state=st&code=4%2F0AX&scope=drive.file HTTP/1.1";
        assert_eq!(code_from(line), Some(("4/0AX".to_owned(), "st".to_owned())));
    }

    #[test]
    fn a_refusal_is_not_a_code() {
        assert_eq!(
            code_from("GET /?error=access_denied&state=st HTTP/1.1"),
            None
        );
        assert_eq!(code_from("GET / HTTP/1.1"), None);
        assert_eq!(code_from(""), None);
    }

    #[test]
    fn a_client_with_no_id_is_not_set_up() {
        assert!(
            !Client {
                id: String::new(),
                secret: "s".to_owned()
            }
            .is_set()
        );
        assert!(
            !Client {
                id: "   ".to_owned(),
                secret: String::new()
            }
            .is_set()
        );
        assert!(
            Client {
                id: "abc".to_owned(),
                secret: String::new()
            }
            .is_set()
        );
    }

    #[test]
    fn random_bytes_are_the_length_asked_for_and_not_all_the_same() {
        assert_eq!(random(48).len(), 48);
        assert_ne!(random(32), random(32));
    }

    #[test]
    fn an_unconnected_drive_says_so_and_refuses_to_upload() {
        let dir = std::env::temp_dir().join(format!("drive-test-{}", std::process::id()));
        let drive = Drive::new(&dir);
        assert!(!drive.is_connected());
        assert!(drive.access().is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
