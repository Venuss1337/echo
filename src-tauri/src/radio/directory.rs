//! Radio Browser discovery. The connector only accepts HTTPS mirrors inside the
//! provider namespace, and station results are treated as untrusted references.
use super::{RequestKind, bounded_body, net, source_url, store};
use crate::{
    contract::{RadioFault, RadioSort, Station, StationPage, StationQuery},
    library::text::clean,
};
use serde::Deserialize;
use std::{
    collections::{HashSet, hash_map::RandomState},
    hash::BuildHasher,
    path::PathBuf,
    time::{Duration, Instant},
};
use tokio::{
    sync::{Mutex, Semaphore},
    time::timeout,
};
use url::Url;

const MAX_PAGE_BYTES: usize = 768 * 1024;
const MIRROR_TTL: Duration = Duration::from_secs(300);
const MAX_OFFSET: u32 = 12_000;
static DISCOVERY_DNS: Semaphore = Semaphore::const_new(1);

#[derive(Default)]
pub(super) struct Directory {
    mirrors: Mutex<Option<(Vec<String>, Instant)>>,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct RemoteStation {
    stationuuid: String,
    name: String,
    url: String,
    url_resolved: String,
    favicon: String,
    countrycode: String,
    language: String,
    tags: String,
    codec: String,
    bitrate: u32,
    lastcheckok: Option<u8>,
    hls: u8,
}

fn uuid(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(index, byte)| {
            if matches!(index, 8 | 13 | 18 | 23) {
                byte == b'-'
            } else {
                byte.is_ascii_hexdigit()
            }
        })
}

impl RemoteStation {
    fn reference(self) -> Option<store::Reference> {
        let name = clean(&self.name);
        if !uuid(&self.stationuuid) || name.is_empty() {
            return None;
        }
        // Preserve a supplied HTTPS origin so a downgrade cannot be hidden by
        // the directory's pre-resolved URL. Each actual hop is owned by ECHO.
        let stream = source_url(if self.url.is_empty() {
            &self.url_resolved
        } else {
            &self.url
        })
        .ok()?;
        let artwork = source_url(&self.favicon).ok().map(|url| url.to_string());
        let codec = if self.hls != 0 {
            "HLS".into()
        } else {
            clean(&self.codec)
        };
        Some(store::Reference {
            station: Station {
                id: self.stationuuid.to_ascii_lowercase(),
                name,
                country: clean(&self.countrycode),
                language: clean(&self.language),
                tags: clean(&self.tags),
                codec,
                bitrate: self.bitrate.min(1_000_000),
                reported_online: self.lastcheckok.map(|status| status == 1),
                manual: false,
                favorite: false,
                artwork: artwork.is_some(),
            },
            stream: stream.to_string(),
            artwork,
        })
    }
}

fn normalize(mut query: StationQuery) -> Result<StationQuery, RadioFault> {
    if query.offset > MAX_OFFSET
        || [&query.name, &query.country, &query.language, &query.tag]
            .iter()
            .any(|value| value.len() > 240)
    {
        return Err(RadioFault::Invalid);
    }
    query.name = clean(&query.name);
    query.country = clean(&query.country).to_ascii_uppercase();
    query.language = clean(&query.language).to_lowercase();
    query.tag = clean(&query.tag).to_lowercase();
    if !query.country.is_empty()
        && (query.country.len() != 2
            || !query.country.bytes().all(|byte| byte.is_ascii_alphabetic()))
    {
        return Err(RadioFault::Invalid);
    }
    Ok(query)
}

fn search_url(host: &str, query: &StationQuery) -> Result<Url, RadioFault> {
    let mut url = Url::parse(&format!("https://{host}/json/stations/search"))
        .map_err(|_| RadioFault::DirectoryUnavailable)?;
    let order = match query.sort {
        RadioSort::Popular => "clickcount",
        RadioSort::Name => "name",
        RadioSort::Recent => "clicktimestamp",
    };
    let mut params = url.query_pairs_mut();
    for (key, value) in [
        ("name", &query.name),
        ("countrycode", &query.country),
        ("language", &query.language),
        ("tag", &query.tag),
    ] {
        if !value.is_empty() {
            params.append_pair(key, value);
        }
    }
    params
        .append_pair("order", order)
        .append_pair(
            "reverse",
            if matches!(query.sort, RadioSort::Name) {
                "false"
            } else {
                "true"
            },
        )
        .append_pair("limit", &(store::PAGE + 1).to_string())
        .append_pair("offset", &query.offset.to_string());
    drop(params);
    Ok(url)
}

fn parse_page(raw: &[u8]) -> Result<(Vec<store::Reference>, bool), RadioFault> {
    // Parse each row independently so one malformed station cannot hide all
    // other results. Use the raw row count to advance the provider's cursor.
    let stations: Vec<serde_json::Value> =
        serde_json::from_slice(raw).map_err(|_| RadioFault::DirectoryUnavailable)?;
    if stations.len() > store::PAGE as usize + 1
        || (!stations.is_empty() && stations.iter().all(|row| !row.is_object()))
    {
        return Err(RadioFault::DirectoryUnavailable);
    }
    let more = stations.len() > store::PAGE as usize;
    let mut ids = HashSet::new();
    let entries = stations
        .into_iter()
        .take(store::PAGE as usize)
        .filter_map(|row| {
            serde_json::from_value::<RemoteStation>(row)
                .ok()?
                .reference()
        })
        .filter(|entry| ids.insert(entry.station.id.clone()))
        .collect();
    Ok((entries, more))
}

