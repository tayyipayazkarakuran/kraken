use std::fs::{File, OpenOptions};
use std::io::{BufReader, BufWriter};
use std::os::windows::io::FromRawHandle;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::time::Duration;

use kraken_proto::{AgentMessage, AppMessage, read_frame, write_frame};
use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_PIPE_CONNECTED, GetLastError, HANDLE, INVALID_HANDLE_VALUE, LocalFree,
};
use windows_sys::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows_sys::Win32::Security::{
    GetTokenInformation, PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER, TokenUser,
};
use windows_sys::Win32::Storage::FileSystem::{PIPE_ACCESS_INBOUND, PIPE_ACCESS_OUTBOUND};
use windows_sys::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS,
    PIPE_TYPE_BYTE, PIPE_WAIT,
};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

const BUFFER: u32 = 256 * 1024;

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

fn user_sid() -> Option<String> {
    unsafe {
        let mut token: HANDLE = std::ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return None;
        }
        let mut size = 0u32;
        GetTokenInformation(token, TokenUser, std::ptr::null_mut(), 0, &mut size);
        let mut buffer = vec![0u8; size as usize];
        let ok = GetTokenInformation(token, TokenUser, buffer.as_mut_ptr().cast(), size, &mut size);
        CloseHandle(token);
        if ok == 0 {
            return None;
        }
        let user = &*buffer.as_ptr().cast::<TOKEN_USER>();
        let mut text: *mut u16 = std::ptr::null_mut();
        if ConvertSidToStringSidW(user.User.Sid, &mut text) == 0 {
            return None;
        }
        let len = (0..).take_while(|i| *text.add(*i) != 0).count();
        let sid = String::from_utf16_lossy(std::slice::from_raw_parts(text, len));
        LocalFree(text.cast());
        Some(sid)
    }
}

struct Security {
    descriptor: PSECURITY_DESCRIPTOR,
}

impl Security {
    fn new() -> Option<Self> {
        let sddl = wide(&format!("D:P(A;;GA;;;{})(A;;GA;;;SY)", user_sid()?));
        let mut descriptor: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
        let ok = unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                SDDL_REVISION_1,
                &mut descriptor,
                std::ptr::null_mut(),
            )
        };
        (ok != 0).then_some(Self { descriptor })
    }

    fn attributes(&self) -> SECURITY_ATTRIBUTES {
        SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: self.descriptor,
            bInheritHandle: 0,
        }
    }
}

impl Drop for Security {
    fn drop(&mut self) {
        unsafe { LocalFree(self.descriptor) };
    }
}

fn listen(name: &str, access: u32, security: &Security) -> Option<File> {
    let name = wide(name);
    let attributes = security.attributes();
    let handle = unsafe {
        CreateNamedPipeW(
            name.as_ptr(),
            access,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            1,
            BUFFER,
            BUFFER,
            0,
            &attributes,
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return None;
    }
    let connected = unsafe { ConnectNamedPipe(handle, std::ptr::null_mut()) } != 0
        || unsafe { GetLastError() } == ERROR_PIPE_CONNECTED;
    if !connected {
        unsafe { CloseHandle(handle) };
        return None;
    }
    Some(unsafe { File::from_raw_handle(handle as _) })
}

fn disconnect(file: &File) {
    use std::os::windows::io::AsRawHandle;
    unsafe { DisconnectNamedPipe(file.as_raw_handle() as HANDLE) };
}

static QUIT: AtomicBool = AtomicBool::new(false);
const SECOND_PIPE_WAIT: Duration = Duration::from_secs(3);

fn poke(name: &str) {
    let _ = OpenOptions::new().write(true).open(name);
    let _ = OpenOptions::new().read(true).open(name);
}

fn listen_second(name: &str, security: &Security) -> Option<File> {
    let done = Arc::new(AtomicBool::new(false));
    let poked = Arc::new(AtomicBool::new(false));
    let watchdog = {
        let (done, poked, name) = (done.clone(), poked.clone(), name.to_owned());
        std::thread::spawn(move || {
            let started = std::time::Instant::now();
            while !done.load(Ordering::SeqCst) {
                if started.elapsed() >= SECOND_PIPE_WAIT || QUIT.load(Ordering::SeqCst) {
                    poked.store(true, Ordering::SeqCst);
                    poke(&name);
                    return;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        })
    };
    let file = listen(name, PIPE_ACCESS_OUTBOUND, security);
    done.store(true, Ordering::SeqCst);
    let _ = watchdog.join();
    if poked.load(Ordering::SeqCst) {
        if let Some(file) = &file {
            disconnect(file);
        }
        return None;
    }
    file
}

pub fn shutdown(pid: u32) {
    QUIT.store(true, Ordering::SeqCst);
    let (down, up) = kraken_proto::pipe_names(pid);
    poke(&down);
    poke(&up);
}

pub fn serve(pid: u32, inbox: &Sender<AppMessage>, outbox: &Receiver<AgentMessage>) {
    let Some(security) = Security::new() else {
        tracing::error!("pipe security descriptor unavailable");
        return;
    };
    let (down, up) = kraken_proto::pipe_names(pid);
    while !QUIT.load(Ordering::SeqCst) {
        let Some(reader) = listen(&down, PIPE_ACCESS_INBOUND, &security) else {
            std::thread::sleep(Duration::from_millis(500));
            continue;
        };
        let Some(writer) = listen_second(&up, &security) else {
            disconnect(&reader);
            continue;
        };
        if QUIT.load(Ordering::SeqCst) {
            disconnect(&reader);
            disconnect(&writer);
            break;
        }
        tracing::info!("controller connected");
        while outbox.try_recv().is_ok() {}
        let alive = Arc::new(AtomicBool::new(true));
        let reader_alive = alive.clone();
        let reader_inbox = inbox.clone();
        let reading = std::thread::spawn(move || {
            let mut stream = BufReader::new(reader);
            while let Ok(message) = read_frame::<AppMessage, _>(&mut stream) {
                // Interface frames go straight to the renderer, not through
                // the game tick.
                if let AppMessage::Frame(frame) = message {
                    crate::overlay::submit(*frame);
                    continue;
                }
                if reader_inbox.send(message).is_err() {
                    break;
                }
            }
            crate::overlay::clear();
            disconnect(stream.get_ref());
            reader_alive.store(false, Ordering::SeqCst);
        });
        let mut stream = BufWriter::new(writer);
        while alive.load(Ordering::SeqCst) {
            match outbox.recv_timeout(Duration::from_millis(200)) {
                Ok(message) => {
                    if write_frame(&mut stream, &message).is_err() {
                        break;
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
        disconnect(stream.get_ref());
        drop(stream);
        let _ = reading.join();
        tracing::info!("controller disconnected");
    }
    tracing::info!("pipe server stopped");
}
