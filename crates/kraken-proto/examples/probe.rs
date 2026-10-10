use std::fs::OpenOptions;
use std::io::BufReader;
use std::time::{Duration, Instant};

use kraken_proto::{AgentMessage, AppMessage, PROTO_VERSION, pipe_names, read_frame, write_frame};

fn main() {
    let mut args = std::env::args().skip(1);
    let pid: u32 = args
        .next()
        .and_then(|p| p.parse().ok())
        .expect("usage: probe <pid> [secs] [dump]");
    let secs: u64 = args.next().and_then(|s| s.parse().ok()).unwrap_or(5);
    let dump = args.next().is_some_and(|a| a == "dump");
    let (down, up) = pipe_names(pid);
    let mut writer = OpenOptions::new().write(true).open(&down).expect("down pipe");
    let reader = (0..60)
        .find_map(|_| {
            OpenOptions::new().read(true).open(&up).ok().or_else(|| {
                std::thread::sleep(Duration::from_millis(50));
                None
            })
        })
        .expect("up pipe");
    write_frame(
        &mut writer,
        &AppMessage::Hello {
            version: PROTO_VERSION,
        },
    )
    .expect("hello");
    if dump {
        write_frame(&mut writer, &AppMessage::DumpTypes).expect("dump");
    }
    let started = Instant::now();
    let mut reader = BufReader::new(reader);
    let mut last_telemetry = None;
    while started.elapsed() < Duration::from_secs(secs) {
        match read_frame::<AgentMessage, _>(&mut reader) {
            Ok(AgentMessage::Telemetry(t)) => last_telemetry = Some(t),
            Ok(AgentMessage::Catalog(c)) => {
                println!(
                    "catalog [{}]: npcs={} monsters={} boxes={} ammo={} harpoons={}",
                    c.language,
                    c.npcs.len(),
                    c.monsters.len(),
                    c.boxes.len(),
                    c.ammo.len(),
                    c.harpoons.len()
                );
                let sample = |entries: &[kraken_proto::CatalogEntry]| {
                    entries
                        .iter()
                        .take(3)
                        .map(|e| format!("{}={}", e.type_id, e.name))
                        .collect::<Vec<_>>()
                        .join(", ")
                };
                println!(
                    "  items: {}",
                    c.items
                        .iter()
                        .map(|i| format!(
                            "{}={}x{}{}",
                            i.id,
                            i.name,
                            i.amount,
                            if i.active { "*" } else { "" }
                        ))
                        .collect::<Vec<_>>()
                        .join(", ")
                );
                println!("  npcs: {}", sample(&c.npcs));
                println!("  boxes: {}", sample(&c.boxes));
                println!(
                    "  ammo: {}",
                    c.ammo
                        .iter()
                        .take(3)
                        .map(|e| format!("{}={}", e.id, e.name))
                        .collect::<Vec<_>>()
                        .join(", ")
                );
            }
            Ok(other) => println!("{other:?}"),
            Err(error) => {
                println!("stream ended: {error}");
                break;
            }
        }
    }
    println!("last telemetry: {last_telemetry:#?}");
}