impl Directory {
    async fn mirrors(&self) -> Result<Vec<String>, RadioFault> {
        let mut cache = self.mirrors.lock().await;
        if let Some((mirrors, refreshed)) = &*cache
            && refreshed.elapsed() < MIRROR_TTL
        {
            return Ok(mirrors.clone());
        }
        let permit = DISCOVERY_DNS
            .try_acquire()
            .map_err(|_| RadioFault::DirectoryUnavailable)?;
        let mut mirrors = timeout(
            Duration::from_secs(5),
            tauri::async_runtime::spawn_blocking(move || {
                let _permit = permit;
                net::directory_mirrors()
            }),
        )
        .await
        .ok()
        .and_then(Result::ok)
        .unwrap_or_default();
        if mirrors.is_empty() {
            return Err(RadioFault::DirectoryUnavailable);
        }
        let state = RandomState::new();
        mirrors.sort_by_cached_key(|host| state.hash_one(host));
        *cache = Some((mirrors.clone(), Instant::now()));
        Ok(mirrors)
    }

    /// A single explicit directory Play can count a click. Retries, Resume
    /// and manual stations do not report anything to the directory.
    pub(super) async fn count_click(&self, id: String) {
        static REPORTING: Semaphore = Semaphore::const_new(1);
        let Ok(_permit) = REPORTING.try_acquire() else {
            return;
        };
        if !uuid(&id) {
            return;
        }
        let _ = timeout(Duration::from_secs(8), async {
            let mirrors = self.mirrors().await.ok()?;
            let mirror = mirrors.first()?;
            let url = Url::parse(&format!("https://{mirror}/json/url/{id}")).ok()?;
            bounded_body(url, RequestKind::Directory, 8192).await.ok()
        })
        .await;
    }

    pub(super) async fn search(
        &self,
        path: PathBuf,
        query: StationQuery,
    ) -> Result<StationPage, RadioFault> {
        let query = normalize(query)?;
        let offset = query.offset;
        let key = serde_json::to_string(&query).map_err(|_| RadioFault::Invalid)?;
        let reader_path = path.clone();
        let reader_key = key.clone();
        let cached = tauri::async_runtime::spawn_blocking(move || {
            let db = store::database(&reader_path)?;
            store::cached(&db, &reader_key)
        })
        .await
        .map_err(|_| RadioFault::Unavailable)??;
        let page = |stations, more, stale| StationPage {
            stations,
            next: (more && offset < MAX_OFFSET).then_some(offset + store::PAGE),
            stale,
        };
        if let Some((stations, more, false)) = cached {
            return Ok(page(stations, more, false));
        }
        let remote = async {
            for host in self.mirrors().await?.into_iter().take(3) {
                let url = search_url(&host, &query)?;
                if let Ok(bytes) = bounded_body(url, RequestKind::Directory, MAX_PAGE_BYTES).await
                    && let Ok(parsed) = parse_page(&bytes)
                {
                    return Ok(parsed);
                }
            }
            Err(RadioFault::DirectoryUnavailable)
        };
        if let Ok(Ok((entries, more))) = timeout(Duration::from_secs(20), remote).await {
            let stations = tauri::async_runtime::spawn_blocking(move || {
                let mut db = store::database(&path)?;
                store::remember_directory(&mut db, &key, &entries, more)?;
                entries
                    .iter()
                    .map(|entry| store::station(&db, &entry.station.id).map(|saved| saved.station))
                    .collect::<Result<Vec<_>, _>>()
            })
            .await
            .map_err(|_| RadioFault::Unavailable)??;
            return Ok(page(stations, more, false));
        }
        cached
            .map(|(stations, more, _)| page(stations, more, true))
            .ok_or(RadioFault::DirectoryUnavailable)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn admits_only_sanitized_station_metadata_and_opaque_ids() {
        let (entries, more) = parse_page(br#"[
          {"stationuuid":"8bd94a11-97b5-4d4b-bd44-11fb7eadfc9a","name":"North\u202e FM","url":"https://broadcast.invalid/live","favicon":"file:///logo.png","lastcheckok":1,"codec":"AAC"},
          {"stationuuid":"not-a-uuid", "name":"Ignore", "url":"https://broadcast.invalid/live"},
          {"stationuuid":"35311a55-32c6-4457-a170-369db5eb4031","name":"Local","url":"http://127.0.0.1/","url_resolved":"https://elsewhere.invalid/live"}
        ]"#).unwrap();
        assert!(!more);
        assert_eq!(entries.len(), 1);
        let entry = &entries[0];
        assert_eq!(entry.station.name, "North FM");
        assert!(!entry.station.artwork);
        assert_eq!(entry.station.reported_online, Some(true));
        assert!(
            !serde_json::to_string(&entry.station)
                .unwrap()
                .contains("broadcast.invalid")
        );
        assert!(parse_page(b"{ \"results\": [] }").is_err());
        assert!(parse_page(b"[1, 2]").is_err());
    }
}
