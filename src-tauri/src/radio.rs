//! Rust-owned direct radio fetches. Only the native decoder may consume the byte stream.
use reqwest::{Client, Response, dns, header, redirect};
use std::{
    collections::HashSet,
    io::{self, Read},
    net::{IpAddr, SocketAddr, ToSocketAddrs},
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::{
    runtime::Handle,
    sync::{Semaphore, mpsc},
    task::JoinHandle,
    time::timeout,
};
use url::{Host, Url};

pub(crate) mod artwork;
pub(crate) mod desktop;
mod directory;
#[cfg(test)]
pub(crate) mod fixtures;
mod icy;
mod net;
mod store;

const DEADLINE: Duration = Duration::from_secs(5);
const MAX_REDIRECTS: usize = 4;
const CHUNK_BYTES: usize = 16 * 1024;
const BUFFER_CHUNKS: usize = 4;
const MAX_URL_BYTES: usize = 4096;

#[derive(Clone)]
struct PinnedAddress {
    host: String,
    address: SocketAddr,
}

impl dns::Resolve for PinnedAddress {
    fn resolve(&self, name: dns::Name) -> dns::Resolving {
        let matched = self.host.eq_ignore_ascii_case(name.as_str());
        let address = self.address;
        Box::pin(async move {
            if !matched {
                return Err(io::Error::other("unexpected connector DNS query").into());
            }
            Ok(Box::new(std::iter::once(address)) as dns::Addrs)
        })
    }
}

fn valid_url(url: &Url) -> bool {
    matches!(url.scheme(), "http" | "https")
        && url.host().is_some()
        && url.port_or_known_default().is_some_and(|port| port != 0)
        && url.username().is_empty()
        && url.password().is_none()
        && url.fragment().is_none()
        && url.as_str().len() <= MAX_URL_BYTES
}

fn source_url(raw: &str) -> io::Result<Url> {
    let rejected = || io::Error::new(io::ErrorKind::PermissionDenied, "invalid media URL");
    if raw.len() > MAX_URL_BYTES {
        return Err(rejected());
    }
    let url = Url::parse(raw).map_err(|_| rejected())?;
    if !valid_url(&url)
        || match url.host() {
            Some(Host::Domain(name)) => !remote_hostname(name),
            Some(Host::Ipv4(ip)) => !public_address(ip.into()),
            Some(Host::Ipv6(ip)) => !public_address(ip.into()),
            None => true,
        }
    {
        return Err(rejected());
    }
    Ok(url)
}

fn remote_hostname(name: &str) -> bool {
    let name = name.trim_end_matches('.');
    name.contains('.')
        && !name.ends_with(".localhost")
        && !name.ends_with(".local")
        && name != "home.arpa"
        && !name.ends_with(".home.arpa")
}

// Reject entire special-purpose allocations, including globally reachable exceptions.
// Mapped IPv6 addresses are classified under the IPv4 rules before any dial.
fn public_address(ip: IpAddr) -> bool {
    match ip.to_canonical() {
        IpAddr::V4(ip) => {
            let [a, b, c, _] = ip.octets();
            !matches!(
                (a, b, c),
                (0 | 10 | 127 | 224..=255, _, _)
                    | (100, 64..=127, _)
                    | (169, 254, _)
                    | (172, 16..=31, _)
                    | (192, 0, 0 | 2)
                    | (192, 88, 99)
                    | (192, 168, _)
                    | (198, 18 | 19, _)
                    | (198, 51, 100)
                    | (203, 0, 113)
            )
        }
        IpAddr::V6(ip) => {
            let [a, b, ..] = ip.segments();
            a & 0xe000 == 0x2000
                && !(a == 0x2001 && (b < 0x200 || b == 0xdb8))
                && a != 0x2002
                && a != 0x3fff
        }
    }
}

fn allowed_address(ip: IpAddr, local: &HashSet<IpAddr>) -> bool {
    let ip = ip.to_canonical();
    public_address(ip) && !local.contains(&ip)
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum RequestKind {
    #[default]
    Stream,
    Directory,
    Artwork,
}

struct FetchOptions {
    limit: Duration,
    kind: RequestKind,
    #[cfg(test)]
    test_root: Option<rustls::pki_types::CertificateDer<'static>>,
}

// The verifier checks the server-supplied chain and bundled WebPKI roots in
// process. A system chain builder could fetch certificate URLs outside the
// pinned connector. Session resumption stays scoped to the fresh HTTP client.
fn radio_tls(options: &FetchOptions) -> io::Result<rustls::ClientConfig> {
    static ROOTS: OnceLock<Arc<rustls::RootCertStore>> = OnceLock::new();
    let roots = ROOTS
        .get_or_init(|| {
            Arc::new(rustls::RootCertStore {
                roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
            })
        })
        .clone();
    #[cfg(test)]
    let roots = if let Some(extra) = &options.test_root {
        let mut roots = (*roots).clone();
        roots
            .add(extra.clone())
            .map_err(|_| denied("invalid fixture certificate"))?;
        Arc::new(roots)
    } else {
        roots
    };
    #[cfg(not(test))]
    let _ = options;
    rustls::ClientConfig::builder_with_provider(rustls::crypto::ring::default_provider().into())
        .with_safe_default_protocol_versions()
        .map_err(|_| denied("radio TLS setup failed"))
        .map(|builder| builder.with_root_certificates(roots).with_no_client_auth())
}

impl FetchOptions {
    fn new(limit: Duration, kind: RequestKind) -> Self {
        Self {
            limit,
            kind,
            #[cfg(test)]
            test_root: None,
        }
    }
}

async fn fetch(url: Url, kind: RequestKind) -> io::Result<Response> {
    fetch_checked(
        url,
        resolve_host,
        |addresses| {
            let local = net::local_addresses()?;
            Ok(addresses
                .iter()
                .all(|address: &SocketAddr| allowed_address(address.ip(), &local)))
        },
        FetchOptions::new(DEADLINE, kind),
    )
    .await
}

async fn resolve_host(host: String, port: u16) -> io::Result<Vec<SocketAddr>> {
    static DNS: Semaphore = Semaphore::const_new(4);
    let permit = DNS.acquire().await.map_err(io::Error::other)?;
    // Windows' resolver task is blocking. Keep its permit until the actual
    // lookup exits, even if the request is cancelled or exceeds its deadline.
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        Ok((host, port).to_socket_addrs()?.take(65).collect())
    })
    .await
    .map_err(|_| io::Error::other("radio DNS unavailable"))?
}

