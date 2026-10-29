// Blocking stdin/stdout frame server for zi-envd.

use crate::protocol::frame::{decode_frame, Frame, ERROR, PING, REQUEST, RESULT};
use crate::protocol::{FEATURES, PROTOCOL, SYNC_PREFIX};
use bytes::BytesMut;
use serde_json::{json, Value};
use std::fs;
use std::io::{self, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

const VERSION: &str = env!("CARGO_PKG_VERSION");
const PING_INTERVAL: Duration = Duration::from_secs(5);

struct Server {
    cwd: PathBuf,
}

fn remote_error(code: &str, message: impl Into<String>) -> Value {
    json!({ "code": code, "message": message.into() })
}

impl Server {
    fn dispatch(&self, json: &Value, payload: &[u8]) -> Result<(Value, Vec<u8>), Value> {
        let op = json.get("op").and_then(Value::as_str).unwrap_or("");
        match op {
            "hello" => Ok((
                json!({
                    "protocol": PROTOCOL,
                    "version": VERSION,
                    "os": std::env::consts::OS,
                    "arch": std::env::consts::ARCH,
                    "home": home_dir(),
                    "separator": std::path::MAIN_SEPARATOR_STR,
                    "tmpdir": std::env::temp_dir().to_string_lossy(),
                    "cwd": self.cwd.to_string_lossy(),
                    "driveCwds": json!({}),
                    "pid": std::process::id(),
                    "features": FEATURES,
                }),
                Vec::new(),
            )),
            "read" => {
                let path = field(json, "path")?;
                let bytes = fs::read(path).map_err(|error| io_failure(&error, "read", path))?;
                Ok((json!({}), bytes))
            }
            "write" => {
                let path = field(json, "path")?;
                let append = json.get("append").and_then(Value::as_bool).unwrap_or(false);
                if append {
                    use std::io::Write as _;
                    let mut file = fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(path)
                        .map_err(|error| io_failure(&error, "write", path))?;
                    file.write_all(payload)
                        .map_err(|error| io_failure(&error, "write", path))?;
                } else if let Some(parent) = Path::new(path).parent() {
                    fs::create_dir_all(parent).ok();
                    fs::write(path, payload).map_err(|error| io_failure(&error, "write", path))?;
                } else {
                    fs::write(path, payload).map_err(|error| io_failure(&error, "write", path))?;
                }
                Ok((json!({}), Vec::new()))
            }
            "exec" => {
                let cwd = json
                    .get("cwd")
                    .and_then(Value::as_str)
                    .map(PathBuf::from)
                    .unwrap_or_else(|| self.cwd.clone());
                let command = field(json, "command")?;
                let output = Command::new("sh")
                    .arg("-lc")
                    .arg(command)
                    .current_dir(cwd)
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .output()
                    .map_err(|error| remote_error("spawn_error", error.to_string()))?;
                let mut combined = output.stdout;
                combined.extend_from_slice(&output.stderr);
                Ok((
                    json!({ "exitCode": output.status.code().unwrap_or(-1) }),
                    combined,
                ))
            }
            "listFiles" => list_files(json),
            _ => Err(remote_error("EINVAL", format!("unknown operation {op}"))),
        }
    }
}

fn field<'a>(json: &'a Value, key: &str) -> Result<&'a str, Value> {
    json.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| remote_error("EINVAL", format!("missing field {key}")))
}

fn io_failure(error: &std::io::Error, syscall: &str, path: &str) -> Value {
    let code = match error.kind() {
        io::ErrorKind::NotFound => "ENOENT",
        io::ErrorKind::PermissionDenied => "EACCES",
        _ => "unknown",
    };
    json!({
        "code": code,
        "message": error.to_string(),
        "syscall": syscall,
        "path": path,
    })
}

fn home_dir() -> String {
    std::env::var("HOME").unwrap_or_else(|_| "/".into())
}

fn list_files(json: &Value) -> Result<(Value, Vec<u8>), Value> {
    let path = field(json, "path")?;
    let root = Path::new(path);
    if !root.is_dir() {
        return Err(remote_error("ENOTDIR", format!("not a directory: {path}")));
    }
    let max_entries = json
        .get("maxEntries")
        .and_then(Value::as_u64)
        .unwrap_or(10_000) as usize;
    let mut count = 0usize;
    let mut truncated = false;
    let mut payload = Vec::new();
    walk_list(root, root, &mut payload, &mut count, max_entries, &mut truncated)?;
    Ok((json!({ "count": count, "truncated": truncated }), payload))
}

fn walk_list(
    root: &Path,
    dir: &Path,
    payload: &mut Vec<u8>,
    count: &mut usize,
    max: usize,
    truncated: &mut bool,
) -> Result<(), Value> {
    if *count >= max {
        *truncated = true;
        return Ok(());
    }
    let entries = fs::read_dir(dir).map_err(|error| io_failure(&error, "readdir", dir.to_str().unwrap_or("")))?;
    for entry in entries {
        let entry = entry.map_err(|error| io_failure(&error, "readdir", dir.to_str().unwrap_or("")))?;
        let file_type = entry.file_type().map_err(|error| {
            io_failure(&error, "lstat", entry.path().to_string_lossy().as_ref())
        })?;
        let path = entry.path();
        let rel = path
            .strip_prefix(root)
            .unwrap_or(path.as_path())
            .to_string_lossy()
            .replace('\\', "/");
        let kind_byte = if file_type.is_dir() {
            b'd'
        } else if file_type.is_symlink() {
            b'l'
        } else {
            b'f'
        };
        payload.push(kind_byte);
        payload.extend_from_slice(rel.as_bytes());
        payload.push(0);
        *count += 1;
        if file_type.is_dir() {
            walk_list(root, &entry.path(), payload, count, max, truncated)?;
        }
        if *count >= max {
            *truncated = true;
            return Ok(());
        }
    }
    Ok(())
}

fn write_frame(output: &mut impl Write, frame: &Frame) -> io::Result<()> {
    output.write_all(&frame.encode())?;
    output.flush()
}

/// Run the daemon on stdin/stdout until EOF.
pub fn serve(token: &str) -> io::Result<()> {
    {
        let mut stdout = io::stdout().lock();
        writeln!(stdout, "{SYNC_PREFIX} {token}")?;
        stdout.flush()?;
    }
    let server = Arc::new(Server {
        cwd: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
    });
    let output = Arc::new(Mutex::new(io::stdout()));
    {
        let output = output.clone();
        thread::spawn(move || loop {
            thread::sleep(PING_INTERVAL);
            let frame = Frame::new(PING, 0, json!({}));
            if write_frame(&mut *output.lock().unwrap(), &frame).is_err() {
                break;
            }
        });
    }
    let mut reader = BufReader::with_capacity(1 << 20, io::stdin().lock());
    let mut buffer = BytesMut::new();
    let mut read_buf = [0u8; 8192];
    loop {
        let read = reader.read(&mut read_buf)?;
        if read == 0 {
            break;
        }
        buffer.extend_from_slice(&read_buf[..read]);
        while let Some(request) = decode_frame(&mut buffer)? {
            if request.kind != REQUEST {
                continue;
            }
            let id = request.id;
            let reply = match server.dispatch(&request.json, &request.payload) {
                Ok((json, payload)) => Frame::with_payload(RESULT, id, json, payload),
                Err(error) => Frame::new(ERROR, id, error),
            };
            write_frame(&mut *output.lock().unwrap(), &reply)?;
        }
    }
    Ok(())
}
