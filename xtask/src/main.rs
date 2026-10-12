use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

const APP_TARGET: &str = "x86_64-pc-windows-msvc";
const AGENT_TARGET: &str = "i686-pc-windows-msvc";
const WEB_TARGET: &str = "wasm32-unknown-unknown";

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .to_path_buf()
}

fn cargo(args: &[&str]) -> Result<(), String> {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    run(&cargo, args, &root())
}

fn run(program: &str, args: &[&str], dir: &Path) -> Result<(), String> {
    println!("> {program} {}", args.join(" "));
    let status = Command::new(program)
        .args(args)
        .current_dir(dir)
        .status()
        .map_err(|e| format!("{program}: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{program} {} failed with {status}", args.join(" ")))
    }
}

fn copy(from: &Path, to: &Path) -> Result<(), String> {
    if std::fs::copy(from, to).is_ok() {
        return Ok(());
    }
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis());
    let aside = to.with_extension(format!("{stamp}.old"));
    std::fs::rename(to, &aside).map_err(|e| format!("move locked {} aside: {e}", to.display()))?;
    std::fs::copy(from, to)
        .map(|_| ())
        .map_err(|e| format!("copy {} -> {}: {e}", from.display(), to.display()))
}

/// One file: the 32-bit agent and injector are built first and embedded in
/// kraken.exe (see kraken-app/build.rs).
fn dist() -> Result<PathBuf, String> {
    cargo(&[
        "build",
        "--release",
        "--package",
        "kraken-agent",
        "--package",
        "kraken-injector",
        "--target",
        AGENT_TARGET,
    ])?;
    let target = root().join("target");
    let agent = target.join(AGENT_TARGET).join("release");
    // SAFETY: xtask is single-threaded; the variable is read by the child cargo.
    unsafe { std::env::set_var("KRAKEN_EMBED_DIR", &agent) };
    let built = cargo(&[
        "build",
        "--release",
        "--package",
        "kraken-app",
        "--target",
        APP_TARGET,
    ]);
    unsafe { std::env::remove_var("KRAKEN_EMBED_DIR") };
    built?;
    let app = target.join(APP_TARGET).join("release");
    let out = root().join("dist").join("Kraken");
    std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
    copy(&app.join("kraken.exe"), &out.join("kraken.exe"))?;
    // Leftovers of the three-file layout would shadow nothing but confuse.
    for old in ["kraken-agent.dll", "kraken-injector.exe"] {
        let _ = std::fs::remove_file(out.join(old));
    }
    Ok(out)
}

/// kraken-web for the browser build: the wasm, its `wasm-bindgen` glue, the
/// bootstrap and the userscript, written into the site (`web/public/__kraken`)
/// and the Android app's assets.
fn web() -> Result<PathBuf, String> {
    cargo(&[
        "build",
        "--release",
        "--package",
        "kraken-web",
        "--target",
        WEB_TARGET,
    ])?;
    let wasm = root()
        .join("target")
        .join(WEB_TARGET)
        .join("release")
        .join("kraken_web.wasm");
    let out = root().join("web/public/__kraken");
    std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
    let (wasm, out_dir) = (wasm.to_string_lossy(), out.to_string_lossy());
    // The CLI must match the `wasm-bindgen` version in Cargo.lock:
    // `cargo install wasm-bindgen-cli --version <that version>`.
    run(
        "wasm-bindgen",
        &[
            "--target",
            "no-modules",
            "--no-typescript",
            "--out-dir",
            &out_dir,
            &wasm,
        ],
        &root(),
    )?;
    let read =
        |path: &str| std::fs::read_to_string(root().join(path)).map_err(|e| format!("read {path}: {e}"));
    let boot = read("android/kraken-web/js/kraken-boot.js")?;
    std::fs::write(out.join("kraken-boot.js"), &boot).map_err(|e| e.to_string())?;
    // The Worker fills in the site's origin when it serves /kraken.user.js.
    let userscript = read("web/userscript.template.js")?
        .replace("__KRAKEN_VERSION__", env!("CARGO_PKG_VERSION"))
        .replace("__KRAKEN_BOOT__", &js_string(&boot));
    std::fs::write(out.join("userscript.js"), userscript).map_err(|e| e.to_string())?;

    let assets = root().join("android/app/src/main/assets/kraken");
    std::fs::create_dir_all(&assets).map_err(|e| e.to_string())?;
    for file in ["kraken_web.js", "kraken_web_bg.wasm", "kraken-boot.js"] {
        copy(&out.join(file), &assets.join(file))?;
    }
    Ok(out)
}

