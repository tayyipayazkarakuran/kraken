/// The 32-bit agent and injector, built first by `cargo xtask dist`, ride
/// inside kraken.exe so the release is a single file. Other builds embed
/// nothing and find the parts next to the executable instead.
fn embed() {
    println!("cargo:rerun-if-env-changed=KRAKEN_EMBED_DIR");
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    let source = std::env::var_os("KRAKEN_EMBED_DIR").map(std::path::PathBuf::from);
    for (from, to) in [
        ("kraken_agent.dll", "agent.bin"),
        ("kraken-injector.exe", "injector.bin"),
    ] {
        let bytes = match &source {
            Some(dir) => {
                let path = dir.join(from);
                println!("cargo:rerun-if-changed={}", path.display());
                std::fs::read(&path).unwrap_or_else(|e| panic!("embed {}: {e}", path.display()))
            }
            None => Vec::new(),
        };
        std::fs::write(out.join(to), bytes).expect("write embedded part");
    }
}

fn main() {
    embed();
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let mut resource = winresource::WindowsResource::new();
    resource
        .set("ProductName", "Kraken")
        .set("FileDescription", "Kraken control center");
    if let Err(error) = resource.compile() {
        println!("cargo:warning=resource compilation skipped: {error}");
    }
}