#[cfg(test)]
async fn fetch_with<F, R>(
    url: Url,
    resolve: F,
    permits: fn(IpAddr) -> bool,
    limit: Duration,
) -> io::Result<Response>
where
    F: Fn(String, u16) -> R,
    R: Future<Output = io::Result<Vec<SocketAddr>>>,
{
    fetch_checked(
        url,
        resolve,
        |addresses| Ok(addresses.iter().all(|address| permits(address.ip()))),
        FetchOptions::new(limit, RequestKind::Stream),
    )
    .await
}

async fn fetch_checked<F, R>(
    mut url: Url,
    resolve: F,
    permits: impl Fn(&[SocketAddr]) -> io::Result<bool>,
    options: FetchOptions,
) -> io::Result<Response>
where
    F: Fn(String, u16) -> R,
    R: Future<Output = io::Result<Vec<SocketAddr>>>,
{
    let limit = options.limit;
    let kind = options.kind;
    let mut seen = HashSet::new();
    for hop in 0..=MAX_REDIRECTS {
        if !valid_url(&url)
            || (kind == RequestKind::Directory
                && (url.scheme() != "https"
                    || url.port_or_known_default() != Some(443)
                    || !url.host_str().is_some_and(net::mirror_name)))
        {
            return Err(denied("invalid media URL"));
        }
        if !seen.insert(url.clone()) {
            return Err(denied("radio redirect loop"));
        }
        let port = url
            .port_or_known_default()
            .ok_or_else(|| io::Error::other("missing port"))?;
        let host = url.host().ok_or_else(|| io::Error::other("missing host"))?;
        let addresses = match host {
            Host::Domain(name) => {
                if !remote_hostname(name) {
                    return Err(denied("local hostname"));
                }
                timeout(limit, resolve(name.to_owned(), port))
                    .await
                    .map_err(|_| io::Error::other("radio DNS timeout"))?
                    .map_err(|_| io::Error::other("radio DNS failure"))?
            }
            Host::Ipv4(ip) => vec![SocketAddr::new(ip.into(), port)],
            Host::Ipv6(ip) => vec![SocketAddr::new(ip.into(), port)],
        };
        if addresses.is_empty()
            || addresses.len() > 64
            || addresses.iter().any(|address| address.port() != port)
            || !permits(&addresses)?
        {
            return Err(denied("radio destination rejected"));
        }
        // One client and one approved address per hop. A fresh client cannot reuse a
        // connection opened under an earlier DNS result. The URL still owns TLS SNI
        // and HTTP authority. Any unexpected connector lookup fails closed.
        let client = Client::builder()
            .tls_backend_preconfigured(radio_tls(&options)?)
            .dns_resolver(PinnedAddress {
                host: url.host_str().unwrap_or_default().to_owned(),
                address: addresses[0],
            })
            .no_proxy()
            .user_agent(concat!("ECHO/", env!("CARGO_PKG_VERSION")))
            .redirect(redirect::Policy::none())
            .retry(reqwest::retry::never())
            .http1_only()
            .http1_max_headers(32)
            .pool_max_idle_per_host(0)
            .connect_timeout(limit)
            .no_gzip()
            .no_brotli()
            .no_deflate()
            .no_zstd()
            .build()
            .map_err(|_| io::Error::other("radio TLS setup failed"))?;
        let request = client.get(url.clone());
        let request = if kind == RequestKind::Stream {
            request.header("icy-metadata", "1")
        } else {
            request
        };
        let response = timeout(limit, request.send())
            .await
            .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "radio header timeout"))?
            .map_err(connection_failure)?;
        match response.status().as_u16() {
            301 | 302 | 303 | 307 | 308 if hop < MAX_REDIRECTS => {
                let locations = response.headers().get_all(header::LOCATION);
                let mut values = locations.iter();
                let location = values
                    .next()
                    .and_then(|value| value.to_str().ok())
                    .filter(|value| {
                        !value.is_empty() && value.len() <= MAX_URL_BYTES && values.next().is_none()
                    })
                    .ok_or_else(|| denied("invalid radio redirect"))?;
                let next = url
                    .join(location)
                    .map_err(|_| denied("invalid radio redirect"))?;
                if url.scheme() == "https" && next.scheme() != "https" {
                    return Err(denied("radio TLS downgrade"));
                }
                url = next;
            }
            200 if response
                .headers()
                .get_all(header::CONTENT_ENCODING)
                .iter()
                .all(|value| value.as_bytes().eq_ignore_ascii_case(b"identity")) =>
            {
                return Ok(response);
            }
            301 | 302 | 303 | 307 | 308 => return Err(denied("radio redirect limit")),
            _ => return Err(io::Error::other("radio HTTP status rejected")),
        }
    }
    Err(denied("radio redirect limit"))
}

