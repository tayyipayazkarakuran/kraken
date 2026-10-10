use windows_sys::Win32::System::SystemInformation::GetLocalTime;

pub fn stamp() -> String {
    let time = unsafe {
        let mut time = std::mem::zeroed();
        GetLocalTime(&mut time);
        time
    };
    format!("{:02}:{:02}:{:02}", time.wHour, time.wMinute, time.wSecond)
}
