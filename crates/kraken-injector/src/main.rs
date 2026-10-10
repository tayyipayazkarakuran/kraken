use std::path::PathBuf;
use std::process::ExitCode;

use game::{Failure, Handle, Target, find_client, open_target, process_access};
use inject::{attach, attach_suspended};
use launch::Child;
use windows_sys::Win32::System::Threading::OpenProcess;

mod game;
mod inject;
mod launch;

fn canonical(path: &str) -> PathBuf {
    match std::fs::canonicalize(path) {
        Ok(path) => PathBuf::from(path.to_string_lossy().trim_start_matches(r"\\?\")),
        Err(_) => PathBuf::from(path),
    }
}

fn usage() -> ExitCode {
    eprintln!(
        "usage: kraken-injector <pid> <dll> [--force]\n       \
         kraken-injector --launch <dll> [--exe <path>] [--parent none] [--method remote]"
    );
    ExitCode::from(Failure::Usage.code())
}

fn exit(failure: Failure) -> ExitCode {
    eprintln!("{failure:?}");
    ExitCode::from(failure.code())
}

fn option(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|at| args.get(at + 1).cloned())
}

fn has(args: &[String], name: &str) -> bool {
    args.iter().any(|a| a == name)
}

fn attach_command(args: &[String]) -> ExitCode {
    let (Some(pid), Some(dll)) = (args.get(1).and_then(|p| p.parse::<u32>().ok()), args.get(2)) else {
        return usage();
    };
    let dll = canonical(dll);
    let force = has(args, "--force");
    match open_target(pid, force) {
        Ok(Target::Already) => {
            println!("already");
            ExitCode::SUCCESS
        }
        Ok(Target::Fresh(process)) => match attach(process.0, &dll) {
            Ok(status) => {
                println!("{status}");
                ExitCode::SUCCESS
            }
            Err(failure) => exit(failure),
        },
        Err(failure) => exit(failure),
    }
}

/// Kraken starts the game: find the client the launcher keeps, hide it
/// behind explorer.exe, load it suspended with the agent queued on its main
/// thread, and let it run. The supervisor takes over from the PID.
fn launch_command(args: &[String]) -> ExitCode {
    let Some(dll) = option(args, "--dll").or_else(|| args.get(2).cloned()) else {
        return usage();
    };
    let dll = canonical(&dll);
    let exe = match option(args, "--exe") {
        Some(exe) => canonical(&exe),
        None => match find_client() {
            Some(exe) => exe,
            None => return exit(Failure::Launch),
        },
    };
    let hidden = option(args, "--parent").as_deref() != Some("none");
    let Child { process, thread, pid } =
        match launch::start(&exe, hidden.then(game::explorer_handle).flatten()) {
            Ok(child) => child,
            Err(failure) => return exit(failure),
        };
    // The agent can also come in through its own thread, on a game that has
    // already started.
    if option(args, "--method").as_deref() == Some("remote") {
        let handle = unsafe { OpenProcess(process_access(), 0, pid) };
        if handle.is_null() {
            return exit(Failure::Open);
        }
        let handle = Handle(handle);
        if let Err(failure) = attach(handle.0, &dll) {
            return exit(failure);
        }
    } else if let Err(failure) = attach_suspended(process, thread, &dll) {
        return exit(failure);
    }
    if let Err(failure) = launch::resume(thread) {
        return exit(failure);
    }
    println!("launched {pid}");
    ExitCode::SUCCESS
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    if has(&args, "--launch") {
        launch_command(&args)
    } else {
        attach_command(&args)
    }
}