fn denied(reason: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, reason)
}

fn connection_failure(error: reqwest::Error) -> io::Error {
    if error.is_timeout() {
        return io::Error::new(io::ErrorKind::TimedOut, "radio header timeout");
    }
    let mut cause: Option<&(dyn std::error::Error + 'static)> = Some(&error);
    while let Some(current) = cause {
        if matches!(
            current.downcast_ref::<rustls::Error>(),
            Some(rustls::Error::InvalidCertificate(_) | rustls::Error::NoCertificatesPresented)
        ) {
            return denied("radio TLS certificate rejected");
        }
        cause = current
            .downcast_ref::<io::Error>()
            .and_then(io::Error::get_ref)
            .map(|cause| cause as &dyn std::error::Error)
            .or_else(|| current.source());
    }
    io::Error::other("radio connection failed")
}

async fn bounded_body(url: Url, kind: RequestKind, cap: usize) -> io::Result<Vec<u8>> {
    timeout(DEADLINE * 3, async {
        let mut response = fetch(url, kind).await?;
        let mut body = Vec::new();
        while let Some(chunk) = next_chunk(&mut response, DEADLINE).await? {
            let chunk = chunk.as_ref();
            if chunk.len() > cap.saturating_sub(body.len()) {
                return Err(io::Error::other("radio response too large"));
            }
            body.extend_from_slice(chunk);
        }
        Ok(body)
    })
    .await
    .map_err(|_| io::Error::other("radio response deadline exceeded"))?
}

async fn next_chunk(
    response: &mut Response,
    deadline: Duration,
) -> io::Result<Option<impl AsRef<[u8]> + use<>>> {
    // Measure a pending read only. A response-level timer also runs while the
    // producer is awaiting queue space from the decoder, which is healthy
    // backpressure and can last longer than the socket idle deadline.
    timeout(deadline, response.chunk())
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "radio body idle"))?
        .map_err(|_| io::Error::other("radio body interrupted"))
}

/// A bounded read-only stream handed to a synchronous decoder thread.
pub struct RadioStream {
    receiver: mpsc::Receiver<io::Result<Option<Vec<u8>>>>,
    pending: std::io::Cursor<Vec<u8>>,
    task: JoinHandle<()>,
    stopped: Arc<AtomicBool>,
    eof: bool,
    info: StreamStatus,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TransportFailure {
    Policy,
    Unsupported,
    Interrupted,
}

#[derive(Clone, Default)]
pub(crate) struct StreamStatus(Arc<Mutex<StreamReport>>);

#[derive(Clone, Default)]
pub(crate) struct StreamReport {
    pub bytes_ready: bool,
    pub title: Option<String>,
    pub failure: Option<TransportFailure>,
}

impl StreamStatus {
    pub(crate) fn read(&self) -> StreamReport {
        self.0
            .lock()
            .map_or_else(|_| StreamReport::default(), |report| report.clone())
    }
}

impl RadioStream {
    /// Starts networking on the supplied async runtime. Reads belong on a native worker.
    pub fn start(runtime: &Handle, source: &str) -> io::Result<Self> {
        if source.len() > MAX_URL_BYTES {
            return Err(io::Error::other("invalid media URL"));
        }
        let url = Url::parse(source).map_err(|_| io::Error::other("invalid media URL"))?;
        if !valid_url(&url) {
            return Err(io::Error::other("invalid media URL"));
        }
        Ok(Self::spawn(runtime, fetch(url, RequestKind::Stream)))
    }

