#![cfg(all(windows, target_arch = "x86_64"))]

use echo_lib::{
    contract::{PlayerFault, PlayerState, PlayerView},
    native::NativeLibrary,
    playback::{
        decoder::{Decoder, DecoderError, DecoderEvent, Output},
        queue::{EntryId, Repeat},
        service::{Command as PlayerCommand, Service},
    },
};
use std::{
    fs::File,
    io::{self, Write},
    net::TcpListener,
    path::PathBuf,
    process::Command,
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

#[test]
#[ignore = "run against the staged local-only DLL through test-local.ps1"]
fn local_decoder() {
    if std::env::var_os("ECHO_LOCAL_NO_OUTPUT").is_some() {
        run("no_output_worker");
        return;
    }
    if std::env::var_os("ECHO_LOCAL_DEFAULT_OUTPUT").is_some() {
        run("default_output_worker");
        return;
    }
    if std::env::var_os("ECHO_MEASURE_LOCAL").is_some() {
        run("measurement_worker");
        return;
    }
    run("decoder_worker");
    if std::env::var_os("ECHO_LEGACY_NATIVE_ROOT").is_some() {
        run("legacy_worker");
    }
}

fn run(name: &str) {
    let mut worker = Command::new(std::env::current_exe().unwrap())
        .args(["--ignored", "--exact", name, "--nocapture"])
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(35);
    loop {
        if let Some(status) = worker.try_wait().unwrap() {
            assert!(status.success(), "{status}");
            return;
        }
        if Instant::now() >= deadline {
            let _ = worker.kill();
            worker.wait().unwrap();
            panic!("local decoder worker exceeded its deadline");
        }
        thread::sleep(Duration::from_millis(25));
    }
}

fn staged_library() -> Arc<NativeLibrary> {
    let root = PathBuf::from(std::env::var_os("ECHO_LOCAL_NATIVE_ROOT").unwrap());
    let manifest = std::env::var_os("ECHO_LOCAL_NATIVE_MANIFEST")
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join("dlls.json"));
    let manifest = std::fs::read_to_string(manifest).unwrap();
    // SAFETY: the harness receives the exclusively staged native artifact and its manifest.
    Arc::new(unsafe { NativeLibrary::open(&root, &manifest) }.unwrap())
}

