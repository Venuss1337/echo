#![cfg(all(windows, target_arch = "x86_64"))]

#[path = "support/http.rs"]
mod http;
#[path = "support/mpv.rs"]
mod mpv;

use echo_lib::native::{NativeError, NativeLibrary};
use echo_lib::radio::RadioStream;
use http::{Response, Server};
use mpv::{EOF, ERROR, Event, LoadError, Mpv, STOP};
use std::{
    ffi::CString,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

#[test]
fn missing_and_relative_dll() {
    let dir = tempfile::tempdir().unwrap();
    // SAFETY: no DLL exists in the fresh temp directory; relative paths are rejected.
    unsafe {
        assert_eq!(
            Mpv::open(&dir.path().join("missing.dll")).err(),
            Some(LoadError::Dll)
        );
        assert_eq!(
            Mpv::open(Path::new("libmpv-2.dll")).err(),
            Some(LoadError::RelativePath)
        );
    }
}

#[test]
fn native_loader_abi_and_symbols() {
    run("loader");
}

#[test]
fn installed_resource_failures() {
    let root = tempfile::tempdir().unwrap();
    let native = root.path().join("native");
    std::fs::create_dir(&native).unwrap();
    let primary = native.join("libmpv-2.dll");
    let digest = "ab".repeat(32);
    let manifest = format!(r#"{{"libmpv-2.dll":"{digest}"}}"#);
    // SAFETY: nothing in this fresh directory is allowed to reach the OS loader.
    unsafe {
        assert_eq!(
            NativeLibrary::open(root.path(), &manifest).err(),
            Some(NativeError::Missing("libmpv-2.dll".into()))
        );
        std::fs::write(&primary, b"corrupt").unwrap();
        assert_eq!(
            NativeLibrary::open(root.path(), &manifest).err(),
            Some(NativeError::Digest("libmpv-2.dll".into()))
        );
        let incomplete = format!(r#"{{"extra.dll":"{digest}","libmpv-2.dll":"{digest}"}}"#);
        assert_eq!(
            NativeLibrary::open(root.path(), &incomplete).err(),
            Some(NativeError::Missing("extra.dll".into()))
        );
        std::fs::write(native.join("unexpected.dll"), b"unlisted").unwrap();
        assert_eq!(
            NativeLibrary::open(root.path(), &manifest).err(),
            Some(NativeError::UnexpectedFile)
        );
    }
}

#[test]
#[ignore = "requires the pinned development libmpv DLL and generated fixtures"]
fn local_playback_transition_and_eof() {
    run("local");
}

#[test]
#[ignore = "requires a confirmed device-free Windows environment; use test-no-output.ps1"]
fn no_output_device() {
    assert_eq!(std::env::var("ECHO_TEST_NO_OUTPUT").as_deref(), Ok("1"));
    run("no-output");
}

#[test]
#[ignore = "requires the pinned development libmpv DLL"]
fn bad_files_are_accepted_as_commands_then_fail_to_load() {
    run("bad-files");
}

#[test]
#[ignore = "requires the pinned development libmpv DLL and generated fixtures"]
fn loopback_http_chunked_playback() {
    run("http-ok");
}

#[test]
#[ignore = "requires the pinned development libmpv DLL and generated fixtures"]
fn loopback_http_rejection() {
    run("http-rejected");
}

#[test]
#[ignore = "requires the pinned libmpv DLL and generated fixtures"]
fn loopback_http_redirect_connects_to_new_destination() {
    run("http-redirect");
}

#[test]
#[ignore = "requires the pinned development libmpv DLL and generated fixtures"]
fn loopback_http_interruption() {
    run("http-interrupted");
}

#[test]
#[ignore = "requires the pinned development libmpv DLL and generated fixtures"]
fn loopback_http_cancellation() {
    run("http-cancelled");
}

fn run(case: &str) {
    // libmpv can block in synchronous calls or destruction. Keep the fixture
    // server and all native work in a child the parent can kill and reap.
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--ignored", "--exact", "native_worker", "--nocapture"])
        .env("ECHO_NATIVE_WORKER_CASE", case)
        .env("no_proxy", "127.0.0.1")
        .env_remove("http_proxy")
        .env_remove("https_proxy")
        .env_remove("all_proxy")
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(24);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(status.success(), "{case} failed: {status}");
            return;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            child.wait().unwrap();
            panic!("{case} exceeded the 24-second worker deadline");
        }
        thread::sleep(Duration::from_millis(25));
    }
}

#[test]
#[ignore = "internal subprocess entry point; invoked by parent tests"]
fn native_worker() {
    let case = std::env::var("ECHO_NATIVE_WORKER_CASE").expect("internal worker case");
    if case == "loader" {
        loader_cases();
        return;
    }
    let mut player = if let Some(root) = std::env::var_os("ECHO_INSTALLED_NATIVE_ROOT") {
        let root = PathBuf::from(root);
        let manifest_path = std::env::var_os("ECHO_INSTALLED_NATIVE_MANIFEST")
            .map(PathBuf::from)
            .unwrap_or_else(|| root.join("dlls.json"));
        let manifest = std::fs::read_to_string(manifest_path).unwrap();
        // SAFETY: this test process is dedicated to the manifest staged by the build operator.
        let library = unsafe { NativeLibrary::open(&root, &manifest) }.unwrap();
        let audio_output = if case == "no-output" {
            assert_eq!(std::env::var("ECHO_TEST_NO_OUTPUT").as_deref(), Ok("1"));
            c"wasapi"
        } else {
            c"null"
        };
        let mut player = Mpv::from_installed(library, audio_output).unwrap();
        assert_eq!(player.text(c"ffmpeg-version").as_deref(), Some("9.1"));
        player
    } else {
        assert_ne!(case, "no-output", "requires installed native resources");
        let dll = std::env::var_os("ECHO_VERIFIED_LIBMPV")
            .expect("prepare the pinned developer DLL first");
        // SAFETY: the operator verifies the DLL with the documented artifact script.
        unsafe { Mpv::open(dll.as_ref()) }.unwrap()
    };
    match case.as_str() {
        "radio-denied" => rejected_custom_stream(&mut player),
        "local" => local(&mut player),
        "no-output" => {
            const AO_INIT_FAILED: i32 = -14; // MPV_ERROR_AO_INIT_FAILED, client.h
            load(&mut player, fixture("tone.wav").to_str().unwrap());
            player.expect(Event::End {
                reason: ERROR,
                error: AO_INIT_FAILED,
            });
            assert!(player.text(c"current-ao").is_none());
        }
        "bad-files" => bad_files(&mut player),
        "http-ok" => http_case(&mut player, Response::Complete),
        "http-rejected" => http_case(&mut player, Response::Rejected),
        "http-redirect" => http_redirect(&mut player),
        "http-interrupted" => http_case(&mut player, Response::Interrupted),
        "http-cancelled" => http_case(&mut player, Response::Cancelled),
        _ => panic!("unknown worker case"),
    }
    drop(player);
    println!("{case}: libmpv shutdown returned");
}

#[test]
#[ignore = "requires native libmpv"]
fn custom_stream_rejects_private_destination() {
    run("radio-denied");
}

fn rejected_custom_stream(player: &mut Mpv) {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    listener.set_nonblocking(true).unwrap();
    let source = format!("http://{}/stream", listener.local_addr().unwrap());
    let stream = RadioStream::start(runtime.handle(), &source).unwrap();
    let opaque = player.radio().replace(stream).unwrap();
    std::thread::scope(|scope| {
        let (done, wait) = tokio::sync::oneshot::channel();
        scope.spawn(|| {
            runtime.block_on(async {
                let _ = wait.await;
            })
        });
        load(player, opaque.to_str().unwrap());
        expect_load_error(player);
        assert_eq!(
            listener.accept().unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
        let _ = done.send(());
    });
}

fn loader_cases() {
    let dir = tempfile::tempdir().unwrap();
    for (body, expected) in [
        (
            "pub extern \"C\" fn unrelated() -> u32 { 0 }",
            LoadError::Symbol("mpv_client_api_version"),
        ),
        (
            "pub extern \"C\" fn mpv_client_api_version() -> u32 { 3 << 16 }",
            LoadError::Abi(3 << 16),
        ),
        (
            "pub extern \"C\" fn mpv_client_api_version() -> u32 { (2 << 16) | 99 }",
            LoadError::Symbol("mpv_create"),
        ),
    ] {
        let source = dir.path().join("fixture.rs");
        let dll = dir.path().join(format!("fixture-{}.dll", body.len()));
        std::fs::write(&source, format!("#[unsafe(no_mangle)] {body}")).unwrap();
        assert!(
            Command::new("rustc")
                .args(["--crate-type", "cdylib", "--edition", "2024"])
                .arg(&source)
                .arg("-o")
                .arg(&dll)
                .status()
                .unwrap()
                .success()
        );
        let native_expected = match &expected {
            LoadError::Symbol(symbol) => NativeError::MissingExport(symbol),
            LoadError::Abi(abi) => NativeError::IncompatibleAbi(*abi),
            _ => unreachable!(),
        };
        // SAFETY: the test compiled this DLL from the literal declarations above.
        assert_eq!(unsafe { Mpv::open(&dll) }.err(), Some(expected));
        let native_dir = dir.path().join("native");
        std::fs::create_dir_all(&native_dir).unwrap();
        let bytes = std::fs::read(&dll).unwrap();
        std::fs::write(native_dir.join("libmpv-2.dll"), &bytes).unwrap();
        use sha2::{Digest, Sha256};
        let digest = Sha256::digest(&bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let manifest = serde_json::json!({"libmpv-2.dll": digest}).to_string();
        // SAFETY: the manifest pins the same test-compiled DLL in the controlled directory.
        assert_eq!(
            unsafe { NativeLibrary::open(dir.path(), &manifest) }.err(),
            Some(native_expected)
        );
    }
}

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../.echo-fixtures")
        .join(name)
}

fn load(player: &mut Mpv, source: &str) {
    let source = CString::new(source).unwrap();
    assert_eq!(
        player.command(&[c"loadfile", &source]),
        0,
        "load command acceptance"
    );
}

fn wait_for_progress(player: &mut Mpv, limit: f64) {
    player.until(|player, event| {
        assert!(
            !matches!(event, Event::End { .. }),
            "ended before progress: {event:?}"
        );
        player.number(c"time-pos").filter(|&pos| pos > limit)
    });
}

fn local(player: &mut Mpv) {
    println!(
        "embedded FFmpeg: {}",
        player.text(c"ffmpeg-version").unwrap()
    );
    assert_eq!(player.command(&[c"set", c"pause", c"yes"]), 0);
    load(player, fixture("transport.wav").to_str().unwrap());
    player.expect(Event::Loaded);
    player.until(|player, _| (player.flag(c"pause") == Some(true)).then_some(()));
    assert_eq!(player.command(&[c"set", c"pause", c"no"]), 0);
    wait_for_progress(player, 0.25);
    assert_eq!(player.text(c"current-ao").as_deref(), Some("null"));

    assert_eq!(player.command(&[c"set", c"pause", c"yes"]), 0);
    let paused_at = player.until(|player, _| {
        (player.flag(c"pause") == Some(true))
            .then(|| player.number(c"time-pos"))
            .flatten()
    });
    let end = Instant::now() + Duration::from_millis(350);
    while Instant::now() < end {
        assert!(!matches!(
            player.event(Duration::from_millis(25)),
            Event::End { .. }
        ));
        assert!((player.number(c"time-pos").unwrap() - paused_at).abs() < 0.1);
    }
    assert_eq!(player.flag(c"seekable"), Some(true));
    assert_eq!(player.command(&[c"seek", c"5", c"absolute+exact"]), 0);
    player.expect(Event::Seek);
    player.until(|player, event| {
        assert!(!matches!(event, Event::End { .. }));
        (player.flag(c"seeking") == Some(false)
            && player
                .number(c"time-pos")
                .is_some_and(|pos| (pos - 5.0).abs() < 0.2))
        .then_some(())
    });
    assert_eq!(player.flag(c"pause"), Some(true));
    assert_eq!(player.command(&[c"set", c"pause", c"no"]), 0);
    wait_for_progress(player, 5.3);
    assert_eq!(player.command(&[c"stop"]), 0);
    player.expect(Event::End {
        reason: STOP,
        error: 0,
    });

    load(player, fixture("tone.wav").to_str().unwrap());
    player.expect(Event::Loaded);
    player.expect(Event::Restarted);
    player.expect(Event::End {
        reason: EOF,
        error: 0,
    });
}

fn bad_files(player: &mut Mpv) {
    let dir = tempfile::tempdir().unwrap();
    for (name, contents) in [
        ("corrupt.wav", &b"RIFF incomplete"[..]),
        (
            "unsupported.echo",
            &b"ECHO fixture. This is plain text, not media."[..],
        ),
    ] {
        let path = dir.path().join(name);
        std::fs::write(&path, contents).unwrap();
        load(player, path.to_str().unwrap());
        expect_load_error(player);
    }
}

fn expect_load_error(player: &mut Mpv) {
    player.until(|_, event| match event {
        Event::Loaded | Event::Restarted => panic!("rejected media began playback"),
        Event::End { reason, error } => {
            assert_eq!(reason, ERROR);
            assert!(error < 0);
            Some(())
        }
        _ => None,
    });
}

fn http_redirect(player: &mut Mpv) {
    let destination = Server::new(
        Response::Complete,
        std::fs::read(fixture("tone.wav")).unwrap(),
    );
    let origin = Server::new(Response::Redirect(destination.port), Vec::new());
    load(player, &origin.url);
    player.expect(Event::Loaded);
    player.expect(Event::Restarted);
    player.expect(Event::End {
        reason: EOF,
        error: 0,
    });
    // Both sockets must actually receive a request. Initial URL validation
    // would have seen only the origin, even though the decoder played the target.
    origin.worker.join().unwrap().unwrap();
    destination.worker.join().unwrap().unwrap();
}

fn http_case(player: &mut Mpv, response: Response) {
    let name = if matches!(response, Response::Complete) {
        "tone.wav"
    } else {
        "transport.wav"
    };
    let server = Server::new(response, std::fs::read(fixture(name)).unwrap());
    load(player, &server.url);
    if matches!(response, Response::Rejected) {
        expect_load_error(player);
        // An incomplete IPv6 URL cannot name any endpoint.
        load(player, "http://[");
        expect_load_error(player);
    } else {
        player.expect(Event::Loaded);
        player.expect(Event::Restarted);
        match response {
            Response::Complete => player.expect(Event::End {
                reason: EOF,
                error: 0,
            }),
            Response::Interrupted => {
                wait_for_progress(player, 0.15);
                server.interrupt.send(()).unwrap();
                let mut furthest = 0.0_f64;
                player.until(|player, event| {
                    if let Some(position) = player.number(c"time-pos") {
                        furthest = furthest.max(position);
                    }
                    if let Event::End { reason, error } = event {
                        // A truncated PCM response can be reported as early EOF.
                        assert!((reason == ERROR && error < 0) || (reason == EOF && error == 0));
                        assert!(
                            (0.15..8.0).contains(&furthest),
                            "position at cut: {furthest}"
                        );
                        return Some(());
                    }
                    None
                });
            }
            Response::Cancelled => {
                wait_for_progress(player, 0.15);
                assert_eq!(player.command(&[c"stop"]), 0);
                player.expect(Event::End {
                    reason: STOP,
                    error: 0,
                });
            }
            Response::Rejected | Response::Redirect(_) => unreachable!(),
        }
    }
    server.worker.join().unwrap().unwrap();
}
