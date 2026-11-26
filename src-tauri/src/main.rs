// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    #[cfg(all(windows, feature = "native-resources"))]
    if std::env::args_os()
        .nth(1)
        .is_some_and(|arg| arg == "--verify-native-resources")
    {
        if let Err(error) = echo_lib::native::verify_installed_executable() {
            eprintln!("ECHO native resources: {error:?}");
            std::process::exit(1);
        }
        return;
    }
    echo_lib::run()
}