#[test]
#[ignore = "internal isolated subprocess"]
fn decoder_worker() {
    let fixtures = PathBuf::from(std::env::var_os("ECHO_LOCAL_NATIVE_FIXTURES").unwrap());
    let library = staged_library();

    for (index, name) in [
        "tone.wav",
        "tone.mp3",
        "tone.flac",
        "tone.ogg",
        "tone.m4a",
        "tone.aac",
    ]
    .iter()
    .enumerate()
    {
        let generation = index as u64 + 1;
        let file = File::open(fixtures.join(name)).unwrap();
        let mut decoder = Decoder::start(library.clone(), file, generation, Output::Null)
            .unwrap_or_else(|error| panic!("{name}: {error:?}"));
        wait(&mut decoder, generation, |event| {
            event == DecoderEvent::Loaded
        });
        wait(&mut decoder, generation, |event| {
            event == DecoderEvent::Restarted
        });
        assert!(decoder.audio_ready(), "{name}");
        assert!(decoder.position().is_some(), "{name}");
        println!("Decoded {name} through approved callback");
        if index == 0 {
            decoder.pause(true).unwrap();
            assert_eq!(decoder.is_paused(), Some(true));
            assert_eq!(decoder.is_seekable(), Some(true));
            decoder.seek(6000).unwrap();
            wait(&mut decoder, generation, |event| {
                event == DecoderEvent::Restarted
            });
            let landed = decoder.position().unwrap();
            assert!(
                (5.75..=6.25).contains(&landed),
                "seek landed at {landed:.3} seconds"
            );
            decoder.volume(42).unwrap();
            decoder.mute(true).unwrap();
            assert_eq!(decoder.mixer_volume(), Some(42.0));
            assert_eq!(decoder.is_muted(), Some(true));
            let end = (decoder.duration().unwrap() * 1000.0) as u64;
            decoder.seek(end - 350).unwrap();
            decoder.pause(false).unwrap();
            wait(&mut decoder, generation, |event| {
                event
                    == DecoderEvent::End {
                        reason: 0,
                        error: 0,
                    }
            });
        } else {
            decoder.stop().unwrap();
            wait(&mut decoder, generation, |event| {
                event
                    == DecoderEvent::End {
                        reason: 2,
                        error: 0,
                    }
            });
        }
    }

    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = listener.local_addr().unwrap();
    for contents in [
        format!("#EXTM3U\nhttp://{address}/tone.wav\n"),
        format!("ffconcat version 1.0\nfile http://{address}/tone.wav\n"),
        format!("[playlist]\nFile1=http://{address}/tone.wav\nNumberOfEntries=1\n"),
        format!("#EXTM3U\n{}\n", fixtures.join("tone.wav").display()),
        format!(
            "[playlist]\nFile1=\\\\{}/share/tone.wav\nNumberOfEntries=1\n",
            address.ip()
        ),
    ] {
        let mut fixture = tempfile::NamedTempFile::new().unwrap();
        fixture.write_all(contents.as_bytes()).unwrap();
        let mut decoder =
            Decoder::start(library.clone(), fixture.reopen().unwrap(), 99, Output::Null).unwrap();
        wait(&mut decoder, 99, |event| {
            matches!(event, DecoderEvent::End { reason: 4, .. })
        });
        drop(decoder);
        assert_eq!(
            listener.accept().unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );
    }
    // Exercise an external data-reference atom in an allowed container too.
    // The synthetic alias is a well-formed MOV record with a secondary path.
    let mut external = tempfile::NamedTempFile::new().unwrap();
    let m4a = std::fs::read(fixtures.join("tone.m4a")).unwrap();
    external
        .write_all(&replace_mov_reference(&m4a, "//127.0.0.1/share/tone.m4a"))
        .unwrap();
    let mut decoder = Decoder::start(
        library.clone(),
        external.reopen().unwrap(),
        100,
        Output::Null,
    )
    .unwrap();
    wait(&mut decoder, 100, |event| {
        matches!(event, DecoderEvent::Loaded | DecoderEvent::End { .. })
    });
    drop(decoder);
    assert_eq!(
        listener.accept().unwrap_err().kind(),
        io::ErrorKind::WouldBlock
    );

    let database = tempfile::tempdir().unwrap();
    let path = database.path().join("player.sqlite3");
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let service = Service::start(library.clone(), path.clone(), Output::Null).unwrap();
    runtime.block_on(async {
        service.request(PlayerCommand::Volume(31)).await.unwrap();
        service
            .request(PlayerCommand::Append(fixtures.join("tone.wav")))
            .await
            .unwrap();
        assert_eq!(
            service.request(PlayerCommand::Next).await.unwrap().state,
            PlayerState::Loading
        );
        let initial = await_view(&service, |view| {
            view.state == PlayerState::Playing && view.position_ms > 0.0
        })
        .await;
        let queued = service
            .request(PlayerCommand::Append(fixtures.join("tone.flac")))
            .await
            .unwrap();
        let second = EntryId::parse(&queued.tracks[1].id).unwrap();
        let first_generation = initial.generation.parse().unwrap();
        service
            .request(PlayerCommand::Seek {
                generation: first_generation,
                milliseconds: initial.duration_ms.unwrap() as u64 - 300,
            })
            .await
            .unwrap();
        let advanced = await_view(&service, |view| {
            view.current.as_deref() == Some(&queued.tracks[1].id)
                && view.state == PlayerState::Playing
        })
        .await;
        assert_eq!(
            advanced.generation.parse::<u64>().unwrap(),
            first_generation + 1
        );
        assert!(
            service
                .request(PlayerCommand::Seek {
                    generation: first_generation,
                    milliseconds: 200
                })
                .await
                .is_err()
        );

        for _ in 0..8 {
            service.request(PlayerCommand::Previous).await.unwrap();
            service.request(PlayerCommand::Play(second)).await.unwrap();
            service.request(PlayerCommand::Stop).await.unwrap();
            await_view(&service, |view| view.state == PlayerState::Idle).await;
        }
        let corrupt = database.path().join("corrupt.wav");
        std::fs::write(&corrupt, b"not a supported audio stream").unwrap();
        let queued = service
            .request(PlayerCommand::Append(corrupt))
            .await
            .unwrap();
        let invalid = EntryId::parse(&queued.tracks[2].id).unwrap();
        service
            .request(PlayerCommand::Repeat(Repeat::One))
            .await
            .unwrap();
        service.request(PlayerCommand::Play(invalid)).await.unwrap();
        let failed = await_view(&service, |view| view.state == PlayerState::Error).await;
        assert!(matches!(failed.failure, Some(PlayerFault::DecodeFailed)));
        let unchanged = service.request(PlayerCommand::Snapshot).await.unwrap();
        assert_eq!(unchanged.generation, failed.generation);
        service.request(PlayerCommand::Mute(true)).await.unwrap();
        service.request(PlayerCommand::Play(second)).await.unwrap();
        await_view(&service, |view| {
            view.state == PlayerState::Playing && view.position_ms > 0.0
        })
        .await;
        assert_eq!(
            service
                .request(PlayerCommand::Pause(true))
                .await
                .unwrap()
                .state,
            PlayerState::Paused
        );
    });
    service.shutdown().unwrap();
    let restored = Service::start(library, path, Output::Null).unwrap();
    let snapshot = runtime
        .block_on(restored.request(PlayerCommand::Snapshot))
        .unwrap();
    assert_eq!(snapshot.state, PlayerState::Idle);
    assert_eq!(snapshot.volume, 31);
    assert!(snapshot.muted);
    assert!(matches!(
        snapshot.repeat,
        echo_lib::contract::RepeatMode::One
    ));
    assert!(snapshot.position_ms > 0.0);
    let persisted_at = snapshot.position_ms;
    runtime.block_on(async {
        let current = EntryId::parse(snapshot.current.as_deref().unwrap()).unwrap();
        let started = restored
            .request(PlayerCommand::Play(current))
            .await
            .unwrap();
        assert_eq!(started.state, PlayerState::Loading);
        let resumed = await_view(&restored, |view| {
            view.state == PlayerState::Playing && view.position_ms > 0.0
        })
        .await;
        assert!(resumed.position_ms >= persisted_at - 250.0);
    });
    restored.shutdown().unwrap();
}