    pub(crate) fn status(&self) -> StreamStatus {
        self.info.clone()
    }

    fn spawn(
        runtime: &Handle,
        fetch: impl Future<Output = io::Result<Response>> + Send + 'static,
    ) -> Self {
        let (sender, receiver) = mpsc::channel(BUFFER_CHUNKS);
        let info = StreamStatus::default();
        let report = info.0.clone();
        let task = runtime.spawn(async move {
            let result = async {
                let mut response = fetch.await?;
                let content = response
                    .headers()
                    .get(header::CONTENT_TYPE)
                    .and_then(|value| value.to_str().ok())
                    .unwrap_or("")
                    .split(';')
                    .next()
                    .unwrap_or("")
                    .trim()
                    .to_ascii_lowercase();
                if !(content.is_empty()
                    || matches!(
                        content.as_str(),
                        "audio/mpeg"
                            | "audio/mp3"
                            | "audio/aac"
                            | "audio/aacp"
                            | "audio/x-aac"
                            | "application/octet-stream"
                    ))
                {
                    return Err(io::Error::new(
                        io::ErrorKind::Unsupported,
                        "unsupported radio content",
                    ));
                }
                let mut metadata = response
                    .headers()
                    .get("icy-metaint")
                    .map(|value| icy::Icy::new(value.to_str().unwrap_or("")))
                    .transpose()?;
                while let Some(bytes) = next_chunk(&mut response, DEADLINE).await? {
                    for slice in bytes.as_ref().chunks(CHUNK_BYTES) {
                        let (audio, title) = if let Some(icy) = &mut metadata {
                            icy.strip(slice)
                        } else {
                            (slice.to_vec(), None)
                        };
                        if let Ok(mut state) = report.lock() {
                            state.bytes_ready |= !audio.is_empty();
                            if let Some(title) = title {
                                state.title = (!title.is_empty()).then_some(title);
                            }
                        }
                        if !audio.is_empty() && sender.send(Ok(Some(audio))).await.is_err() {
                            return Ok(());
                        }
                    }
                }
                Ok(())
            }
            .await;
            if let Err(error) = &result
                && let Ok(mut state) = report.lock()
            {
                state.failure = Some(match error.kind() {
                    io::ErrorKind::PermissionDenied => TransportFailure::Policy,
                    io::ErrorKind::Unsupported | io::ErrorKind::InvalidData => {
                        TransportFailure::Unsupported
                    }
                    _ => TransportFailure::Interrupted,
                });
            }
            let _ = sender.send(result.map(|()| None)).await;
        });
        Self {
            receiver,
            pending: Default::default(),
            task,
            stopped: Default::default(),
            eof: false,
            info,
        }
    }

    /// Cancels networking and wakes a decoder blocked waiting for bytes.
    pub fn cancellation(&self) -> impl Fn() + Send + Sync + use<> {
        let abort = self.task.abort_handle();
        let stopped = Arc::clone(&self.stopped);
        move || {
            stopped.store(true, Ordering::Release);
            abort.abort();
        }
    }
}

impl Read for RadioStream {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if out.is_empty() {
            return Ok(0);
        }
        loop {
            if self.stopped.load(Ordering::Acquire) {
                return Err(io::Error::other("radio cancelled"));
            }
            let len = self.pending.read(out)?;
            if len > 0 || self.eof {
                return Ok(len);
            }
            match self.receiver.blocking_recv().transpose()? {
                Some(Some(bytes)) => self.pending = std::io::Cursor::new(bytes),
                Some(None) => self.eof = true,
                None => return Err(io::Error::other("radio stream closed")),
            }
        }
    }
}

impl Drop for RadioStream {
    fn drop(&mut self) {
        self.task.abort();
    }
}
mod protocol;

