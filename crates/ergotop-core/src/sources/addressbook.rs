//! ergexplorer.com address book: fetch, disk cache, embedded snapshot.
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use serde::Deserialize;

use super::{Result, SourceError};
use crate::classify::{BookEntry, Kind};

pub const BOOK_API: &str = "https://api.ergexplorer.com";
const MAX_AGE: Duration = Duration::from_secs(24 * 3600);
const SNAPSHOT: &str = include_str!("../../../../assets/addressbook-snapshot.json");

#[derive(Deserialize)]
struct BookPage {
    items: Vec<BookItem>,
}

#[derive(Deserialize)]
struct BookItem {
    address: String,
    name: String,
    #[serde(rename = "type", default)]
    kind: String,
}

pub fn parse(json: &str) -> Result<Vec<BookEntry>> {
    let page: BookPage =
        serde_json::from_str(json).map_err(|e| SourceError::Parse(e.to_string()))?;
    Ok(page
        .items
        .into_iter()
        .map(|i| BookEntry {
            address: i.address,
            name: i.name,
            kind: Kind::parse(&i.kind),
        })
        .collect())
}

pub fn snapshot() -> Vec<BookEntry> {
    parse(SNAPSHOT).expect("assets/addressbook-snapshot.json")
}

pub fn is_stale(modified: SystemTime, now: SystemTime) -> bool {
    now.duration_since(modified)
        .map(|age| age > MAX_AGE)
        .unwrap_or(false)
}

pub fn load_cache(path: &Path) -> Option<(Vec<BookEntry>, SystemTime)> {
    let text = std::fs::read_to_string(path).ok()?;
    let modified = std::fs::metadata(path).ok()?.modified().ok()?;
    Some((parse(&text).ok()?, modified))
}

pub fn save_cache(path: &Path, raw: &str) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, raw)
}

/// Entries to start with (cache, else embedded snapshot) and whether a network refresh is due.
pub fn initial(cache: Option<&Path>) -> (Vec<BookEntry>, bool) {
    match cache.and_then(load_cache) {
        Some((entries, modified)) => (entries, is_stale(modified, SystemTime::now())),
        None => (snapshot(), true),
    }
}

pub async fn fetch(http: &reqwest::Client, base: &str) -> Result<(String, Vec<BookEntry>)> {
    let url = format!(
        "{}/addressbook/getAddresses?offset=0&limit=5000&type=all&order=nameAsc&query=&testnet=0",
        base.trim_end_matches('/')
    );
    let resp = http.get(&url).send().await?;
    if !resp.status().is_success() {
        return Err(SourceError::Status(resp.status().as_u16()));
    }
    let raw = resp.text().await?;
    let entries = parse(&raw)?;
    if entries.is_empty() {
        return Err(SourceError::Parse(
            "address book response had no entries".into(),
        ));
    }
    Ok((raw, entries))
}

/// Default cache file location.
pub fn default_cache_path(cache_dir: Option<PathBuf>) -> Option<PathBuf> {
    cache_dir.map(|d| d.join("addressbook.json"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::classify::Kind;
    use crate::sources::http_client;
    use std::time::Duration;
    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const SAMPLE: &str = r#"{"items":[
        {"address":"88dhgzEuTXaRQTX5KNdnaWTTX7fEZVEQRn6qP4MJotPuRnS3QpoJxYpSaXoU1y7SHp8ZXMp92TH22DBY","name":"2miners","url":"https://2miners.com","type":"Mining pool","urltype":"","addressmd5":"x"},
        {"address":"9fyeEQBXvJzRYpRmrNy2eaB2kDqQGDk3KoSQGUB62db3tVDw2Z1","name":"$BASS","url":"","type":"Meme","urltype":"Pond","addressmd5":"y"}
    ],"total":2,"tokens":[]}"#;

    fn temp_file(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ergotop-book-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir.join("addressbook.json")
    }

    #[test]
    fn parses_entries_and_kinds() {
        let e = parse(SAMPLE).unwrap();
        assert_eq!(e.len(), 2);
        assert_eq!(e[0].name, "2miners");
        assert_eq!(e[0].kind, Kind::MiningPool);
        assert_eq!(e[1].kind, Kind::Meme);
    }

    #[test]
    fn embedded_snapshot_is_usable_offline() {
        let e = snapshot();
        assert!(e.len() >= 300, "got {}", e.len());
        assert!(e.iter().any(|b| b.kind == Kind::MiningPool));
    }

    #[test]
    fn staleness_is_24h() {
        let now = SystemTime::now();
        assert!(!is_stale(now - Duration::from_secs(23 * 3600), now));
        assert!(is_stale(now - Duration::from_secs(25 * 3600), now));
    }

    #[test]
    fn initial_uses_snapshot_without_cache_and_cache_when_present() {
        let path = temp_file("initial");
        let (entries, refresh) = initial(Some(path.as_path()));
        assert!(entries.len() >= 300);
        assert!(refresh, "no cache -> refresh needed");

        save_cache(&path, SAMPLE).unwrap();
        let (entries, refresh) = initial(Some(path.as_path()));
        assert_eq!(entries.len(), 2);
        assert!(!refresh, "fresh cache -> no refresh");

        let (entries, refresh) = initial(None);
        assert!(entries.len() >= 300);
        assert!(refresh);
    }

    #[tokio::test]
    async fn fetches_from_api() {
        let s = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/addressbook/getAddresses"))
            .and(query_param("type", "all"))
            .respond_with(ResponseTemplate::new(200).set_body_string(SAMPLE))
            .mount(&s)
            .await;
        let (raw, entries) = fetch(&http_client(), &s.uri()).await.unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(raw, SAMPLE);
    }

    #[tokio::test]
    async fn empty_book_from_api_is_rejected() {
        let s = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/addressbook/getAddresses"))
            .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"items":[],"total":0}"#))
            .mount(&s)
            .await;
        assert!(fetch(&http_client(), &s.uri()).await.is_err());
    }
}
