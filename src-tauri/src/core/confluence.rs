//! Read-only Confluence Cloud pages. Search does not import content.
use super::{
    plugins::{self, AuthMethod, ConfluenceMode, PluginConnection},
    types::*,
    ReadOnlyConnector,
};
use serde::Serialize;
use serde_json::Value;
use std::{
    collections::HashSet,
    io::Write,
    process::{Command, Stdio},
};

fn failed(message: &str) -> CoreError {
    CoreError::Connector {
        message: message.into(),
    }
}
fn invalid(message: &str) -> CoreError {
    CoreError::InvalidInput {
        field: "confluence".into(),
        message: message.into(),
    }
}
#[derive(Debug, Clone, Serialize)]
pub struct ConfluenceSearchResult {
    pub id: String,
    pub title: String,
    pub url: String,
}
pub struct ConfluenceConnector {
    connection: PluginConnection,
}
impl ConfluenceConnector {
    pub fn new(connection: PluginConnection) -> Self {
        Self { connection }
    }
}
impl ReadOnlyConnector for ConfluenceConnector {
    fn manifest(&self) -> ConnectorManifest {
        ConnectorManifest {
            id: "confluence".into(),
            version: "0.1.0".into(),
            api_version: "1".into(),
            capabilities: vec!["list".into(), "fetch".into()],
            permissions: vec!["read:confluence-content.all".into()],
        }
    }
    fn connection_id(&self) -> &str {
        &self.connection.id
    }
    fn list(&self) -> CoreResult<Vec<Snapshot>> {
        let mut c = self.connection.clone();
        c.validate()?;
        let token = plugins::token(&c)?;
        list_with(&c, |path| request(&c, &token, path))
    }
    fn fetch(&self, source: &SourceRef) -> CoreResult<Snapshot> {
        self.list()?
            .into_iter()
            .find(|s| &s.source == source)
            .ok_or_else(|| failed("Confluence page is outside the selected scope or unavailable"))
    }
}
fn numeric_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 32 && id.bytes().all(|b| b.is_ascii_digit())
}
/// Accept numeric IDs, modern page URLs and legacy viewpage.action?pageId URLs on this site only.
pub fn normalize_page_id(c: &PluginConnection, input: &str) -> CoreResult<String> {
    let input = input.trim();
    if numeric_id(input) {
        return Ok(input.into());
    }
    let path = input
        .strip_prefix(&format!("{}/", c.scope.trim_end_matches('/')))
        .ok_or_else(|| invalid("use a page ID or a page URL from this connection's site"))?;
    if let Some((_, tail)) = path.split_once("/pages/") {
        let id = tail.split(['/', '?', '#']).next().unwrap_or("");
        if numeric_id(id) {
            return Ok(id.into());
        }
    }
    if path.starts_with("wiki/pages/viewpage.action?") {
        for pair in path
            .split_once('?')
            .unwrap()
            .1
            .split('#')
            .next()
            .unwrap_or("")
            .split('&')
        {
            if let Some(id) = pair.strip_prefix("pageId=") {
                if numeric_id(id) {
                    return Ok(id.into());
                }
            }
        }
    }
    Err(invalid("URL must contain a numeric Confluence page ID"))
}
fn encode(value: &str) -> String {
    value
        .bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}