/// `text` as a JavaScript string literal.
fn js_string(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\u{2028}' => out.push_str("\\u2028"),
            '\u{2029}' => out.push_str("\\u2029"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn android() -> Result<PathBuf, String> {
    web()?;
    let project = root().join("android");
    let gradle = if cfg!(windows) { "gradlew.bat" } else { "./gradlew" };
    run(gradle, &["assembleDebug"], &project)?;
    Ok(project.join("app/build/outputs/apk/debug/app-debug.apk"))
}

fn check() -> Result<(), String> {
    cargo(&["fmt", "--all", "--check"])?;
    cargo(&["clippy", "--workspace", "--all-targets", "--", "-D", "warnings"])?;
    cargo(&["test", "--workspace"])?;
    cargo(&[
        "clippy",
        "--package",
        "kraken-agent",
        "--package",
        "kraken-injector",
        "--target",
        AGENT_TARGET,
        "--",
        "-D",
        "warnings",
    ])?;
    cargo(&[
        "clippy",
        "--package",
        "kraken-web",
        "--target",
        WEB_TARGET,
        "--",
        "-D",
        "warnings",
    ])
}

fn unload(pid: u32) {
    let (down, up) = kraken_proto::pipe_names(pid);
    let Ok(mut writer) = std::fs::OpenOptions::new().write(true).open(&down) else {
        println!("no agent pipe for {pid}");
        return;
    };
    let reader = (0..60).find_map(|_| {
        std::fs::OpenOptions::new().read(true).open(&up).ok().or_else(|| {
            std::thread::sleep(std::time::Duration::from_millis(50));
            None
        })
    });
    let sent = kraken_proto::write_frame(&mut writer, &kraken_proto::AppMessage::Unload).is_ok();
    println!("unload sent: {sent}");
    std::thread::sleep(std::time::Duration::from_millis(200));
    drop(reader);
    drop(writer);
    std::thread::sleep(std::time::Duration::from_secs(2));
}

fn reload(pid: &str) -> Result<(), String> {
    let pid: u32 = pid.parse().map_err(|_| format!("bad pid {pid}"))?;
    cargo(&[
        "build",
        "--release",
        "--package",
        "kraken-agent",
        "--package",
        "kraken-injector",
        "--target",
        AGENT_TARGET,
    ])?;
    let built = root().join("target").join(AGENT_TARGET).join("release");
    let local = std::env::var_os("LOCALAPPDATA").ok_or("LOCALAPPDATA is not set")?;
    let agents = PathBuf::from(local).join("Kraken").join("agents");
    std::fs::create_dir_all(&agents).map_err(|e| e.to_string())?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis());
    let dll = agents.join(format!("kraken-agent-{stamp}.dll"));
    copy(&built.join("kraken_agent.dll"), &dll)?;
    unload(pid);
    let status = Command::new(built.join("kraken-injector.exe"))
        .arg(pid.to_string())
        .arg(&dll)
        .arg("--force")
        .status()
        .map_err(|e| e.to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("injector failed with {status}"))
    }
}

fn main() -> ExitCode {
    let task = std::env::args().nth(1);
    let result = match task.as_deref() {
        Some("dist") => dist().map(|out| println!("dist ready: {}", out.display())),
        Some("check") => check(),
        Some("web") => web().map(|out| println!("web build ready: {}", out.display())),
        Some("android") => android().map(|apk| println!("apk ready: {}", apk.display())),
        Some("reload") => match std::env::args().nth(2) {
            Some(pid) => reload(&pid),
            None => Err("usage: cargo xtask reload <pid>".into()),
        },
        _ => Err("usage: cargo xtask <dist|check|web|android|reload <pid>>".into()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}
