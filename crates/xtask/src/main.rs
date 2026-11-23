use std::path::PathBuf;
use std::process::{Command, ExitCode};

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let command = args.next().unwrap_or_default();
    let args_rest: Vec<String> = args.collect();
    let release = args_rest.iter().any(|a| a == "--release");
    let args_rest: Vec<String> = args_rest.into_iter().filter(|a| a != "--release").collect();

    let result = match command.as_str() {
        "build" => build(release).map(|_| ()),
        "register" => build(release).and_then(|dll| regsvr32(&dll, false)),
        "unregister" => regsvr32(&dll_path(release), true),
        "reinstall" => {
            let _ = regsvr32(&dll_path(release), true);
            build(release).and_then(|dll| regsvr32(&dll, false))
        }
        "trace-on" => trace_on(args_rest.first().map(String::as_str)),
        "trace-off" => trace_off(),
        "check" => check(),
        _ => {
            eprintln!("{}", include_str!("usage.txt"));
            return ExitCode::from(2);
        }
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("error: {message}");
            ExitCode::FAILURE
        }
    }
}

fn cargo() -> Command {
    Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()))
}

fn run(mut command: Command) -> Result<(), String> {
    let status = command.status().map_err(|e| format!("{command:?}: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{command:?} が失敗しました ({status})"))
    }
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("workspace root")
}

fn dll_path(release: bool) -> PathBuf {
    workspace_root()
        .join("target")
        .join(if release { "release" } else { "debug" })
        .join("ivtc_dmo.dll")
}

fn build(release: bool) -> Result<PathBuf, String> {
    let mut c = cargo();
    c.args(["build", "-p", "ivtc-dmo"]);
    if release {
        c.arg("--release");
    }
    run(c)?;
    Ok(dll_path(release))
}

fn regsvr32(dll: &PathBuf, unregister: bool) -> Result<(), String> {
    let mut c = Command::new("regsvr32");
    c.arg("/s");
    if unregister {
        c.arg("/u");
    }
    c.arg(dll);
    run(c).map_err(|e| format!("{e}\n  (登録はHKCRに書くため、管理者として実行した PowerShell / Windows Terminal で試してください)"))
}

fn check() -> Result<(), String> {
    let mut fmt = cargo();
    fmt.args(["fmt", "--all", "--", "--check"]);
    run(fmt)?;
    let mut clippy = cargo();
    clippy.args([
        "clippy",
        "--workspace",
        "--all-targets",
        "--",
        "-D",
        "warnings",
    ]);
    run(clippy)?;
    let mut test = cargo();
    test.args(["test", "--workspace"]);
    run(test)
}

const TRACE_KEY: &str = r"HKLM\SOFTWARE\PlaybackIVTC";

fn trace_on(target: Option<&str>) -> Result<(), String> {
    let default = std::env::var("PUBLIC")
        .map(|p| format!(r"{p}\PlaybackIVTC\trace.log"))
        .unwrap_or_else(|_| r"C:\Users\Public\PlaybackIVTC\trace.log".into());
    let value = target.unwrap_or(&default);
    let mut c = Command::new("reg");
    c.args([
        "add", TRACE_KEY, "/v", "Trace", "/t", "REG_SZ", "/d", value, "/f",
    ]);
    run(c)?;
    println!("トレース出力先: {value}\n(対象アプリを再起動してください)");
    Ok(())
}

fn trace_off() -> Result<(), String> {
    let mut c = Command::new("reg");
    c.args(["delete", TRACE_KEY, "/v", "Trace", "/f"]);
    run(c)
}