fn cql_literal(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}
fn search_path(cql: &str) -> String {
    format!("/wiki/rest/api/search?cql={}&limit=100", encode(cql))
}
fn page_url(c: &PluginConnection, id: &str) -> String {
    format!("{}/wiki/pages/viewpage.action?pageId={id}", c.scope)
}
fn result(c: &PluginConnection, value: &Value) -> CoreResult<ConfluenceSearchResult> {
    if value["type"].as_str() != Some("page") {
        return Err(failed("Confluence returned a non-page search result"));
    }
    let id = value["id"]
        .as_str()
        .filter(|s| numeric_id(s))
        .ok_or_else(|| failed("Confluence page ID is missing or invalid"))?;
    let title = value["title"]
        .as_str()
        .ok_or_else(|| failed("Confluence page title is missing"))?;
    Ok(ConfluenceSearchResult {
        id: id.into(),
        title: title.into(),
        url: page_url(c, id),
    })
}
fn next_path(c: &PluginConnection, link: &str) -> CoreResult<String> {
    let relative = link.strip_prefix(&c.scope).unwrap_or(link);
    let path = if relative.starts_with("/rest/api/search?") {
        format!("/wiki{relative}")
    } else {
        relative.into()
    };
    if !path.starts_with("/wiki/rest/api/search?")
        || path.chars().any(char::is_control)
        || path.contains('#')
    {
        return Err(failed("Confluence returned an unsafe pagination link"));
    }
    Ok(path)
}
fn search_with(
    c: &PluginConnection,
    cql: &str,
    request: &mut impl FnMut(&str) -> CoreResult<Value>,
) -> CoreResult<Vec<ConfluenceSearchResult>> {
    let mut path = search_path(cql);
    let mut seen = HashSet::new();
    let mut ids = HashSet::new();
    let mut results = Vec::new();
    for _ in 0..1000 {
        if !seen.insert(path.clone()) {
            return Err(failed(
                "Confluence repeated a pagination cursor; existing cache retained",
            ));
        }
        let response = request(&path)?;
        let rows = response["results"]
            .as_array()
            .ok_or_else(|| failed("Confluence search results are missing"))?;
        for row in rows {
            let item = result(c, &row["content"])?;
            if ids.insert(item.id.clone()) {
                results.push(item);
            }
        }
        match response["_links"].get("next") {
            Some(Value::String(next)) if !next.is_empty() => {
                if rows.is_empty() {
                    return Err(failed(
                        "Confluence pagination is incomplete; existing cache retained",
                    ));
                }
                path = next_path(c, next)?;
            }
            None | Some(Value::Null) => {
                // totalSize is supplied by the search endpoint; reject a truncated response when known.
                if response["totalSize"]
                    .as_u64()
                    .is_some_and(|total| total > results.len() as u64)
                {
                    return Err(failed(
                        "Confluence pagination is incomplete; existing cache retained",
                    ));
                }
                return Ok(results);
            }
            _ => return Err(failed("Confluence pagination link is malformed")),
        }
    }
    Err(failed(
        "Confluence pagination limit reached; existing cache retained",
    ))
}
pub fn search(
    connection: &PluginConnection,
    query: &str,
) -> CoreResult<Vec<ConfluenceSearchResult>> {
    let mut c = connection.clone();
    c.validate()?;
    if !c.reads_confluence() {
        return Err(invalid("select a Confluence connection"));
    }
    let query = query.trim();
    if query.is_empty() || query.len() > 500 || query.chars().any(char::is_control) {
        return Err(invalid("enter a search term of 1 to 500 bytes"));
    }
    let token = plugins::token(&c)?;
    search_with(
        &c,
        &format!("type = page AND text ~ {}", cql_literal(query)),
        &mut |path| request(&c, &token, path),
    )
}
pub fn verify_access(connection: &PluginConnection, token: &str) -> CoreResult<()> {
    let path = format!(
        "/wiki/rest/api/search?cql={}&limit=1",
        encode("type = page")
    );
    let result = request(connection, token, &path)?;
    if !result["results"].is_array() {
        return Err(failed("Confluence search response is incomplete"));
    }
    Ok(())
}
fn list_with(
    c: &PluginConnection,
    mut request: impl FnMut(&str) -> CoreResult<Value>,
) -> CoreResult<Vec<Snapshot>> {
    let ids = match c.confluence_mode {
        ConfluenceMode::Selected => c
            .confluence_page_ids
            .iter()
            .map(|id| normalize_page_id(c, id))
            .collect::<CoreResult<Vec<_>>>()?,
        ConfluenceMode::Authored | ConfluenceMode::Watched => {
            let field = if c.confluence_mode == ConfluenceMode::Authored {
                "creator"
            } else {
                "watcher"
            };
            search_with(
                c,
                &format!("type = page AND {field} = currentUser() ORDER BY lastmodified DESC"),
                &mut request,
            )?
            .into_iter()
            .map(|r| r.id)
            .collect()
        }
    };
    let mut seen = HashSet::new();
    let mut snapshots = Vec::new();
    for id in ids {
        if !seen.insert(id.clone()) {
            continue;
        }
        let value = request(&format!(
            "/wiki/rest/api/content/{id}?expand=body.storage,version"
        ))?;
        let page = result(c, &value)?;
        if page.id != id {
            return Err(failed("Confluence returned a different page ID"));
        }
        let body = value["body"]["storage"]["value"]
            .as_str()
            .ok_or_else(|| failed("Confluence page body is missing; existing cache retained"))?;
        snapshots.push(Snapshot::new(
            SourceRef::new(
                "confluence",
                &c.account,
                format!("{}:{id}", c.scope),
                page.url,
                "page",
            ),
            page.title,
            storage_text(body),
            value["version"]["when"].as_str().map(str::to_owned),
            now_rfc3339(),
        ));
    }
    Ok(snapshots)
}
/// Extract inert text only; storage markup is never rendered as HTML.
fn storage_text(html: &str) -> String {
    // Confluence code macros store literal code in CDATA. Escape it before tag
    // removal so code containing angle brackets remains searchable text.
    let mut escaped = String::new();
    let mut remaining = html;
    while let Some((before, tail)) = remaining.split_once("<![CDATA[") {
        escaped.push_str(before);
        if let Some((code, after)) = tail.split_once("]]>") {
            escaped.push_str(
                &code
                    .replace('&', "&amp;")
                    .replace('<', "&lt;")
                    .replace('>', "&gt;"),
            );
            remaining = after;
        } else {
            remaining = tail;
            break;
        }
    }
    escaped.push_str(remaining);
    let html = escaped.as_str();
    let mut text = String::new();
    let mut tag = String::new();
    let mut in_tag = false;
    let mut suppressed = false;
    for ch in html.chars() {
        if ch == '<' {
            in_tag = true;
            tag.clear();
        } else if in_tag && ch == '>' {
            in_tag = false;
            let name = tag
                .split_whitespace()
                .next()
                .unwrap_or("")
                .to_ascii_lowercase();
            if matches!(name.as_str(), "script" | "style") {
                suppressed = true;
            }
            if matches!(name.as_str(), "/script" | "/style") {
                suppressed = false;
            }
            if matches!(
                name.as_str(),
                "p" | "/p"
                    | "br"
                    | "br/"
                    | "div"
                    | "/div"
                    | "li"
                    | "/li"
                    | "tr"
                    | "/tr"
                    | "td"
                    | "/td"
                    | "h1"
                    | "/h1"
                    | "h2"
                    | "/h2"
            ) {
                text.push(' ');
            }
        } else if in_tag {
            tag.push(ch);
        } else if !suppressed {
            text.push(ch);
        }
    }
    let text = text
        .replace("&nbsp;", " ")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&amp;", "&");
    let mut decoded = String::new();
    let mut remaining = text.as_str();
    while let Some((before, tail)) = remaining.split_once("&#") {
        decoded.push_str(before);
        if let Some((digits, after)) = tail.split_once(';') {
            let code = if let Some(hex) = digits
                .strip_prefix('x')
                .or_else(|| digits.strip_prefix('X'))
            {
                u32::from_str_radix(hex, 16).ok()
            } else {
                digits.parse::<u32>().ok()
            };
            if let Some(ch) = code.and_then(char::from_u32) {
                decoded.push(ch);
                remaining = after;
                continue;
            }
        }
        decoded.push_str("&#");
        remaining = tail;
    }
    decoded.push_str(remaining);
    decoded.split_whitespace().collect::<Vec<_>>().join(" ")
}
fn quote(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}
fn request(c: &PluginConnection, token: &str, path: &str) -> CoreResult<Value> {
    if token.is_empty()
        || token.chars().any(char::is_control)
        || !path.starts_with("/wiki/rest/api/")
        || path.chars().any(char::is_control)
    {
        return Err(invalid("invalid Confluence request"));
    }
    let url = if c.auth == AuthMethod::Oauth {
        format!(
            "https://api.atlassian.com/ex/confluence/{}{path}",
            super::oauth::cloud_id(c)?
        )
    } else {
        format!("{}{path}", c.scope)
    };
    let authorization = if c.auth == AuthMethod::Oauth {
        format!(
            "header = {}\n",
            quote(&format!("Authorization: Bearer {token}"))
        )
    } else {
        format!("user = {}\n", quote(&format!("{}:{token}", c.account)))
    };
    let config = format!("url = {}\n{}header = \"Accept: application/json\"\nwrite-out = \"\\nAIDEBOOK_STATUS:%{{http_code}}\"\n", quote(&url), authorization);
    let mut child = Command::new("curl")
        .args([
            "-q",
            "--silent",
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
        .map_err(|_| failed("Confluence transport could not start"))?;
    if child
        .stdin
        .take()
        .ok_or_else(|| failed("Confluence request unavailable"))?
        .write_all(config.as_bytes())
        .is_err()
    {
        let _ = child.kill();
        let _ = child.wait();
        return Err(failed("Confluence request could not be sent"));
    }
    let output = child
        .wait_with_output()
        .map_err(|_| failed("Confluence request failed"))?;
    if !output.status.success() {
        return Err(failed(
            "Confluence network request failed or exceeded its limit",
        ));
    }
    let text =
        String::from_utf8(output.stdout).map_err(|_| failed("Confluence returned invalid text"))?;
    let (body, status) = text
        .rsplit_once("\nAIDEBOOK_STATUS:")
        .ok_or_else(|| failed("Confluence response status missing"))?;
    if status != "200" {
        return Err(failed(match status {
            "401" => "Confluence authentication expired or was rejected",
            "403" => "Confluence page access denied",
            "404" => "Confluence page is missing or inaccessible; existing cache retained",
            "429" => "Confluence rate limit exceeded; retry later",
            _ => "Confluence request was rejected",
        }));
    }
    serde_json::from_str(body).map_err(|_| failed("Confluence returned invalid JSON"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn connection(mode: &str) -> PluginConnection {
        serde_json::from_value(json!({"id":"confluence-test","provider":"confluence","label":"Docs","account":"test@example.com","scope":"https://example.atlassian.net","auth":"token","confluence_mode":mode,"confluence_page_ids":["12"]})).unwrap()
    }
    fn page(id: &str) -> Value {
        json!({"id":id,"type":"page","title":"Page","body":{"storage":{"value":"<p>Hello &amp; welcome</p><script>secret()</script><p>World</p>"}},"version":{"when":"2026-09-22"}})
    }
    #[test]
    fn selected_fetches_only_selected_ids_and_extracts_text() {
        let c = connection("selected");
        let snapshots = list_with(&c, |path| {
            assert_eq!(
                path,
                "/wiki/rest/api/content/12?expand=body.storage,version"
            );
            Ok(page("12"))
        })
        .unwrap();
        assert_eq!(snapshots.len(), 1);
        assert_eq!(
            snapshots[0].source.external_id,
            "https://example.atlassian.net:12"
        );
        assert_eq!(storage_text("<ac:plain-text-body><![CDATA[if (a < b) { x(); }]]></ac:plain-text-body><p>&#54620;&#xAE00;</p>"), "if (a < b) { x(); } 한글");
        assert_eq!(
            storage_text("<p>Hello &amp; welcome</p><script>secret()</script><p>World</p>"),
            "Hello & welcome World"
        );
        assert!(list_with(&c, |_| Ok(page("13"))).is_err());
        assert!(list_with(&c, |_| Err(failed("forbidden"))).is_err());
    }
    #[test]
    fn authored_and_watched_use_current_user_cql() {
        for (mode, field) in [("authored", "creator"), ("watched", "watcher")] {
            list_with(&connection(mode), |path| {
                assert!(path.contains(&encode(&format!("{field} = currentUser()"))));
                Ok(json!({"results":[],"totalSize":0,"_links":{}}))
            })
            .unwrap();
        }
    }
    #[test]
    fn search_follows_validated_pagination_and_rejects_incomplete_data() {
        let c = connection("authored");
        let mut calls = 0;
        let result = search_with(&c, "type = page", &mut |path| { calls += 1; if calls == 1 { Ok(json!({"results":[{"content":page("12")}],"totalSize":2,"_links":{"next":"/rest/api/search?cursor=next"}})) } else { assert_eq!(path,"/wiki/rest/api/search?cursor=next"); Ok(json!({"results":[{"content":page("13")}],"totalSize":2,"_links":{}})) } }).unwrap();
        assert_eq!(result.len(), 2);
        assert!(search_with(&c, "type = page", &mut |_| Ok(
            json!({"results":[],"totalSize":1,"_links":{}})
        ))
        .is_err());
        assert!(search_with(&c, "type = page", &mut |_| Ok(json!({"results":[{"content":page("12")}],"_links":{"next":"/rest/api/search?cursor=same"}}))).is_err());
        for url in [
            "https://evil.test/wiki/rest/api/search?x=1",
            "https://example.atlassian.net.evil.test/wiki/rest/api/search?x=1",
            "/wiki/rest/api/content?x=1",
            "//evil.test/wiki/rest/api/search?x=1",
        ] {
            assert!(next_path(&c, url).is_err());
        }
    }
    #[test]
    fn page_urls_are_site_bound_and_cql_is_quoted() {
        let c = connection("selected");
        for (input, expected) in [
            ("12", "12"),
            (
                "https://example.atlassian.net/wiki/spaces/TEAM/pages/123/Title",
                "123",
            ),
            (
                "https://example.atlassian.net/wiki/pages/viewpage.action?pageId=456",
                "456",
            ),
        ] {
            assert_eq!(normalize_page_id(&c, input).unwrap(), expected);
        }
        for input in [
            "https://evil.test/wiki/spaces/TEAM/pages/123",
            "https://example.atlassian.net.evil.test/wiki/pages/123",
            "12 OR type=page",
            "",
        ] {
            assert!(normalize_page_id(&c, input).is_err());
        }
        assert_eq!(cql_literal("\" OR type=page"), "\"\\\" OR type=page\"");
    }
}
