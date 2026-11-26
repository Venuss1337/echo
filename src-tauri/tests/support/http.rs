//! One-request HTTP fixtures bound to IPv4 loopback with no external DNS.
use std::{
    io::{self, Read, Write},
    net::{TcpListener, TcpStream},
    sync::mpsc::{self, Sender},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

#[derive(Clone, Copy)]
pub enum Response {
    Complete,
    Rejected,
    Redirect(u16),
    Interrupted,
    Cancelled,
}

pub struct Server {
    pub url: String,
    pub port: u16,
    pub interrupt: Sender<()>,
    pub worker: JoinHandle<io::Result<()>>,
}

impl Server {
    pub fn new(response: Response, wav: Vec<u8>) -> Self {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let addr = listener.local_addr().unwrap();
        listener.set_nonblocking(true).unwrap();
        let (interrupt, release) = mpsc::channel();
        let worker = thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(6);
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(error)
                        if error.kind() == io::ErrorKind::WouldBlock
                            && Instant::now() < deadline =>
                    {
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(error) => return Err(error),
                }
            };
            drop(listener);
            stream.set_nonblocking(false)?;
            stream.set_read_timeout(Some(Duration::from_secs(6)))?;
            stream.set_write_timeout(Some(Duration::from_secs(6)))?;
            receive_request(&mut stream)?;

            match response {
                Response::Rejected => stream.write_all(
                    b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                ),
                Response::Redirect(port) => write!(
                    stream,
                    "HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:{port}/fixture.wav\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                ),
                Response::Complete => {
                    stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: audio/wav\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n")?;
                    for chunk in wav.chunks(4096) {
                        write!(stream, "{:x}\r\n", chunk.len())?;
                        stream.write_all(chunk)?;
                        stream.write_all(b"\r\n")?;
                    }
                    stream.write_all(b"0\r\n\r\n")
                }
                Response::Interrupted | Response::Cancelled => {
                    write!(
                        stream,
                        "HTTP/1.1 200 OK\r\nContent-Type: audio/wav\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        wav.len()
                    )?;
                    // Leave the declared response incomplete, then either drop it
                    // on cue or hold the socket until mpv closes it after stop.
                    stream.write_all(&wav[..wav.len() / 2])?;
                    if matches!(response, Response::Interrupted) {
                        release
                            .recv_timeout(Duration::from_secs(6))
                            .map_err(io::Error::other)?;
                        return Ok(());
                    }
                    match stream.read(&mut [0u8]) {
                        Ok(0) => Ok(()),
                        Err(error) if error.kind() == io::ErrorKind::ConnectionReset => Ok(()),
                        Err(error) => Err(error),
                        Ok(_) => Err(io::Error::other("unexpected data on cancelled connection")),
                    }
                }
            }
        });
        Self {
            url: format!("http://{addr}/fixture.wav"),
            port: addr.port(),
            interrupt,
            worker,
        }
    }
}

fn receive_request(stream: &mut TcpStream) -> io::Result<()> {
    let mut request = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(3);
    while request.len() < 8192 && Instant::now() < deadline {
        let mut byte = [0u8; 1];
        stream.read_exact(&mut byte)?;
        request.push(byte[0]);
        if request.ends_with(b"\r\n\r\n") && request.starts_with(b"GET /fixture.wav HTTP/") {
            return Ok(());
        }
    }
    Err(io::Error::other("expected loopback WAV request"))
}
