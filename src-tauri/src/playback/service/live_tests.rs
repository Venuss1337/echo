use super::*;
use crate::radio::fixtures::{Reply as Fixture, Server};
use std::{net::TcpListener, path::Path};

fn wait(service: &Service, ready: impl Fn(&PlayerView) -> bool) -> PlayerView {
    let deadline = Instant::now() + Duration::from_secs(12);
    loop {
        let view = request(service, Command::Snapshot);
        if ready(&view) {
            return view;
        }
        assert!(Instant::now() < deadline, "{view:?}");
        thread::sleep(Duration::from_millis(20));
    }
}

fn request(service: &Service, command: Command) -> PlayerView {
    tauri::async_runtime::block_on(async {
        tokio::time::timeout(Duration::from_secs(8), service.request(command))
            .await
            .unwrap()
            .unwrap()
    })
}

fn tune(service: &Service, station: &str, server: &Server) {
    let station = Station {
        id: station.into(),
        name: station.into(),
        country: String::new(),
        language: String::new(),
        tags: String::new(),
        codec: String::new(),
        bitrate: 0,
        reported_online: None,
        manual: true,
        favorite: false,
        artwork: false,
    };
    request(
        service,
        Command::PlayFixtureStation {
            station,
            address: server.address,
        },
    );
}

fn status(view: &PlayerView, expected: LiveState) -> bool {
    view.live
        .as_ref()
        .is_some_and(|live| live.status == expected)
}

fn reviewed_decoder() -> Arc<NativeLibrary> {
    let root = PathBuf::from(std::env::var_os("ECHO_LIVE_RESOURCE_ROOT").unwrap());
    let manifest = std::fs::read_to_string(root.join("dlls.json")).unwrap();
    // SAFETY: test-live.ps1 supplies the staged native resource root.
    Arc::new(unsafe { NativeLibrary::open(&root, &manifest) }.unwrap())
}

#[test]
#[ignore = "requires the reviewed network-disabled native DLL; see test-live.ps1"]
fn native_live_radio() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("library.sqlite3");
    let service = Service::start(reviewed_decoder(), db, Output::Null).unwrap();
    let wav = temp.path().join("original tone.wav");
    std::fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../.echo-fixtures/transport.wav"),
        &wav,
    )
    .unwrap();
    request(&service, Command::AddPicked(vec![wav], PickMode::Listen));
    let local = wait(&service, |view| {
        view.state == PlayerState::Playing && view.position_ms > 0.0
    });
    let selected = EntryId::parse(local.current.as_deref().unwrap()).unwrap();
    request(&service, Command::Volume(34));
    request(&service, Command::Mute(true));

    let mp3 = Server::start(
        Fixture::Loop(include_bytes!("../../../../tests/fixtures/audio/tone.mp3").to_vec()),
        "audio/mpeg",
    );
    let start = Instant::now();
    tune(&service, "Fixture MP3", &mp3);
    let playing = wait(&service, |view| status(view, LiveState::Playing));
    println!(
        "MP3 native playback restart confirmed after {} ms",
        start.elapsed().as_millis()
    );
    assert_eq!(playing.volume, 34);
    assert!(playing.muted);
    assert!(!playing.seekable);
    assert_eq!(playing.duration_ms, None);
    assert_eq!(playing.position_ms, 0.0);
    assert_eq!(playing.tracks[0].id, local.tracks[0].id);
    assert!(playing.generation.parse::<u64>().unwrap() > local.generation.parse::<u64>().unwrap());

    let paused = request(&service, Command::Pause(true));
    assert!(status(&paused, LiveState::Paused));
    assert_eq!(mp3.connections(), 1);
    request(&service, Command::Pause(false));
    let resumed = wait(&service, |view| status(view, LiveState::Playing));
    assert_ne!(resumed.generation, playing.generation);
    assert_eq!(mp3.connections(), 2);

    let aac = Server::start(
        Fixture::Loop(include_bytes!("../../../../tests/fixtures/audio/tone.aac").to_vec()),
        "audio/aac",
    );
    let start = Instant::now();
    tune(&service, "Fixture AAC", &aac);
    wait(&service, |view| status(view, LiveState::Playing));
    println!(
        "AAC native playback restart confirmed after {} ms",
        start.elapsed().as_millis()
    );
    let replacement = request(&service, Command::Play(selected));
    assert!(replacement.live.is_none());
    wait(&service, |view| {
        view.state == PlayerState::Playing && view.current == local.current
    });

    let recovered = Server::start(
        Fixture::Recover(include_bytes!("../../../../tests/fixtures/audio/tone.mp3").to_vec()),
        "audio/mpeg",
    );
    tune(&service, "Recovery", &recovered);
    wait(&service, |view| status(view, LiveState::Reconnecting));
    wait(&service, |view| status(view, LiveState::Playing));
    assert_eq!(recovered.connections(), 2);

    let broken = Server::start(Fixture::Interrupted, "audio/mpeg");
    tune(&service, "Interrupted", &broken);
    wait(&service, |view| status(view, LiveState::Reconnecting));
    wait(&service, |view| status(view, LiveState::Offline));
    assert_eq!(broken.connections(), 4);
    assert!(request(&service, Command::Stop).live.is_none());

    let unsupported = Server::start(Fixture::Finite(b"plain text".to_vec()), "text/plain");
    tune(&service, "Unsupported", &unsupported);
    wait(&service, |view| status(view, LiveState::Unsupported));
    assert_eq!(unsupported.connections(), 1);

    let trap = TcpListener::bind("127.0.0.1:0").unwrap();
    trap.set_nonblocking(true).unwrap();
    let playlist = format!(
        "#EXTM3U\nhttp://{}/never-open\n",
        trap.local_addr().unwrap()
    );
    // A mistaken MIME type still cannot turn a playlist into another open.
    let hostile = Server::start(Fixture::Finite(playlist.into_bytes()), "audio/mpeg");
    tune(&service, "Playlist", &hostile);
    wait(&service, |view| status(view, LiveState::Unsupported));
    assert_eq!(
        trap.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );

    let stalled = Server::start(Fixture::Stalled, "audio/aac");
    tune(&service, "Stalled", &stalled);
    let deadline = Instant::now() + Duration::from_secs(1);
    while stalled.connections() == 0 {
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(5));
    }
    let exit = Instant::now();
    service.shutdown().unwrap();
    assert!(
        exit.elapsed() < Duration::from_secs(1),
        "native shutdown took {:?}",
        exit.elapsed()
    );
}