fn replace_mov_reference(m4a: &[u8], url: &str) -> Vec<u8> {
    fn atom(name: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut bytes = (8 + body.len() as u32).to_be_bytes().to_vec();
        bytes.extend(name);
        bytes.extend(body);
        bytes
    }

    fn replace(data: &[u8], parents: &[[u8; 4]], reference: &[u8]) -> Option<Vec<u8>> {
        let mut offset = 0;
        while offset + 8 <= data.len() {
            let header = &data[offset..offset + 8];
            let size = u32::from_be_bytes(header[..4].try_into().ok()?) as usize;
            let end = offset.checked_add(size)?;
            if size < 8 || end > data.len() {
                return None;
            }
            if header[4..] == parents[0] {
                let changed = if parents.len() == 1 {
                    reference.to_vec()
                } else {
                    let body = replace(&data[offset + 8..end], &parents[1..], reference)?;
                    atom(&parents[0], &body)
                };
                return Some([&data[..offset], &changed, &data[end..]].concat());
            }
            offset = end;
        }
        None
    }

    // Offset 162 from the outer atom start follows version/flags and the
    // fixed Finder alias fields read by FFmpeg's mov_read_dref.
    let mut alias = vec![0_u8; 154];
    alias.extend(2_u16.to_be_bytes());
    alias.extend((url.len() as u16).to_be_bytes());
    alias.extend(url.as_bytes());
    if !url.len().is_multiple_of(2) {
        alias.push(0);
    }
    alias.extend([0xff, 0xff, 0, 0]);
    let mut body = vec![0, 0, 0, 0, 0, 0, 0, 1];
    body.extend(atom(b"alis", &alias));
    let reference = atom(b"dref", &body);
    replace(
        m4a,
        &[*b"moov", *b"trak", *b"mdia", *b"minf", *b"dinf", *b"dref"],
        &reference,
    )
    .expect("generated M4A fixture contains a normal data reference")
}