pub use protocol::{RadioProtocol, StreamInfo};

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{BufRead, BufReader, Write},
        net::TcpListener,
        path::Path,
        process::{Command, Stdio},
        thread,
    };

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
    }

    fn fixture(reply: impl AsRef<[u8]> + Send + 'static) -> (SocketAddr, thread::JoinHandle<()>) {
        fixture_at("127.0.0.1", reply)
    }

    fn fixture_at(
        ip: &str,
        reply: impl AsRef<[u8]> + Send + 'static,
    ) -> (SocketAddr, thread::JoinHandle<()>) {
        let listener = TcpListener::bind((ip, 0)).unwrap();
        let addr = listener.local_addr().unwrap();
        listener.set_nonblocking(true).unwrap();
        let worker = thread::spawn(move || {
            let deadline = std::time::Instant::now() + Duration::from_secs(3);
            let mut socket = loop {
                match listener.accept() {
                    Ok((socket, _)) => break socket,
                    Err(err)
                        if err.kind() == io::ErrorKind::WouldBlock
                            && std::time::Instant::now() < deadline =>
                    {
                        thread::sleep(Duration::from_millis(5))
                    }
                    Err(err) => panic!("{err}"),
                }
            };
            // Windows can inherit the listener's nonblocking mode on accept.
            socket.set_nonblocking(false).unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            socket
                .set_write_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut input = Vec::new();
            while !input.ends_with(b"\r\n\r\n") && input.len() < 4096 {
                let mut byte = [0];
                socket.read_exact(&mut byte).unwrap();
                input.push(byte[0]);
            }
            assert!(
                String::from_utf8_lossy(&input)
                    .contains(&format!("host: fixture.invalid:{}\r\n", addr.port()))
            );
            socket.write_all(reply.as_ref()).unwrap();
        });
        (addr, worker)
    }

    #[test]
    fn pins_connector_and_preserves_authority() {
        let rt = runtime();
        let (addr, worker) = fixture(b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\n\r\nabc");
        let url = Url::parse(&format!("http://fixture.invalid:{}/live", addr.port())).unwrap();
        rt.block_on(async {
            let mut response = fetch_with(
                url,
                |_, _| async { Ok(vec![addr]) },
                |ip| ip.is_loopback(),
                DEADLINE,
            )
            .await
            .unwrap();
            assert_eq!(&response.chunk().await.unwrap().unwrap()[..], b"abc");
        });
        worker.join().unwrap();
    }

    #[test]
    fn pins_ipv6_at_the_real_connector() {
        let (addr, worker) = fixture_at("::1", b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\n\r\nabc");
        let url = Url::parse(&format!("http://fixture.invalid:{}/live", addr.port())).unwrap();
        let response = runtime()
            .block_on(fetch_with(
                url,
                |_, _| async { Ok(vec![addr]) },
                |ip| ip.is_loopback(),
                DEADLINE,
            ))
            .unwrap();
        assert_eq!(response.remote_addr(), Some(addr));
        worker.join().unwrap();
    }

    #[test]
    fn rejects_a_production_local_destination_before_dial() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let addresses = net::local_addresses().unwrap();
        assert!(addresses.contains(&listener.local_addr().unwrap().ip()));
        let address = listener.local_addr().unwrap();
        let url = Url::parse(&format!("http://{address}/live")).unwrap();
        assert_eq!(
            runtime()
                .block_on(fetch(url, RequestKind::Stream))
                .unwrap_err()
                .kind(),
            io::ErrorKind::PermissionDenied
        );
        assert_eq!(
            listener.accept().unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );
    }

    #[test]
    fn rejects_global_addresses_present_in_the_local_table() {
        let local: HashSet<IpAddr> = ["8.8.8.8", "2606:4700::1111"]
            .into_iter()
            .map(|ip| ip.parse().unwrap())
            .collect();
        for ip in ["8.8.8.8", "::ffff:8.8.8.8", "2606:4700::1111"] {
            assert!(!allowed_address(ip.parse().unwrap(), &local));
        }
        assert!(allowed_address("8.8.4.4".parse().unwrap(), &local));
    }

    #[test]
    fn mixed_public_and_forbidden_dns_answers_never_dial() {
        let trap = TcpListener::bind("127.0.0.1:0").unwrap();
        trap.set_nonblocking(true).unwrap();
        let forbidden = trap.local_addr().unwrap();
        let public = SocketAddr::from(([8, 8, 8, 8], forbidden.port()));
        let url = Url::parse(&format!("http://fixture.invalid:{}/live", forbidden.port())).unwrap();
        let result = runtime().block_on(fetch_with(
            url,
            |_, _| async { Ok(vec![public, forbidden]) },
            public_address,
            DEADLINE,
        ));
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(trap.accept().unwrap_err().kind(), io::ErrorKind::WouldBlock);
    }

    #[test]
    fn forbidden_redirect_literal_receives_no_connection() {
        let trap = TcpListener::bind("127.0.0.2:0").unwrap();
        trap.set_nonblocking(true).unwrap();
        let (addr, worker) = fixture(format!(
            "HTTP/1.1 302 Found\r\nLocation: http://{}/other\r\nContent-Length: 0\r\n\r\n",
            trap.local_addr().unwrap(),
        ));
        let url = Url::parse(&format!("http://fixture.invalid:{}/live", addr.port())).unwrap();
        let result = runtime().block_on(fetch_with(
            url,
            |_, _| async { Ok(vec![addr]) },
            |ip| ip == IpAddr::from([127, 0, 0, 1]),
            DEADLINE,
        ));
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(trap.accept().unwrap_err().kind(), io::ErrorKind::WouldBlock);
        worker.join().unwrap();
    }

    #[test]
    fn does_not_use_the_environment_proxy() {
        let trap = TcpListener::bind("127.0.0.1:0").unwrap();
        trap.set_nonblocking(true).unwrap();
        let proxy = format!("http://{}", trap.local_addr().unwrap());
        let status = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "radio::tests::ambient_proxy_worker", "--ignored"])
            .env("HTTP_PROXY", &proxy)
            .env("HTTPS_PROXY", &proxy)
            .env("ALL_PROXY", &proxy)
            .env_remove("NO_PROXY")
            .status()
            .unwrap();
        assert!(status.success());
        assert_eq!(trap.accept().unwrap_err().kind(), io::ErrorKind::WouldBlock);
    }

    #[test]
    #[ignore = "the parent configures a process-local trap proxy"]
    fn ambient_proxy_worker() {
        pins_connector_and_preserves_authority();
    }

    #[test]
    fn expires_stalled_headers_and_stalled_body() {
        for prefix in [
            &b"HTTP/1.1 200 OK\r\n"[..],
            &b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\n\r\na"[..],
        ] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let addr = listener.local_addr().unwrap();
            let (finish, wait) = std::sync::mpsc::channel::<()>();
            let worker = thread::spawn(move || {
                let (mut socket, _) = listener.accept().unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut request = Vec::new();
                while !request.ends_with(b"\r\n\r\n") && request.len() < 4096 {
                    let mut byte = [0];
                    socket.read_exact(&mut byte).unwrap();
                    request.push(byte[0]);
                }
                socket.write_all(prefix).unwrap();
                let _ = wait.recv_timeout(Duration::from_secs(3));
            });
            let url = Url::parse(&format!("http://fixture.invalid:{}/", addr.port())).unwrap();
            let result = runtime().block_on(async {
                let mut response = fetch_with(
                    url,
                    |_, _| async { Ok(vec![addr]) },
                    |ip| ip.is_loopback(),
                    Duration::from_millis(750),
                )
                .await?;
                let idle = Duration::from_millis(750);
                assert!(next_chunk(&mut response, idle).await?.is_some());
                let error = next_chunk(&mut response, idle).await.err().unwrap();
                assert_eq!(error.kind(), io::ErrorKind::TimedOut);
                Ok::<_, io::Error>(())
            });
            let _ = finish.send(());
            worker.join().unwrap();
            if prefix.len() > 24 {
                result.unwrap();
            } else {
                assert_eq!(result.unwrap_err().kind(), io::ErrorKind::TimedOut);
            }
        }
    }

    #[test]
    fn rejects_a_redirect_beyond_the_hop_limit() {
        let trap = TcpListener::bind("127.0.0.1:0").unwrap();
        trap.set_nonblocking(true).unwrap();
        let mut port = trap.local_addr().unwrap().port();
        let mut workers = Vec::new();
        for _ in 0..=MAX_REDIRECTS {
            let (addr, worker) = fixture(format!(
                "HTTP/1.1 302 Found\r\nLocation: http://fixture.invalid:{port}/live\r\nContent-Length: 0\r\n\r\n"
            ));
            port = addr.port();
            workers.push(worker);
        }
        let url = Url::parse(&format!("http://fixture.invalid:{port}/live")).unwrap();
        let result = runtime().block_on(fetch_with(
            url,
            |_, port| async move { Ok(vec![SocketAddr::from(([127, 0, 0, 1], port))]) },
            |ip| ip.is_loopback(),
            DEADLINE,
        ));
        assert_eq!(result.unwrap_err().to_string(), "radio redirect limit");
        assert_eq!(trap.accept().unwrap_err().kind(), io::ErrorKind::WouldBlock);
        for worker in workers {
            worker.join().unwrap();
        }
    }

    #[test]
    fn verifies_tls_trust_hostname_and_downgrade() {
        let trap = TcpListener::bind("127.0.0.1:0").unwrap();
        trap.set_nonblocking(true).unwrap();
        for (trusted, hostname, downgrade) in [
            (true, "fixture.invalid", false),
            (false, "fixture.invalid", false),
            (true, "elsewhere.invalid", false),
            (true, "fixture.invalid", true),
        ] {
            let temp = tempfile::tempdir().unwrap();
            let cert = temp.path().join("ca.der");
            let script =
                Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/fixtures/tls-radio.ps1");
            let mut fixture = Command::new("pwsh")
                .args(["-NoProfile", "-NonInteractive", "-File"])
                .arg(script)
                .arg("-Certificate")
                .arg(&cert)
                .arg("-RedirectPort")
                .arg(
                    if downgrade {
                        trap.local_addr().unwrap().port()
                    } else {
                        0
                    }
                    .to_string(),
                )
                .stdout(Stdio::piped())
                .spawn()
                .unwrap();
            let mut output = BufReader::new(fixture.stdout.take().unwrap());
            let mut port = String::new();
            output.read_line(&mut port).unwrap();
            let port: u16 = port.trim().parse().expect("TLS fixture did not start");
            let address = SocketAddr::from(([127, 0, 0, 1], port));
            let mut options = FetchOptions::new(DEADLINE, RequestKind::Stream);
            if trusted {
                options.test_root = Some(std::fs::read(cert).unwrap().into());
            }
            let url = Url::parse(&format!("https://{hostname}:{port}/live")).unwrap();
            let result = runtime().block_on(async {
                let mut response = fetch_checked(
                    url,
                    |_, _| async { Ok(vec![address]) },
                    |_| Ok(true),
                    options,
                )
                .await?;
                Ok::<_, io::Error>(response.chunk().await.unwrap().unwrap())
            });
            assert!(fixture.wait().unwrap().success());
            if downgrade {
                assert_eq!(result.unwrap_err().to_string(), "radio TLS downgrade");
            } else if trusted && hostname == "fixture.invalid" {
                assert_eq!(result.unwrap().as_ref(), b"abc");
            } else {
                assert_eq!(
                    result.unwrap_err().to_string(),
                    "radio TLS certificate rejected"
                );
            }
        }
        assert_eq!(trap.accept().unwrap_err().kind(), io::ErrorKind::WouldBlock);
    }

    #[test]
    fn mixed_addresses_reject_before_dial() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        listener.set_nonblocking(true).unwrap();
        let addr = listener.local_addr().unwrap();
        let url = Url::parse(&format!("http://fixture.invalid:{}/live", addr.port())).unwrap();
        let result = runtime().block_on(fetch_with(
            url,
            |_, _| async {
                Ok(vec![
                    addr,
                    SocketAddr::new("::1".parse().unwrap(), addr.port()),
                ])
            },
            |ip| ip == IpAddr::from([127, 0, 0, 1]),
            DEADLINE,
        ));
        assert_eq!(
            result.unwrap_err().to_string(),
            "radio destination rejected"
        );
        assert_eq!(
            listener.accept().unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );
    }

    #[test]
    fn redirect_rechecks_dns_and_rejects_changed_answer() {
        let rt = runtime();
        let (addr, worker) =
            fixture(b"HTTP/1.1 302 Found\r\nLocation: /other\r\nContent-Length: 0\r\n\r\n");
        let forbidden = TcpListener::bind(("127.0.0.2", addr.port())).unwrap();
        forbidden.set_nonblocking(true).unwrap();
        let changed = forbidden.local_addr().unwrap();
        let called = std::cell::Cell::new(0);
        let url = Url::parse(&format!("http://fixture.invalid:{}/live", addr.port())).unwrap();
        let result = rt.block_on(fetch_with(
            url,
            |_, _| {
                let count = called.get();
                called.set(count + 1);
                async move { Ok(vec![if count == 0 { addr } else { changed }]) }
            },
            |ip| ip == IpAddr::from([127, 0, 0, 1]),
            DEADLINE,
        ));
        assert_eq!(
            result.unwrap_err().to_string(),
            "radio destination rejected"
        );
        assert_eq!(called.get(), 2);
        assert_eq!(
            forbidden.accept().unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );
        worker.join().unwrap();
    }

    #[test]
    fn rejects_special_addresses_and_url_credentials() {
        for ip in [
            "::1",
            "::ffff:127.0.0.1",
            "::ffff:169.254.169.254",
            "fe80::1",
            "fc00::1",
            "2001:db8::1",
            "10.0.1.2",
            "100.64.0.1",
            "192.0.2.4",
        ] {
            assert!(!public_address(ip.parse().unwrap()), "{ip}");
        }
        for ip in ["8.8.8.8", "2606:4700::1111", "::ffff:8.8.8.8"] {
            assert!(public_address(ip.parse().unwrap()), "{ip}");
        }
        for url in [
            "file:///local",
            "http://user@public.example/",
            "https://public.example/#frag",
            "http://public.example:0/",
        ] {
            assert!(!valid_url(&Url::parse(url).unwrap()), "{url}");
        }
    }

    #[test]
    fn cancellation_unblocks_waiting_decoder() {
        let rt = runtime();
        rt.block_on(async {
            let mut stream = RadioStream::spawn(rt.handle(), std::future::pending());
            let cancel = stream.cancellation();
            let (started_tx, started_rx) = tokio::sync::oneshot::channel();
            let reader = tokio::task::spawn_blocking(move || {
                started_tx.send(()).unwrap();
                stream.read(&mut [0; 4])
            });
            started_rx.await.unwrap();
            cancel();
            let error = timeout(Duration::from_secs(1), reader)
                .await
                .unwrap()
                .unwrap()
                .unwrap_err();
            assert!(matches!(
                error.to_string().as_str(),
                "radio cancelled" | "radio stream closed"
            ));
        });
    }

    #[test]
    fn rejects_http_errors_and_reports_truncated_body() {
        let rt = runtime();
        for reply in [
            &b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\n\r\n"[..],
            &b"invalid HTTP\r\n\r\n"[..],
        ] {
            let (addr, worker) = fixture(reply);
            let url = Url::parse(&format!("http://fixture.invalid:{}/", addr.port())).unwrap();
            assert!(
                rt.block_on(fetch_with(
                    url,
                    |_, _| async { Ok(vec![addr]) },
                    |ip| ip.is_loopback(),
                    DEADLINE
                ))
                .is_err()
            );
            worker.join().unwrap();
        }
        let (addr, worker) = fixture(b"HTTP/1.1 200 OK\r\nContent-Length: 8\r\n\r\nabc");
        let url = Url::parse(&format!("http://fixture.invalid:{}/", addr.port())).unwrap();
        rt.block_on(async {
            let mut body = fetch_with(
                url,
                |_, _| async { Ok(vec![addr]) },
                |ip| ip.is_loopback(),
                DEADLINE,
            )
            .await
            .unwrap();
            loop {
                match body.chunk().await {
                    Ok(Some(_)) => {}
                    Ok(None) => panic!("truncated response reported clean EOF"),
                    Err(_) => break,
                }
            }
        });
        worker.join().unwrap();
    }

    #[test]
    fn rejects_redirect_loop_without_a_second_resolution() {
        let rt = runtime();
        let (addr, worker) =
            fixture(b"HTTP/1.1 302 Found\r\nLocation: /live\r\nContent-Length: 0\r\n\r\n");
        let url = Url::parse(&format!("http://fixture.invalid:{}/live", addr.port())).unwrap();
        let calls = std::cell::Cell::new(0);
        let result = rt.block_on(fetch_with(
            url,
            |_, _| {
                calls.set(calls.get() + 1);
                async { Ok(vec![addr]) }
            },
            |ip| ip.is_loopback(),
            DEADLINE,
        ));
        assert_eq!(result.unwrap_err().to_string(), "radio redirect loop");
        assert_eq!(calls.get(), 1);
        worker.join().unwrap();
    }

    #[test]
    fn stops_a_backpressured_fetch() {
        let rt = runtime();
        let body_bytes = CHUNK_BYTES * (BUFFER_CHUNKS + 4);
        let mut reply =
            format!("HTTP/1.1 200 OK\r\nContent-Length: {body_bytes}\r\n\r\n").into_bytes();
        reply.resize(reply.len() + body_bytes, 0);
        let (addr, worker) = fixture(reply);
        let url = Url::parse(&format!("http://fixture.invalid:{}/", addr.port())).unwrap();
        let mut stream = RadioStream::spawn(
            rt.handle(),
            fetch_with(
                url,
                move |_, _| async move { Ok(vec![addr]) },
                |ip| ip.is_loopback(),
                DEADLINE,
            ),
        );
        rt.block_on(async {
            timeout(Duration::from_secs(2), async {
                while stream.receiver.len() != BUFFER_CHUNKS {
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
            })
            .await
            .unwrap();
            stream.cancellation()();
            assert!(
                timeout(Duration::from_secs(1), &mut stream.task)
                    .await
                    .unwrap()
                    .unwrap_err()
                    .is_cancelled()
            );
        });
        assert!(stream.read(&mut [0; 1]).is_err());
        worker.join().unwrap();
    }

    #[test]
    fn backpressure_does_not_use_up_the_idle_deadline() {
        let rt = runtime();
        let body_bytes = CHUNK_BYTES * (BUFFER_CHUNKS + 8);
        let mut reply =
            format!("HTTP/1.1 200 OK\r\nContent-Length: {body_bytes}\r\n\r\n").into_bytes();
        reply.resize(reply.len() + body_bytes, 42);
        let (addr, server) = fixture(reply);
        let url = Url::parse(&format!("http://fixture.invalid:{}/", addr.port())).unwrap();
        let stream = RadioStream::spawn(
            rt.handle(),
            fetch_with(
                url,
                move |_, _| async move { Ok(vec![addr]) },
                |ip| ip.is_loopback(),
                Duration::from_millis(200),
            ),
        );
        let result = rt.block_on(async {
            let mut stream = stream;
            timeout(Duration::from_secs(2), async {
                while stream.receiver.len() < BUFFER_CHUNKS {
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
            })
            .await
            .unwrap();
            tokio::time::sleep(Duration::from_millis(250)).await;
            tokio::task::spawn_blocking(move || {
                let mut bytes = Vec::new();
                stream.read_to_end(&mut bytes).unwrap();
                bytes
            })
            .await
            .unwrap()
        });
        assert_eq!(result.len(), body_bytes);
        assert!(result.iter().all(|value| *value == 42));
        server.join().unwrap();
    }
}
