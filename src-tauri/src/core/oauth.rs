//! Native OAuth handshakes for explicit GitHub/Jira connections. Secrets stay in Keychain.
use super::{
    github::authenticated_login,
    plugins::{self, AuthMethod, PluginConnection, Provider},
    types::*,
    CredentialStore, KeychainCredentialStore,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    io::{Read, Write},
    net::TcpListener,
    process::{Command, Stdio},
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use url::{form_urlencoded, Url};

const JIRA_CALLBACK: &str = "http://127.0.0.1:48913/aidebook/oauth";
const GITHUB_DEVICE_URL: &str = "https://github.com/login/device";

fn error(message: &str) -> CoreError {
    CoreError::Connector {
        message: message.into(),
    }
}
fn epoch() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
fn quote(value: &str) -> String {
    format!(
        "\"{}\"",
        value
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', "\\n")
            .replace('\r', "\\r")
    )
}
fn request(url: &str, data: Option<(&str, String)>, bearer: Option<&str>) -> CoreResult<Value> {
    let mut config = format!("url = {}\nheader = \"Accept: application/json\"\nwrite-out = \"\\nAIDEBOOK_STATUS:%{{http_code}}\"\n", quote(url));
    if let Some(token) = bearer {
        if token.chars().any(char::is_control) {
            return Err(error("OAuth credential is invalid"));
        }
        config.push_str(&format!(
            "header = {}\n",
            quote(&format!("Authorization: Bearer {token}"))
        ));
    }
    if let Some((content_type, body)) = data {
        config.push_str(&format!(
            "request = \"POST\"\nheader = {}\ndata = {}\n",
            quote(&format!("Content-Type: {content_type}")),
            quote(&body)
        ));
    }
    let mut child = Command::new("curl")
        .args([
            "-q",
            "--silent",
            "--show-error",
            "--max-time",
            "30",
            "--connect-timeout",
            "10",
            "--max-filesize",
            "8388608",
            "--proto",
            "=https",
            "--config",
            "-",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| error("OAuth network transport is unavailable"))?;
    if child
        .stdin
        .take()
        .ok_or_else(|| error("OAuth request unavailable"))?
        .write_all(config.as_bytes())
        .is_err()
    {
        let _ = child.kill();
        let _ = child.wait();
        return Err(error("OAuth request could not be sent"));
    }
    let output = child
        .wait_with_output()
        .map_err(|_| error("OAuth request failed"))?;
    if !output.status.success() {
        return Err(error("OAuth network request failed or timed out"));
    }
    let text = String::from_utf8(output.stdout).map_err(|_| error("OAuth response is not text"))?;
    let (body, status) = text
        .rsplit_once("\nAIDEBOOK_STATUS:")
        .ok_or_else(|| error("OAuth response status missing"))?;
    if status != "200" {
        return Err(error(
            "OAuth provider rejected the request; check the app configuration or reconnect",
        ));
    }
    serde_json::from_str(body).map_err(|_| error("OAuth provider returned invalid JSON"))
}
fn form(pairs: &[(&str, &str)]) -> String {
    form_urlencoded::Serializer::new(String::new())
        .extend_pairs(pairs.iter().copied())
        .finish()
}
fn secret_key(id: &str) -> String {
    format!("plugin-oauth-secret-{id}")
}
pub fn set_client_secret(connection: &PluginConnection, secret: &str) -> CoreResult<()> {
    if connection.provider != Provider::Jira || connection.auth != AuthMethod::Oauth {
        return Err(error(
            "OAuth client secret applies to Jira OAuth connections",
        ));
    }
    if secret.is_empty() || secret.chars().any(char::is_control) {
        return Err(error("OAuth client secret is invalid"));
    }
    KeychainCredentialStore.set(&secret_key(&connection.id), secret)
}
pub fn delete_credentials(id: &str) -> CoreResult<()> {
    KeychainCredentialStore.delete(&plugins::credential_key(id))?;
    KeychainCredentialStore.delete(&secret_key(id))
}
#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredToken {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    expires_at: u64,
    #[serde(default)]
    cloud_id: Option<String>,
}
fn store(id: &str, token: &StoredToken) -> CoreResult<()> {
    let value =
        serde_json::to_string(token).map_err(|_| error("OAuth credential could not be encoded"))?;
    KeychainCredentialStore.set(&plugins::credential_key(id), &value)
}
fn saved(id: &str) -> CoreResult<StoredToken> {
    let value = KeychainCredentialStore
        .get(&plugins::credential_key(id))?
        .ok_or_else(|| error("connect this OAuth account first"))?;
    serde_json::from_str(&value)
        .map_err(|_| error("OAuth credential is invalid; reconnect this account"))
}
fn token_from(value: &Value, cloud_id: Option<String>) -> CoreResult<StoredToken> {
    let access_token = value["access_token"]
        .as_str()
        .filter(|s| !s.is_empty() && !s.chars().any(char::is_control))
        .ok_or_else(|| error("OAuth provider did not return an access token"))?
        .to_string();
    let refresh_token = value["refresh_token"]
        .as_str()
        .filter(|s| !s.is_empty() && !s.chars().any(char::is_control))
        .map(str::to_string);
    let expires_at = value["expires_in"]
        .as_u64()
        .map(|seconds| epoch().saturating_add(seconds))
        .unwrap_or(0);
    Ok(StoredToken {
        access_token,
        refresh_token,
        expires_at,
        cloud_id,
    })
}
fn jira_site_id(connection: &PluginConnection, token: &str) -> CoreResult<String> {
    let sites = request(
        "https://api.atlassian.com/oauth/token/accessible-resources",
        None,
        Some(token),
    )?;
    select_jira_site_id(connection, &sites)
}
fn select_jira_site_id(connection: &PluginConnection, sites: &Value) -> CoreResult<String> {
    let site = sites
        .as_array()
        .and_then(|items| {
            items.iter().find(|site| {
                site["url"]
                    .as_str()
                    .is_some_and(|url| url.trim_end_matches('/') == connection.scope)
                    && site["scopes"]
                        .as_array()
                        .is_some_and(|scopes| scopes.iter().any(|scope| scope == "read:jira-work"))
            })
        })
        .ok_or_else(|| error("OAuth grant does not include this Jira site and read:jira-work"))?;
    let id = site["id"]
        .as_str()
        .ok_or_else(|| error("Jira cloud ID is missing"))?;
    uuid::Uuid::parse_str(id).map_err(|_| error("Jira cloud ID is invalid"))?;
    Ok(id.to_string())
}
fn jira_secret(connection: &PluginConnection) -> CoreResult<String> {
    KeychainCredentialStore
        .get(&secret_key(&connection.id))?
        .ok_or_else(|| error("save the Jira OAuth app client secret first"))
}
fn refresh_token(connection: &PluginConnection, previous: &StoredToken) -> CoreResult<StoredToken> {
    let old = previous
        .refresh_token
        .as_deref()
        .ok_or_else(|| error("OAuth session expired; reconnect this account"))?;
    let response = match connection.provider {
        Provider::Github => request("https://github.com/login/oauth/access_token", Some(("application/x-www-form-urlencoded", form(&[
            ("client_id", &connection.oauth_client_id), ("grant_type", "refresh_token"), ("refresh_token", old)
        ]))), None)?,
        Provider::Jira => request("https://auth.atlassian.com/oauth/token", Some(("application/json", json!({
            "grant_type":"refresh_token", "client_id":connection.oauth_client_id, "client_secret":jira_secret(connection)?, "refresh_token":old
        }).to_string())), None)?,
        _ => return Err(error("OAuth is unavailable for this provider")),
    };
    if response.get("error").is_some() {
        return Err(error("OAuth refresh was rejected; reconnect this account"));
    }
    let next = token_from(&response, previous.cloud_id.clone())?;
    if next.refresh_token.is_none()
        || (connection.provider == Provider::Jira && next.expires_at == 0)
    {
        return Err(error(
            "OAuth refresh did not return a replacement refresh token; reconnect",
        ));
    }
    store(&connection.id, &next)?;
    Ok(next)
}
static REFRESH_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
pub fn access_token(connection: &PluginConnection) -> CoreResult<String> {
    let _guard = REFRESH_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .map_err(|_| error("OAuth session unavailable"))?;
    let mut token = saved(&connection.id)?;
    if token.expires_at != 0 && token.expires_at <= epoch().saturating_add(60) {
        token = refresh_token(connection, &token)?;
    }
    Ok(token.access_token)
}
pub fn cloud_id(connection: &PluginConnection) -> CoreResult<String> {
    saved(&connection.id)?
        .cloud_id
        .ok_or_else(|| error("Jira OAuth site selection is missing; reconnect"))
}
#[derive(Debug, Clone, Serialize)]
pub struct OAuthStart {
    pub url: String,
    pub user_code: Option<String>,
}
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum OAuthStatus {
    Pending,
    Complete,
    Failed { message: String },
}
enum Flow {
    Github {
        device_code: String,
        expires: Instant,
        next_poll: Instant,
        interval: Duration,
        connection: PluginConnection,
    },
    JiraPending,
    Done(Result<(), String>),
}
#[derive(Clone, Default)]
pub struct OAuthManager {
    flows: Arc<Mutex<HashMap<String, Flow>>>,
}
impl OAuthManager {
    pub fn start(&self, mut connection: PluginConnection) -> CoreResult<OAuthStart> {
        connection.validate()?;
        if connection.auth != AuthMethod::Oauth {
            return Err(error("select OAuth for this connection first"));
        }
        let id = connection.id.clone();
        let mut flows = self
            .flows
            .lock()
            .map_err(|_| error("OAuth state unavailable"))?;
        match flows.get(&id) {
            Some(Flow::Done(_)) => {
                flows.remove(&id);
            }
            Some(Flow::Github { expires, .. }) if Instant::now() >= *expires => {
                flows.remove(&id);
            }
            Some(_) => {
                return Err(error(
                    "OAuth sign-in is already in progress for this connection",
                ))
            }
            None => (),
        }
        match connection.provider {
            Provider::Github => {
                let response = request(
                    "https://github.com/login/device/code",
                    Some((
                        "application/x-www-form-urlencoded",
                        form(&[("client_id", &connection.oauth_client_id)]),
                    )),
                    None,
                )?;
                let device_code = response["device_code"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| error("GitHub device code is missing"))?;
                let user_code = response["user_code"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| error("GitHub user code is missing"))?;
                let url = response["verification_uri"]
                    .as_str()
                    .filter(|url| *url == GITHUB_DEVICE_URL)
                    .ok_or_else(|| error("GitHub verification URL is unexpected"))?;
                let expires = response["expires_in"].as_u64().unwrap_or(900).min(900);
                let interval =
                    Duration::from_secs(response["interval"].as_u64().unwrap_or(5).max(5));
                flows.insert(
                    id,
                    Flow::Github {
                        device_code: device_code.into(),
                        expires: Instant::now() + Duration::from_secs(expires),
                        next_poll: Instant::now() + interval,
                        interval,
                        connection,
                    },
                );
                Ok(OAuthStart {
                    url: url.into(),
                    user_code: Some(user_code.into()),
                })
            }
            Provider::Jira => {
                let secret = jira_secret(&connection)?;
                if secret.is_empty() {
                    return Err(error("Jira OAuth app client secret is missing"));
                }
                let listener = TcpListener::bind("127.0.0.1:48913")
                    .map_err(|_| error("OAuth callback port 48913 is unavailable"))?;
                listener
                    .set_nonblocking(true)
                    .map_err(|_| error("OAuth callback listener unavailable"))?;
                let state = uuid::Uuid::new_v4().to_string();
                let mut url = Url::parse("https://auth.atlassian.com/authorize")
                    .map_err(|_| error("OAuth URL unavailable"))?;
                url.query_pairs_mut()
                    .append_pair("audience", "api.atlassian.com")
                    .append_pair("client_id", &connection.oauth_client_id)
                    .append_pair("scope", "read:jira-work offline_access")
                    .append_pair("redirect_uri", JIRA_CALLBACK)
                    .append_pair("state", &state)
                    .append_pair("response_type", "code")
                    .append_pair("prompt", "consent");
                flows.insert(id.clone(), Flow::JiraPending);
                let flows_ref = self.flows.clone();
                std::thread::spawn(move || {
                    let result =
                        jira_callback(listener, &connection, &state).map_err(|e| e.to_string());
                    if let Ok(mut guard) = flows_ref.lock() {
                        guard.insert(id, Flow::Done(result));
                    }
                });
                Ok(OAuthStart {
                    url: url.into(),
                    user_code: None,
                })
            }
            _ => Err(error("OAuth is unavailable for this provider")),
        }
    }
    pub fn status(&self, id: &str) -> CoreResult<OAuthStatus> {
        let mut flows = self
            .flows
            .lock()
            .map_err(|_| error("OAuth state unavailable"))?;
        let Some(flow) = flows.get_mut(id) else {
            return Err(error("start OAuth sign-in first"));
        };
        match flow {
            Flow::JiraPending => Ok(OAuthStatus::Pending),
            Flow::Done(result) => {
                let status = match result {
                    Ok(()) => OAuthStatus::Complete,
                    Err(message) => OAuthStatus::Failed {
                        message: message.clone(),
                    },
                };
                flows.remove(id);
                Ok(status)
            }
            Flow::Github {
                device_code,
                expires,
                next_poll,
                interval,
                connection,
            } => {
                if Instant::now() >= *expires {
                    flows.remove(id);
                    return Ok(OAuthStatus::Failed {
                        message: "GitHub code expired; start again".into(),
                    });
                }
                if Instant::now() < *next_poll {
                    return Ok(OAuthStatus::Pending);
                }
                *next_poll = Instant::now() + *interval;
                let response = request(
                    "https://github.com/login/oauth/access_token",
                    Some((
                        "application/x-www-form-urlencoded",
                        form(&[
                            ("client_id", &connection.oauth_client_id),
                            ("device_code", device_code),
                            ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                        ]),
                    )),
                    None,
                )?;
                match response["error"].as_str() {
                    Some("authorization_pending") => Ok(OAuthStatus::Pending),
                    Some("slow_down") => {
                        *interval += Duration::from_secs(5);
                        *next_poll = Instant::now() + *interval;
                        Ok(OAuthStatus::Pending)
                    }
                    Some(_) => {
                        flows.remove(id);
                        Ok(OAuthStatus::Failed {
                            message: "GitHub authorization failed or expired; start again".into(),
                        })
                    }
                    None => {
                        let token = token_from(&response, None)?;
                        if !authenticated_login(&token.access_token)?
                            .eq_ignore_ascii_case(&connection.account)
                        {
                            flows.remove(id);
                            return Ok(OAuthStatus::Failed {
                                message: "GitHub OAuth account differs from the saved login".into(),
                            });
                        }
                        store(id, &token)?;
                        flows.remove(id);
                        Ok(OAuthStatus::Complete)
                    }
                }
            }
        }
    }
}
fn jira_callback(
    listener: TcpListener,
    connection: &PluginConnection,
    expected_state: &str,
) -> CoreResult<()> {
    let deadline = Instant::now() + Duration::from_secs(300);
    while Instant::now() < deadline {
        match listener.accept() {
            Ok((mut stream, address)) => {
                if !address.ip().is_loopback() {
                    continue;
                }
                stream.set_read_timeout(Some(Duration::from_secs(5))).ok();
                let mut buffer = [0u8; 4096];
                let count = stream
                    .read(&mut buffer)
                    .map_err(|_| error("OAuth callback could not be read"))?;
                let first = String::from_utf8_lossy(&buffer[..count]);
                let path = first
                    .lines()
                    .next()
                    .and_then(|line| line.strip_prefix("GET "))
                    .and_then(|line| line.split_once(' ').map(|(path, _)| path))
                    .unwrap_or("");
                let url = Url::parse(&format!("http://127.0.0.1{path}"))
                    .map_err(|_| error("OAuth callback is invalid"))?;
                let pairs: HashMap<String, String> = url.query_pairs().into_owned().collect();
                let valid = url.path() == "/aidebook/oauth"
                    && pairs
                        .get("state")
                        .is_some_and(|state| state == expected_state);
                if !valid {
                    let _ = stream.write_all(b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                    continue;
                }
                let code = pairs.get("code").filter(|code| !code.is_empty()).cloned();
                let page = if code.is_some() {
                    "Aidebook received the authorization. Return to the app."
                } else {
                    "Authorization was declined. Return to Aidebook."
                };
                let body = format!("<html><body>{page}</body></html>");
                let response = format!("HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                let _ = stream.write_all(response.as_bytes());
                let code = code.ok_or_else(|| error("Jira authorization was declined"))?;
                if code.len() > 4096 || code.chars().any(char::is_control) {
                    return Err(error("Jira authorization code is invalid"));
                }
                let response = request("https://auth.atlassian.com/oauth/token", Some(("application/json", json!({
                    "grant_type":"authorization_code", "client_id":connection.oauth_client_id,
                    "client_secret":jira_secret(connection)?, "code":code, "redirect_uri":JIRA_CALLBACK
                }).to_string())), None)?;
                let mut token = token_from(&response, None)?;
                if token.refresh_token.is_none() || token.expires_at == 0 {
                    return Err(error(
                        "Jira OAuth did not return a renewable session; check offline_access",
                    ));
                }
                token.cloud_id = Some(jira_site_id(connection, &token.access_token)?);
                store(&connection.id, &token)?;
                return Ok(());
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(150))
            }
            Err(_) => return Err(error("OAuth callback listener failed")),
        }
    }
    Err(error("Jira authorization timed out; start again"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn token_response_never_requires_a_refresh_token_for_nonexpiring_github() {
        let token = token_from(&json!({"access_token":"example"}), None).unwrap();
        assert_eq!(token.expires_at, 0);
        assert!(token.refresh_token.is_none());
    }
    #[test]
    fn jira_callback_is_fixed_to_loopback() {
        assert_eq!(
            Url::parse(JIRA_CALLBACK).unwrap().host_str(),
            Some("127.0.0.1")
        );
    }
    #[test]
    fn jira_grant_must_match_selected_site_and_read_scope() {
        let connection: PluginConnection = serde_json::from_value(json!({
            "id":"jira-test", "provider":"jira", "label":"Jira", "account":"personal",
            "scope":"https://team.atlassian.net", "project":"TEST", "auth":"oauth", "oauth_client_id":"client"
        })).unwrap();
        let sites = json!([
            {"id":"11111111-1111-4111-8111-111111111111", "url":"https://other.atlassian.net", "scopes":["read:jira-work"]},
            {"id":"22222222-2222-4222-8222-222222222222", "url":"https://team.atlassian.net", "scopes":["write:jira-work"]}
        ]);
        assert!(select_jira_site_id(&connection, &sites).is_err());
        let allowed = json!([{"id":"33333333-3333-4333-8333-333333333333", "url":"https://team.atlassian.net", "scopes":["read:jira-work"]}]);
        assert_eq!(
            select_jira_site_id(&connection, &allowed).unwrap(),
            "33333333-3333-4333-8333-333333333333"
        );
    }
}