#[test]
#[ignore = "optional process measurement; see test-live.ps1 -Measure"]
fn native_live_measurement() {
    let seconds = std::env::var("ECHO_LIVE_MEASURE_SECONDS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|seconds| (60..=600).contains(seconds))
        .unwrap_or(60);
    let server = Server::start(
        Fixture::Paced(include_bytes!("../../../../tests/fixtures/audio/tone.mp3").to_vec()),
        "audio/mpeg",
    );
    let runtime = tauri::async_runtime::handle();
    let stream = crate::radio::fixtures::stream(runtime.inner(), server.address);
    let transport = stream.status();
    let started = Instant::now();
    let mut decoder = Decoder::start_live(
        reviewed_decoder(),
        stream,
        1,
        StartOptions {
            output: Output::Null,
            volume: 34,
            muted: true,
            resume_at: 0,
        },
    )
    .unwrap();
    loop {
        let (_, event) = decoder.event(Duration::from_millis(25));
        if event == DecoderEvent::Restarted {
            break;
        }
        assert!(started.elapsed() < Duration::from_secs(5), "{event:?}");
    }
    let first = Instant::now();
    let before = decoder.position().unwrap_or_default();
    let mut min = f64::INFINITY;
    let mut max: f64 = 0.0;
    let (cpu_start, _) = process_sample();
    loop {
        let (_, event) = decoder.event(Duration::from_secs(1));
        assert!(
            !matches!(event, DecoderEvent::End { .. } | DecoderEvent::Shutdown),
            "{event:?}, transport failure: {:?}",
            transport.read().failure,
        );
        let (_, memory) = process_sample();
        min = min.min(memory);
        max = max.max(memory);
        if first.elapsed() >= Duration::from_secs(seconds) {
            break;
        }
    }
    let elapsed = first.elapsed();
    let (cpu_end, memory) = process_sample();
    let progress = decoder.position().unwrap() - before;
    assert!(progress >= seconds as f64 * 0.9, "{progress:.1} s decoded");
    assert_eq!(server.connections(), 1);
    let stop = Instant::now();
    drop(decoder);
    println!(
        "{:.1} s MP3, {:.1} s decoded progress, {:.1} ms CPU, private MiB {:.2}–{:.2} (end {:.2}), stop {:.1} ms",
        elapsed.as_secs_f64(),
        progress,
        cpu_end - cpu_start,
        min,
        max,
        memory,
        stop.elapsed().as_secs_f64() * 1000.0,
    );
}

fn process_sample() -> (f64, f64) {
    use windows::Win32::{
        Foundation::FILETIME,
        System::{
            ProcessStatus::{K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS_EX},
            Threading::{GetCurrentProcess, GetProcessTimes},
        },
    };
    let [mut started, mut finished, mut kernel, mut user] = [FILETIME::default(); 4];
    let mut memory = PROCESS_MEMORY_COUNTERS_EX::default();
    memory.cb = std::mem::size_of_val(&memory) as u32;
    // SAFETY: the pseudohandle stays valid, and Win32 uses the given size to
    // recognize the extended form of the writable memory structure.
    unsafe {
        let process = GetCurrentProcess();
        GetProcessTimes(process, &mut started, &mut finished, &mut kernel, &mut user).unwrap();
        assert!(
            K32GetProcessMemoryInfo(process, std::ptr::from_mut(&mut memory).cast(), memory.cb)
                .as_bool()
        );
    }
    let ticks =
        |value: FILETIME| ((value.dwHighDateTime as u64) << 32) | value.dwLowDateTime as u64;
    (
        (ticks(kernel) + ticks(user)) as f64 / 10_000.0,
        memory.PrivateUsage as f64 / (1024.0 * 1024.0),
    )
}
