use crate::error::{Error, Result};
use serde_json::{Value, json};
use std::{
    io::Read,
    os::unix::process::CommandExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

#[derive(Clone)]
pub struct Control {
    pub cancelled: Arc<AtomicBool>,
    pub deadline: Instant,
}
impl Default for Control {
    fn default() -> Self {
        Self {
            cancelled: Arc::new(AtomicBool::new(false)),
            deadline: Instant::now() + Duration::from_secs(60),
        }
    }
}
impl Control {
    pub fn check(&self) -> Result<()> {
        if self.cancelled.load(Ordering::Relaxed) || Instant::now() > self.deadline {
            Err(Error("TIMEOUT"))
        } else {
            Ok(())
        }
    }
}
pub trait Backend: Send + Sync {
    fn inspect(&self, path: &Path, control: &Control) -> Result<(Value, Vec<Value>)>;
    fn transform(
        &self,
        paths: &[PathBuf],
        pages: Option<&[u64]>,
        out: &Path,
        control: &Control,
    ) -> Result<Vec<Value>>;
}
pub struct Qpdf {
    pub executable: PathBuf,
}
fn bounded(mut r: impl Read, max: usize, overflow: Arc<AtomicBool>) -> Vec<u8> {
    let mut out = Vec::new();
    let mut b = [0; 8192];
    loop {
        match r.read(&mut b) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                if out.len() + n > max {
                    overflow.store(true, Ordering::Relaxed);
                    break;
                }
                out.extend_from_slice(&b[..n]);
            }
        }
    }
    out
}
impl Qpdf {
    fn run(&self, args: &[String], control: &Control) -> Result<(i32, Vec<u8>)> {
        control.check()?;
        let mut cmd = Command::new(&self.executable);
        cmd.args(args)
            .env_clear()
            .env("LANG", "C")
            .env("LC_ALL", "C")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        unsafe {
            cmd.pre_exec(|| {
                if libc::setsid() < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                for (res, max) in [
                    (libc::RLIMIT_CPU, 60),
                    (libc::RLIMIT_FSIZE, 250 * 1024 * 1024),
                    (libc::RLIMIT_NOFILE, 64),
                    (libc::RLIMIT_AS, 768 * 1024 * 1024),
                ] {
                    let r = libc::rlimit {
                        rlim_cur: max,
                        rlim_max: max,
                    };
                    if libc::setrlimit(res, &r) != 0 {
                        return Err(std::io::Error::last_os_error());
                    }
                }
                Ok(())
            });
        }
        let mut child = cmd.spawn().map_err(|_| Error("BACKEND_UNAVAILABLE"))?;
        let pid = child.id() as i32;
        let overflow = Arc::new(AtomicBool::new(false));
        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();
        let f = overflow.clone();
        let out = std::thread::spawn(move || bounded(stdout, 32 * 1024 * 1024, f));
        let f = overflow.clone();
        let err = std::thread::spawn(move || bounded(stderr, 65536, f));
        let status = loop {
            if control.check().is_err() || overflow.load(Ordering::Relaxed) {
                unsafe { libc::kill(-pid, libc::SIGKILL) };
                let _ = child.wait();
                let _ = out.join();
                let _ = err.join();
                return Err(Error(if overflow.load(Ordering::Relaxed) {
                    "LIMIT_EXCEEDED"
                } else {
                    "TIMEOUT"
                }));
            }
            match child.try_wait() {
                Ok(Some(s)) => break s,
                Ok(None) => std::thread::sleep(Duration::from_millis(10)),
                Err(_) => {
                    unsafe { libc::kill(-pid, libc::SIGKILL) };
                    let _ = child.wait();
                    let _ = out.join();
                    let _ = err.join();
                    return Err(Error("INTERNAL"));
                }
            }
        };
        // Kill descendants before joining pipe readers, even if the leader exited.
        unsafe { libc::kill(-pid, libc::SIGKILL) };
        let bytes = out.join().map_err(|_| Error("INTERNAL"))?;
        let _ = err.join();
        if overflow.load(Ordering::Relaxed) {
            return Err(Error("LIMIT_EXCEEDED"));
        }
        if status.code().is_none() {
            return Err(Error("LIMIT_EXCEEDED"));
        }
        Ok((status.code().unwrap(), bytes))
    }
    fn qualified(&self, control: &Control) -> Result<()> {
        let (code, bytes) = self.run(&["--version".into()], control)?;
        if code != 0 || !String::from_utf8_lossy(&bytes).starts_with("qpdf version 12.4.2\n") {
            return Err(Error("BACKEND_UNAVAILABLE"));
        }
        Ok(())
    }
}
fn warning(code: i32) -> Vec<Value> {
    if code == 3 {
        vec![
            json!({"code":"QPDF_WARNINGS","message":"qpdf reported structural warnings; appearance and signature trust are not checked."}),
        ]
    } else {
        vec![]
    }
}
fn walk(v: &Value, key: &str) -> bool {
    match v {
        Value::Object(m) => m.contains_key(key) || m.values().any(|v| walk(v, key)),
        Value::Array(a) => a.iter().any(|v| walk(v, key)),
        _ => false,
    }
}
fn sig(v: &Value) -> bool {
    match v {
        Value::Object(m) => {
            m.get("/Type").is_some_and(|x| x == "/Sig")
                || m.get("/FT").is_some_and(|x| x == "/Sig")
                || m.contains_key("/ByteRange")
                || m.contains_key("/Perms")
                || m.values().any(sig)
        }
        Value::Array(a) => a.iter().any(sig),
        _ => false,
    }
}
impl Backend for Qpdf {
    fn inspect(&self, path: &Path, control: &Control) -> Result<(Value, Vec<Value>)> {
        self.qualified(control)?;
        let path = path.to_str().ok_or(Error("ACCESS_DENIED"))?.to_string();
        let (encrypted, _) = self.run(&["--is-encrypted".into(), path.clone()], control)?;
        // qpdf --is-encrypted: 0 encrypted, 2 not encrypted, all other exits are errors.
        if encrypted == 0 {
            return Err(Error("ENCRYPTED_UNSUPPORTED"));
        }
        if encrypted != 2 {
            return Err(Error("INVALID_PDF"));
        }
        let (check, _) = self.run(&["--check".into(), path.clone()], control)?;
        if check != 0 && check != 3 {
            return Err(Error("INVALID_PDF"));
        }
        let (code, data) = self.run(
            &["--json=2".into(), "--json-stream-data=none".into(), path],
            control,
        )?;
        if code != 0 && code != 3 {
            return Err(Error("INVALID_PDF"));
        }
        let v: Value = serde_json::from_slice(&data).map_err(|_| Error("INVALID_PDF"))?;
        let pages = v["pages"]
            .as_array()
            .ok_or(Error("PRESERVATION_UNKNOWN"))?
            .len();
        if pages > 2000 {
            return Err(Error("LIMIT_EXCEEDED"));
        }
        let known = v["qpdf"]
            .as_array()
            .is_some_and(|a| a.len() >= 2 && a[1].is_object());
        let forms = if walk(&v["qpdf"], "/AcroForm")
            || walk(&v["qpdf"], "/XFA")
            || v["acroform"]["hasacroform"] == true
        {
            "present"
        } else if known && v["acroform"]["hasacroform"] == false {
            "absent"
        } else {
            "unknown"
        };
        let signatures = if sig(&v["qpdf"]) {
            "present"
        } else if known {
            "absent"
        } else {
            "unknown"
        };
        let warnings = warning(if check == 3 || code == 3 { 3 } else { 0 });
        Ok((
            json!({"pageCount":pages,"structuralStatus":if warnings.is_empty(){"valid"}else{"warnings"},"encrypted":false,"forms":forms,"signatures":signatures,"signatureValidity":"notChecked"}),
            warnings,
        ))
    }
    fn transform(
        &self,
        paths: &[PathBuf],
        pages: Option<&[u64]>,
        out: &Path,
        control: &Control,
    ) -> Result<Vec<Value>> {
        let mut args = vec![
            "--empty".into(),
            "--deterministic-id".into(),
            "--pages".into(),
        ];
        for path in paths {
            args.push(path.to_str().ok_or(Error("ACCESS_DENIED"))?.into());
            args.push(
                pages
                    .map(|p| p.iter().map(u64::to_string).collect::<Vec<_>>().join(","))
                    .unwrap_or("1-z".into()),
            );
        }
        args.push("--".into());
        args.push(out.to_str().ok_or(Error("ACCESS_DENIED"))?.into());
        let (code, _) = self.run(&args, control)?;
        if code != 0 && code != 3 {
            return Err(Error("INVALID_PDF"));
        }
        Ok(warning(code))
    }
}