#[test]
#[ignore = "invoke only after Windows Core Audio confirms no active render endpoints"]
fn no_output_worker() {
    let fixtures = PathBuf::from(std::env::var_os("ECHO_LOCAL_NATIVE_FIXTURES").unwrap());
    let database = tempfile::tempdir().unwrap();
    let service = Service::start(
        staged_library(),
        database.path().join("device.sqlite3"),
        Output::Wasapi,
    )
    .unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let queued = service
            .request(PlayerCommand::Append(fixtures.join("tone.wav")))
            .await
            .unwrap();
        let entry = EntryId::parse(&queued.tracks[0].id).unwrap();
        for _ in 0..2 {
            let pending = service.request(PlayerCommand::Play(entry)).await.unwrap();
            assert_eq!(pending.state, PlayerState::Loading);
            let failed = await_view(&service, |view| {
                assert_ne!(view.state, PlayerState::Playing);
                view.state == PlayerState::Error
            })
            .await;
            assert!(matches!(
                failed.failure,
                Some(PlayerFault::OutputUnavailable)
            ));
        }
    });
    service.shutdown().unwrap();
}

async fn await_view(service: &Service, ready: impl Fn(&PlayerView) -> bool) -> PlayerView {
    tokio::time::timeout(Duration::from_secs(4), async {
        loop {
            let view = service.request(PlayerCommand::Snapshot).await.unwrap();
            if ready(&view) {
                return view;
            }
            assert_ne!(view.state, PlayerState::Error, "{view:?}");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("service confirmation exceeded deadline")
}

#[test]
#[ignore = "optional muted WASAPI smoke, requires a Windows default render endpoint"]
fn default_output_worker() {
    let fixtures = PathBuf::from(std::env::var_os("ECHO_LOCAL_NATIVE_FIXTURES").unwrap());
    let database = tempfile::tempdir().unwrap();
    let service = Service::start(
        staged_library(),
        database.path().join("device.sqlite3"),
        Output::Wasapi,
    )
    .unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        service.request(PlayerCommand::Volume(0)).await.unwrap();
        service.request(PlayerCommand::Mute(true)).await.unwrap();
        service
            .request(PlayerCommand::Append(fixtures.join("tone.wav")))
            .await
            .unwrap();
        service.request(PlayerCommand::Next).await.unwrap();
        let confirmed = await_view(&service, |view| {
            view.state == PlayerState::Playing && view.position_ms > 0.0
        })
        .await;
        assert_eq!(confirmed.volume, 0);
        assert!(confirmed.muted);
        service.request(PlayerCommand::Stop).await.unwrap();
        await_view(&service, |view| view.state == PlayerState::Idle).await;
    });
    service.shutdown().unwrap();
    println!("Default WASAPI output opened, advanced while muted, stopped and shut down.");
}

#[test]
#[ignore = "optional isolated native performance measurement"]
fn measurement_worker() {
    let fixtures = PathBuf::from(std::env::var_os("ECHO_LOCAL_NATIVE_FIXTURES").unwrap());
    let database = tempfile::tempdir().unwrap();
    let service = Service::start(
        staged_library(),
        database.path().join("perf.sqlite3"),
        Output::Null,
    )
    .unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let window = Duration::from_secs(5);
    runtime.block_on(async {
        let idle = service.request(PlayerCommand::Snapshot).await.unwrap();
        assert_eq!(idle.state, PlayerState::Idle);
        let idle_start = sample_process();
        tokio::time::sleep(window).await;
        let idle_end = sample_process();
        assert_eq!(service.request(PlayerCommand::Snapshot).await.unwrap().revision, idle.revision);
        service.request(PlayerCommand::Append(fixtures.join("tone.wav"))).await.unwrap();
        service.request(PlayerCommand::Next).await.unwrap();
        let with_audio = await_view(&service, |view| view.state == PlayerState::Playing).await;
        let playing_start = sample_process();
        tokio::time::sleep(window).await;
        let playing_end = sample_process();
        let after = service.request(PlayerCommand::Snapshot).await.unwrap();
        assert_eq!(after.state, PlayerState::Playing);
        let revisions = after.revision.parse::<u64>().unwrap() - with_audio.revision.parse::<u64>().unwrap();
        assert!((9..=12).contains(&revisions), "unexpected update count: {revisions}");
        println!(
            "5 second idle window: {:.2} ms CPU, {:.2} MiB working set, 0 revisions\n\
             5 second WAV playback window: {:.2} ms CPU, {:.2} MiB working set, {revisions} revisions",
            idle_end.0 - idle_start.0, idle_end.1, playing_end.0 - playing_start.0, playing_end.1
        );
    });
    let started = Instant::now();
    service.shutdown().unwrap();
    println!(
        "active source shutdown: {:.2} ms",
        started.elapsed().as_secs_f64() * 1000.0
    );
}

fn sample_process() -> (f64, f64) {
    use windows::Win32::{
        Foundation::FILETIME,
        System::{
            ProcessStatus::{K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS},
            Threading::{GetCurrentProcess, GetProcessTimes},
        },
    };
    let (mut creation, mut exit, mut kernel, mut user) = (
        FILETIME::default(),
        FILETIME::default(),
        FILETIME::default(),
        FILETIME::default(),
    );
    let mut memory = PROCESS_MEMORY_COUNTERS::default();
    memory.cb = std::mem::size_of_val(&memory) as u32;
    // SAFETY: the current process pseudohandle is live and the output pointers
    // refer to correctly sized, initialized values for the duration of each call.
    unsafe {
        let process = GetCurrentProcess();
        GetProcessTimes(process, &mut creation, &mut exit, &mut kernel, &mut user).unwrap();
        assert!(K32GetProcessMemoryInfo(process, &mut memory, memory.cb).as_bool());
    }
    let ticks =
        |time: FILETIME| (u64::from(time.dwHighDateTime) << 32) | u64::from(time.dwLowDateTime);
    (
        (ticks(kernel) + ticks(user)) as f64 / 10_000.0,
        memory.WorkingSetSize as f64 / (1024.0 * 1024.0),
    )
}

#[test]
#[ignore = "internal isolated subprocess"]
fn legacy_worker() {
    let fixtures = PathBuf::from(std::env::var_os("ECHO_LOCAL_NATIVE_FIXTURES").unwrap());
    if let Some(root) = std::env::var_os("ECHO_LEGACY_NATIVE_ROOT") {
        let root = PathBuf::from(root);
        let manifest = std::fs::read_to_string(root.join("dlls.json")).unwrap();
        // SAFETY: this is the previously verified feasibility stage, used only
        // to confirm the adapter rejects its profile before issuing a load.
        let library = Arc::new(unsafe { NativeLibrary::open(&root, &manifest) }.unwrap());
        assert!(matches!(
            Decoder::start(
                library,
                File::open(fixtures.join("tone.wav")).unwrap(),
                1,
                Output::Null
            ),
            Err(DecoderError::Profile)
        ));
    }
}

fn wait(decoder: &mut Decoder, generation: u64, predicate: impl Fn(DecoderEvent) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(4);
    while Instant::now() < deadline {
        let (actual, event) = decoder.event(Duration::from_millis(50));
        assert_eq!(actual, generation);
        if predicate(event) {
            return;
        }
        assert!(
            !matches!(event, DecoderEvent::End { .. } | DecoderEvent::Shutdown),
            "unexpected terminal event: {event:?}"
        );
    }
    panic!("timed out waiting for decoder event");
}
