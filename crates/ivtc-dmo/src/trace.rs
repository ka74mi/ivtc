use std::fmt;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::{Mutex, OnceLock};

use windows::Win32::System::Diagnostics::Debug::OutputDebugStringW;
use windows::Win32::System::Registry::{HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ, RegGetValueW};
use windows::core::{PCWSTR, w};

enum Sink {
    Off,
    Debugger,
    File(Mutex<File>),
}

struct Tracer {
    sink: Sink,
    exe: String,
}

static TRACER: OnceLock<Tracer> = OnceLock::new();

fn read_registry_setting() -> Option<String> {
    let mut size = 0u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            w!(r"SOFTWARE\PlaybackIVTC"),
            w!("Trace"),
            RRF_RT_REG_SZ,
            None,
            None,
            Some(&mut size),
        )
    };
    if status.is_err() || size < 2 {
        return None;
    }
    let mut buf = vec![0u16; (size as usize).div_ceil(2)];
    let status = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            w!(r"SOFTWARE\PlaybackIVTC"),
            w!("Trace"),
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr().cast()),
            Some(&mut size),
        )
    };
    if status.is_err() {
        return None;
    }
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    Some(String::from_utf16_lossy(&buf[..len]))
}

fn open_file(path: &str) -> Option<File> {
    if let Some(parent) = Path::new(path).parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    OpenOptions::new().create(true).append(true).open(path).ok()
}

fn tracer() -> &'static Tracer {
    TRACER.get_or_init(|| {
        let setting = std::env::var("PLAYBACK_IVTC_TRACE")
            .ok()
            .filter(|v| !v.is_empty())
            .or_else(read_registry_setting)
            .filter(|v| !v.is_empty());
        let sink = match setting {
            Some(v) if v.eq_ignore_ascii_case("debug") => Sink::Debugger,
            Some(v) => open_file(&v).map_or(Sink::Off, |f| Sink::File(Mutex::new(f))),
            None => Sink::Off,
        };
        let exe = std::env::current_exe()
            .ok()
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
            .unwrap_or_else(|| "?".into());
        Tracer { sink, exe }
    })
}

pub(crate) fn enabled() -> bool {
    !matches!(tracer().sink, Sink::Off)
}

pub(crate) fn write(args: fmt::Arguments<'_>) {
    let t = tracer();
    let line = format!(
        "[{}:{}:{:?}] {}\n",
        t.exe,
        std::process::id(),
        std::thread::current().id(),
        args
    );
    match &t.sink {
        Sink::Off => {}
        Sink::Debugger => {
            let wide: Vec<u16> = format!("PlaybackIVTC {line}")
                .encode_utf16()
                .chain(Some(0))
                .collect();
            unsafe { OutputDebugStringW(PCWSTR(wide.as_ptr())) };
        }
        Sink::File(file) => {
            if let Ok(mut f) = file.lock() {
                let _ = f.write_all(line.as_bytes());
            }
        }
    }
}

macro_rules! trace {
    ($($arg:tt)*) => {
        if $crate::trace::enabled() {
            $crate::trace::write(format_args!($($arg)*));
        }
    };
}
