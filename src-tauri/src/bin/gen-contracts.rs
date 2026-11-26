fn main() {
    let output =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../src/generated/contracts.ts");
    let expected = echo_lib::contract::typescript();
    if std::env::args().nth(1).as_deref() == Some("--check") {
        let current = std::fs::read_to_string(output).expect("read generated bindings");
        assert_eq!(
            current.replace("\r\n", "\n"),
            expected,
            "run bun run gen:contracts"
        );
    } else {
        std::fs::write(output, expected).expect("write generated bindings");
    }
}
