//! Socket fixture permissions exist only in the Rust unit-test executable.
use super::{RadioStream, fetch_with};
use std::{
    io::{self, Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread::{self, JoinHandle},
    time::Duration,
};
use tokio::runtime::Handle;
use url::Url;

pub(crate) fn stream(runtime: &Handle, address: SocketAddr) -> RadioStream {
    let url = Url::parse(&format!("http://fixture.invalid:{}/stream", address.port())).unwrap();
    RadioStream::spawn(
        runtime,
        fetch_with(
            url,
            move |_, _| async move { Ok(vec![address]) },
            |ip| ip.is_loopback(),
            Duration::from_secs(2),
        ),
    )
}

pub(crate) struct Server {
    pub address: SocketAddr,
    connections: Arc<AtomicUsize>,
    stopping: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

pub(crate) enum Reply {
    Loop(Vec<u8>),
    Paced(Vec<u8>),
    Finite(Vec<u8>),
    Interrupted,
    Recover(Vec<u8>),
    Stalled,
}

impl Server {
    pub(crate) fn start(body: Reply, content_type: &'static str) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        listener.set_nonblocking(true).unwrap();
        let stopping = Arc::new(AtomicBool::new(false));
        let connections = Arc::new(AtomicUsize::new(0));
        let stop = stopping.clone();
        let count = connections.clone();
        let worker = thread::spawn(move || {
            while !stop.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((mut socket, _)) => {
                        let index = count.fetch_add(1, Ordering::Release);
                        let reply = if index == 0 && matches!(body, Reply::Recover(_)) {
                            &Reply::Interrupted
                        } else {
                            &body
                        };
                        let _ = serve(&mut socket, reply, content_type, &stop);
                    }
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(error) => panic!("{error}"),
                }
            }
        });
        Self {
            address,
            connections,
            stopping,
            worker: Some(worker),
        }
    }

    pub(crate) fn connections(&self) -> usize {
        self.connections.load(Ordering::Acquire)
    }
}

fn serve(
    socket: &mut TcpStream,
    reply: &Reply,
    content_type: &str,
    stop: &AtomicBool,
) -> io::Result<()> {
    socket.set_nonblocking(false)?;
    socket.set_read_timeout(Some(Duration::from_millis(500)))?;
    socket.set_write_timeout(Some(Duration::from_millis(500)))?;
    let mut input = Vec::new();
    while !input.ends_with(b"\r\n\r\n") && input.len() < 4096 {
        let mut byte = [0];
        socket.read_exact(&mut byte)?;
        input.push(byte[0]);
    }
    write!(
        socket,
        "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nTransfer-Encoding: chunked\r\n\r\n"
    )?;
    let chunks: Box<dyn Iterator<Item = &[u8]>> = match reply {
        Reply::Interrupted => return socket.write_all(b"10\r\nabc"),
        Reply::Stalled => {
            // The client sends no request body. EOF confirms its connection
            // was closed while the callback was waiting for the first bytes.
            while !stop.load(Ordering::Acquire) {
                match socket.read(&mut [0]) {
                    Ok(0) => break,
                    Err(error) if error.kind() != io::ErrorKind::TimedOut => break,
                    _ => {}
                }
            }
            return Ok(());
        }
        Reply::Finite(audio) => Box::new(audio.chunks(1024)),
        Reply::Loop(audio) | Reply::Paced(audio) | Reply::Recover(audio) => {
            Box::new(audio.chunks(1024).cycle())
        }
    };
    for (index, bytes) in chunks.enumerate() {
        if stop.load(Ordering::Acquire) {
            break;
        }
        write!(socket, "{:x}\r\n", bytes.len())?;
        socket.write_all(bytes)?;
        socket.write_all(b"\r\n")?;
        // The 12-second MP3 is 12,404 bytes. After the initial probe bytes,
        // pace it near its encoded rate for the long-running measurement.
        let delay = if matches!(reply, Reply::Paced(_)) && index >= 12 {
            880
        } else {
            4
        };
        thread::sleep(Duration::from_millis(delay));
    }
    socket.write_all(b"0\r\n\r\n")
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stopping.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            worker.join().unwrap();
        }
    }
}
