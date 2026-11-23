use crate::error::{Error, Result};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
#[cfg(target_os = "linux")]
use std::io::{Seek, SeekFrom};
use std::{
    io::{Read, Write},
    os::unix::fs::OpenOptionsExt,
    os::unix::process::CommandExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

/// One coarse progress event emitted through [`Control::report`]. Engines
/// and the dispatcher report named phases with an optional position within
/// a bounded series; front-ends (the CLI's stderr spinner) may render them
/// and agents may ignore them entirely.
#[derive(Debug, Clone)]
pub struct Progress {
    /// Operation phase, e.g. "staging", "inspect", "compress", "resample".
    pub phase: &'static str,
    /// Human detail line, e.g. "resampling images to 150 dpi".
    pub detail: String,
    /// Position within a bounded series (rung i of n) when known.
    pub step: Option<(u64, u64)>,
}

/// Front-end progress sink installed on [`Control`].
pub type ProgressSink = Arc<dyn Fn(&Progress) + Send + Sync>;

#[derive(Clone)]
pub struct Control {
    pub cancelled: Arc<AtomicBool>,
    pub deadline: Instant,
    /// Optional progress sink. Backends call `report()`; transports install
    /// it (or leave None — reporting is then a no-op).
    pub progress: Option<ProgressSink>,
}
impl Default for Control {
    fn default() -> Self {
        Self {
            cancelled: Arc::new(AtomicBool::new(false)),
            deadline: Instant::now() + Duration::from_secs(60),
            progress: None,
        }
    }
}
impl Control {
    pub fn check(&self) -> Result<()> {
        if self.cancelled.load(Ordering::Relaxed) || Instant::now() > self.deadline {
            Err(Error("TIMEOUT", None))
        } else {
            Ok(())
        }
    }
    /// Wall-clock budget left before the deadline; subprocess CPU limits
    /// derive from it so a worker never outlives its job.
    pub fn remaining(&self) -> Duration {
        self.deadline.saturating_duration_since(Instant::now())
    }
    /// Emit a coarse progress event to the optional sink installed by the
    /// calling front-end. Cheap no-op when no sink is installed.
    pub fn report(&self, phase: &'static str, detail: impl Into<String>, step: Option<(u64, u64)>) {
        if let Some(progress) = &self.progress {
            progress(&Progress {
                phase,
                detail: detail.into(),
                step,
            });
        }
    }
}
/// Wall-clock budget for a compress job: a 60 s base plus a per-MiB
/// allowance for image-resample passes, bounded at 10 minutes. Large
/// scanned documents need several minutes of resampling work that the flat
/// default deadline could never fit; the cap keeps the job bounded.
/// `ANPDF_JOB_DEADLINE_MS` overrides the budget for integration tests.
pub fn compress_deadline(input_bytes: u64) -> Duration {
    compress_deadline_targeted(input_bytes, false)
}
/// A size-targeted compress may run a second optimization phase — adaptive
/// page rasterization — after image resampling misses the target, so its
/// budget carries an extra bounded slice.
pub fn compress_deadline_targeted(input_bytes: u64, targeted: bool) -> Duration {
    if let Some(ms) = std::env::var("ANPDF_JOB_DEADLINE_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|v| *v > 0)
    {
        return Duration::from_millis(ms);
    }
    let extra = if targeted { 60 } else { 0 };
    Duration::from_secs(60 + input_bytes.div_ceil(1024 * 1024) * 20 + extra)
        .min(Duration::from_secs(600))
}
const MAX_JSON_OBJECTS: usize = 200_000;
const MAX_WALK_NODES: usize = 2_000_000;

#[cfg(target_os = "macos")]
pub(crate) fn child_rss_exceeded(pid: i32, limit: u64) -> bool {
    let mut info: libc::proc_taskinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<libc::proc_taskinfo>() as libc::c_int;
    let rc = unsafe {
        libc::proc_pidinfo(
            pid,
            libc::PROC_PIDTASKINFO,
            0,
            (&mut info as *mut libc::proc_taskinfo).cast::<libc::c_void>(),
            size,
        )
    };
    rc == size && info.pti_resident_size > limit
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn child_rss_exceeded(pid: i32, limit: u64) -> bool {
    // Linux: read VmRSS from procfs so the RLIMIT_AS clamp's tolerated
    // failure path still leaves the worker's memory bounded.
    let Ok(status) = std::fs::read_to_string(format!("/proc/{pid}/status")) else {
        return false;
    };
    status
        .lines()
        .find_map(|l| l.strip_prefix("VmRSS:"))
        .and_then(|v| v.trim().trim_end_matches("kB").trim().parse::<u64>().ok())
        .is_some_and(|kb| kb * 1024 > limit)
}

#[derive(Clone, Copy)]
pub enum CompressMode {
    Lossless,
    Lossy { jpeg_quality: u8 },
}

/// A secret handed to a spawned backend worker without crossing argv, the
/// process environment, or any loggable surface: on Linux an inherited
/// anonymous `memfd` descriptor (`/proc/self/fd/N`); elsewhere a
/// staging-local file opened private (0o600) at creation and unlinked on
/// drop. The worker's argv carries only the descriptor or staging path,
/// which names a channel meaningless outside the spawned process.
struct SecretChannel {
    arg: String,
    _fd: Option<std::fs::File>,
    staging_file: Option<PathBuf>,
}
impl SecretChannel {
    /// `dir` is the job's private staging directory (the input or output's
    /// parent); it holds the file fallback and must already be private.
    fn new(dir: &Path, secret: &[u8]) -> Result<Self> {
        #[cfg(target_os = "linux")]
        {
            use std::os::fd::FromRawFd;
            let fd = unsafe { libc::memfd_create(c"anpdf-secret".as_ptr(), 0) };
            if fd >= 0 {
                let mut file = unsafe { std::fs::File::from_raw_fd(fd) };
                file.write_all(secret).and_then(|_| file.write_all(b"\n"))?;
                file.seek(SeekFrom::Start(0))?;
                return Ok(Self {
                    arg: format!("/proc/self/fd/{fd}"),
                    _fd: Some(file),
                    staging_file: None,
                });
            }
            // Older kernels lack memfd: fall through to a private file.
        }
        let path = dir.join(".anpdf-secret");
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)?;
        file.write_all(secret).and_then(|_| file.write_all(b"\n"))?;
        file.sync_all()?;
        Ok(Self {
            arg: path
                .to_str()
                .ok_or(Error("ACCESS_DENIED", None))?
                .to_string(),
            _fd: Some(file),
            staging_file: Some(path),
        })
    }
    fn arg(&self) -> &str {
        &self.arg
    }
}
impl Drop for SecretChannel {
    fn drop(&mut self) {
        if let Some(path) = &self.staging_file {
            let _ = std::fs::remove_file(path);
        }
    }
}
/// A random, unguessable owner password for callers who omit one, inside
/// the 127-byte specification bound and never persisted anywhere.
pub fn generated_owner() -> Result<String> {
    let mut bytes = [0u8; 32];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}
fn invalid_password(stderr: &[u8]) -> bool {
    std::str::from_utf8(stderr).is_ok_and(|s| s.contains("invalid password"))
}

pub trait Backend: Send + Sync {
    fn inspect(
        &self,
        path: &Path,
        password: Option<&str>,
        control: &Control,
        deep: bool,
    ) -> Result<(Value, Vec<Value>)>;
    fn transform(
        &self,
        paths: &[PathBuf],
        pages: Option<&[u64]>,
        out: &Path,
        control: &Control,
    ) -> Result<Vec<Value>>;
    fn compress(
        &self,
        path: &Path,
        mode: CompressMode,
        out: &Path,
        control: &Control,
    ) -> Result<Vec<Value>>;
    fn encrypt(
        &self,
        input: &Path,
        out: &Path,
        user: &str,
        owner: &str,
        control: &Control,
    ) -> Result<Vec<Value>>;
    fn decrypt(
        &self,
        input: &Path,
        out: &Path,
        password: &str,
        control: &Control,
    ) -> Result<Vec<Value>>;
}

/// Raster pass used by the compression size ladder. One call covers the
/// whole search: the worker resamples oversized image XObjects at every
/// `rungs` (dpi cap, JPEG quality) pair, measures exact per-rung byte totals,
/// picks the gentlest rung fitting `target` (the hardest when none fits), and
/// saves once — the `Backend::compress` pass that follows only applies the
/// structural finish.
pub trait ImageResampler: Send + Sync {
    fn resample_images(
        &self,
        input: &Path,
        output: &Path,
        rungs: &[(u32, u8)],
        target: Option<u64>,
        control: &Control,
    ) -> Result<Value>;
}

pub const MAX_TEXT_RESPONSE_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_PNG_BYTES: u64 = 64 * 1024 * 1024;
pub const MAX_RENDER_SIDE: u32 = 8192;
pub const MAX_RENDER_PIXELS: u64 = 16 * 1024 * 1024;

pub fn validate_render_dimensions(width: u32, height: u32) -> Result<()> {
    if width == 0 || height == 0 {
        return Err(Error("INTERNAL", None));
    }
    if width > MAX_RENDER_SIDE
        || height > MAX_RENDER_SIDE
        || u64::from(width) * u64::from(height) > MAX_RENDER_PIXELS
    {
        return Err(Error("LIMIT_EXCEEDED", None));
    }
    Ok(())
}

/// Return the contract's ExtractedPage objects and warnings from private staged input.
/// Providers must bound worker output and honor the shared job deadline/cancellation.
pub trait ExtractTextBackend: Send + Sync {
    fn extract_text(
        &self,
        input: &Path,
        pages: &[u64],
        control: &Control,
    ) -> Result<(Vec<Value>, Vec<Value>)>;
}

/// Write a PNG at the private staging path and return warnings, not a workspace path.
/// Enforce dimensions before allocating pixels; finish any worker before returning.
pub trait RenderPageBackend: Send + Sync {
    fn render_page(
        &self,
        input: &Path,
        page: u64,
        dpi: u32,
        output: &Path,
        control: &Control,
    ) -> Result<Vec<Value>>;
}

/// Rebuild eligible pages as raster JPEGs at the gentlest rung fitting
/// `target`, editing the input document in place into `output`. Same isolation
/// rules as the resample worker: one bounded subprocess, staging-only paths,
/// summary JSON on stdout. Pages carrying text, annotations, layers or other
/// protected content keep their original objects.
pub trait PageRasterizer: Send + Sync {
    fn rasterize_document(
        &self,
        input: &Path,
        output: &Path,
        rungs: &[(u32, u8)],
        target: Option<u64>,
        control: &Control,
    ) -> Result<Value>;
}

pub struct Qpdf {
    pub executable: PathBuf,
    pub exact: bool,
    /// Address-space bound for spawned workers; the resample worker needs
    /// more than qpdf since it holds decoded pixels and encoded rungs.
    pub as_limit: libc::rlim_t,
    /// CPU budget multiplier: parallel workers burn CPU across threads, so
    /// the resample worker gets remaining × its thread count.
    pub cpu_scale: u64,
}

pub(crate) fn limit_worker_as(
    cmd: &mut Command,
    file_limit: libc::rlim_t,
    cpu_secs: u64,
    as_limit: libc::rlim_t,
) {
    unsafe {
        cmd.pre_exec(move || {
            if libc::setsid() < 0 {
                return Err(std::io::Error::last_os_error());
            }
            for (res, max) in [(libc::RLIMIT_FSIZE, file_limit), (libc::RLIMIT_NOFILE, 64)] {
                let r = libc::rlimit {
                    rlim_cur: max,
                    rlim_max: max,
                };
                if libc::setrlimit(res, &r) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
            }
            // A hard cap inherited from the parent cannot be raised; clamp
            // address space and CPU to it (and tolerate failure) rather than
            // fail the spawn — the parent's deadline/RSS poll still bounds
            // the worker.
            let mut cur: libc::rlimit = std::mem::zeroed();
            #[cfg(not(target_os = "macos"))]
            {
                let mut as_cap = as_limit;
                if libc::getrlimit(libc::RLIMIT_AS, &mut cur) == 0
                    && cur.rlim_max != libc::RLIM_INFINITY
                {
                    as_cap = as_cap.min(cur.rlim_max);
                }
                let r = libc::rlimit {
                    rlim_cur: as_cap,
                    rlim_max: as_cap,
                };
                let _ = libc::setrlimit(libc::RLIMIT_AS, &r);
            }
            #[cfg(target_os = "macos")]
            let _ = as_limit;
            let mut cpu_cap = cpu_secs as libc::rlim_t;
            if libc::getrlimit(libc::RLIMIT_CPU, &mut cur) == 0
                && cur.rlim_max != libc::RLIM_INFINITY
            {
                cpu_cap = cpu_cap.min(cur.rlim_max);
            }
            let r = libc::rlimit {
                rlim_cur: cpu_cap,
                rlim_max: cpu_cap,
            };
            let _ = libc::setrlimit(libc::RLIMIT_CPU, &r);
            Ok(())
        });
    }
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
    pub(crate) fn run(
        &self,
        args: &[String],
        control: &Control,
    ) -> Result<(i32, Vec<u8>, Vec<u8>)> {
        control.check()?;
        let mut cmd = Command::new(&self.executable);
        cmd.args(args)
            .env_clear()
            .env("LANG", "C")
            .env("LC_ALL", "C")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        // The worker's CPU budget tracks the job's remaining deadline: a
        // resample pass on a large scan needs far more CPU than the old flat
        // 60 s, and the parent still kills the process at the deadline.
        let cpu_secs = control
            .remaining()
            .as_secs()
            .saturating_mul(self.cpu_scale.max(1))
            .saturating_add(30)
            .clamp(30, 600);
        limit_worker_as(&mut cmd, 250 * 1024 * 1024, cpu_secs, self.as_limit);
        let mut child = cmd
            .spawn()
            .map_err(|_| Error("BACKEND_UNAVAILABLE", None))?;
        let pid = child.id() as i32;
        let overflow = Arc::new(AtomicBool::new(false));
        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();
        let f = overflow.clone();
        let out = std::thread::spawn(move || bounded(stdout, 32 * 1024 * 1024, f));
        let f = overflow.clone();
        let err = std::thread::spawn(move || bounded(stderr, 65536, f));
        let status = loop {
            let mem_exceeded = child_rss_exceeded(pid, self.as_limit);
            if control.check().is_err() || overflow.load(Ordering::Relaxed) || mem_exceeded {
                unsafe { libc::kill(-pid, libc::SIGKILL) };
                let _ = child.wait();
                let _ = out.join();
                let _ = err.join();
                return Err(Error(
                    if mem_exceeded || overflow.load(Ordering::Relaxed) {
                        "LIMIT_EXCEEDED"
                    } else {
                        "TIMEOUT"
                    },
                    None,
                ));
            }
            match child.try_wait() {
                Ok(Some(s)) => break s,
                Ok(None) => std::thread::sleep(Duration::from_millis(10)),
                Err(_) => {
                    unsafe { libc::kill(-pid, libc::SIGKILL) };
                    let _ = child.wait();
                    let _ = out.join();
                    let _ = err.join();
                    return Err(Error("INTERNAL", None));
                }
            }
        };
        // Kill descendants before joining pipe readers, even if the leader exited.
        unsafe { libc::kill(-pid, libc::SIGKILL) };
        let bytes = out.join().map_err(|_| Error("INTERNAL", None))?;
        let err_bytes = err.join().unwrap_or_default();
        if overflow.load(Ordering::Relaxed) {
            return Err(Error("LIMIT_EXCEEDED", None));
        }
        if status.code().is_none() {
            return Err(Error("LIMIT_EXCEEDED", None));
        }
        Ok((status.code().unwrap(), bytes, err_bytes))
    }
    /// Run one qpdf invocation described by a job JSON document. The
    /// document — which may carry passwords — reaches the worker through
    /// an inherited descriptor or private staging file; argv stays clean
    /// and secrets survive line breaks (a `--password-file` reads only
    /// its first line).
    fn run_job(
        &self,
        dir: &Path,
        job: &Value,
        control: &Control,
    ) -> Result<(i32, Vec<u8>, Vec<u8>)> {
        let bytes = serde_json::to_vec(job).map_err(|_| Error("INTERNAL", None))?;
        let channel = SecretChannel::new(dir, &bytes)?;
        self.run(&[format!("--job-json-file={}", channel.arg())], control)
    }
    fn qualified(&self, control: &Control) -> Result<Vec<Value>> {
        let (code, bytes, _) = self.run(&["--version".into()], control)?;
        let exact = self.exact || std::env::var_os("ANPDF_QPDF_EXACT").is_some_and(|v| v == "1");
        let version = qpdf_version(&bytes).ok_or(Error("BACKEND_UNAVAILABLE", None))?;
        if code != 0 || !supported_version(version, exact) {
            return Err(Error("BACKEND_UNAVAILABLE", None));
        }
        Ok(if version == (12, 4, 2) {
            vec![]
        } else {
            vec![
                json!({"code":"BACKEND_VERSION_UNTESTED","message":"This qpdf 12.x version is accepted but only 12.4.2 is qualified."}),
            ]
        })
    }
    fn preservation_evidence(
        &self,
        v: &Value,
        path: &str,
        password: Option<&str>,
        control: &Control,
    ) -> Result<(bool, Value)> {
        let dir = Path::new(path)
            .parent()
            .ok_or(Error("ACCESS_DENIED", None))?;
        let objs = &v["qpdf"][1];
        let trailer = &objs["trailer"]["value"];
        let info = trailer
            .get("/Info")
            .map(|i| i.as_str().and_then(|r| resolve_obj(objs, r)).unwrap_or(i));
        let catalog = trailer["/Root"].as_str().and_then(|r| resolve_obj(objs, r));
        let xmp = catalog.and_then(|c| c.get("/Metadata"));
        let mut evidence = json!({});
        let mut metadata = vec![info.cloned().unwrap_or(Value::Null)];
        let mut metadata_known = !walk_any(&json!(info), |m| {
            m.values().any(|v| {
                v.as_str()
                    .is_some_and(|s| s.ends_with(" R") && !s.starts_with("u:"))
            })
        })?;
        if let Some(xmp) = xmp {
            if let Some(reference) = xmp.as_str().and_then(|r| r.strip_suffix(" R")) {
                let mut job = json!({
                    "inputFile": path,
                    "showObject": reference.replace(' ', ","),
                    "filteredStreamData": "",
                });
                if let Some(p) = password {
                    job["password"] = p.into();
                }
                let (code, bytes, _) = self.run_job(dir, &job, control)?;
                if code == 0 {
                    metadata.push(json!(hex_hash(&bytes)));
                } else {
                    metadata_known = false;
                }
            } else {
                metadata_known = false;
            }
        }
        if metadata_known {
            evidence["metadata"] = json!(hex_hash(
                &serde_json::to_vec(&metadata).map_err(|_| Error("INTERNAL", None))?
            ));
        }
        if let Some(outlines) = normalized_outlines(&v["outlines"], &v["pages"]) {
            evidence["bookmarks"] =
                json!(serde_json::to_string(&outlines).map_err(|_| Error("INTERNAL", None))?);
        }
        if let Some(attachments) = v["attachments"].as_object() {
            let mut normalized = json!({});
            let mut known = true;
            for (key, attachment) in attachments {
                if attachment["streams"]
                    .as_object()
                    .is_none_or(|m| m.len() != 1)
                {
                    known = false;
                    break;
                }
                let mut job = json!({"inputFile": path, "showAttachment": key});
                if let Some(p) = password {
                    job["password"] = p.into();
                }
                let extracted = self.run_job(dir, &job, control);
                control.check()?;
                let Ok((0, bytes, _)) = extracted else {
                    known = false;
                    break;
                };
                let mut detail = attachment.clone();
                if let Some(map) = detail.as_object_mut() {
                    map.remove("filespec");
                    map.remove("preferredcontents");
                }
                normalized[key] = json!({"detail":detail,"sha256":hex_hash(&bytes)});
            }
            if known {
                evidence["attachments"] = json!(hex_hash(
                    &serde_json::to_vec(&normalized).map_err(|_| Error("INTERNAL", None))?
                ));
            }
        }
        Ok((info.is_some() || xmp.is_some(), evidence))
    }
}
fn hex_hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn normalized_outlines(value: &Value, pages: &Value) -> Option<Value> {
    let mut outlines = value.as_array()?.clone();
    for outline in &mut outlines {
        let map = outline.as_object_mut()?;
        map.remove("object");
        if let Some(dest) = map.get_mut("dest").and_then(Value::as_array_mut) {
            let reference = dest.first()?.as_str()?;
            let page = pages
                .as_array()?
                .iter()
                .find(|p| p["object"] == reference)?;
            dest[0] = page["pageposfrom1"].clone();
        }
        map.insert("kids".into(), normalized_outlines(map.get("kids")?, pages)?);
    }
    Some(Value::Array(outlines))
}
fn qpdf_version(bytes: &[u8]) -> Option<(u64, u64, u64)> {
    let text = std::str::from_utf8(bytes).ok()?;
    let version = text.lines().next()?.strip_prefix("qpdf version ")?;
    let parts = version.split('.').collect::<Vec<_>>();
    if parts.len() != 3
        || parts
            .iter()
            .any(|p| p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit()))
    {
        return None;
    }
    Some((
        parts[0].parse().ok()?,
        parts[1].parse().ok()?,
        parts[2].parse().ok()?,
    ))
}
fn supported_version(version: (u64, u64, u64), exact: bool) -> bool {
    version == (12, 4, 2) || (!exact && version.0 == 12 && version >= (12, 4, 2))
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
fn walk_any(
    root: &Value,
    mut pred: impl FnMut(&serde_json::Map<String, Value>) -> bool,
) -> Result<bool> {
    let mut stack = vec![root];
    let mut visited = 0usize;
    while let Some(node) = stack.pop() {
        visited += 1;
        if visited > MAX_WALK_NODES {
            return Err(Error("LIMIT_EXCEEDED", None));
        }
        match node {
            Value::Object(m) => {
                if pred(m) {
                    return Ok(true);
                }
                stack.extend(m.values());
            }
            Value::Array(a) => stack.extend(a.iter()),
            _ => {}
        }
    }
    Ok(false)
}
fn walk(v: &Value, key: &str) -> Result<bool> {
    walk_any(v, |m| m.contains_key(key))
}
fn sig(v: &Value) -> Result<bool> {
    walk_any(v, |m| {
        // The /Encrypt dictionary also carries /Perms (encrypted permission
        // bytes) — its /Filter names a security handler, never a signature.
        let encryption_dict = m.get("/Filter").is_some_and(|x| x == "/Standard");
        !encryption_dict
            && (m.get("/Type").is_some_and(|x| x == "/Sig")
                || m.get("/FT").is_some_and(|x| x == "/Sig")
                || m.contains_key("/ByteRange")
                || m.contains_key("/Perms"))
    })
}
fn resolve_obj<'a>(objs: &'a Value, r: &str) -> Option<&'a Value> {
    objs.get(format!("obj:{r}")).and_then(|o| o.get("value"))
}
fn inherited_attr<'a>(objs: &'a Value, start: &'a Value, key: &str) -> Option<&'a Value> {
    let mut cur = Some(start);
    let mut seen = Vec::new();
    for _ in 0..32 {
        let dict = cur?;
        if let Some(v) = dict.get(key) {
            return Some(v);
        }
        let parent_ref = dict.get("/Parent").and_then(Value::as_str)?;
        if seen.contains(&parent_ref) {
            return None;
        }
        seen.push(parent_ref);
        cur = resolve_obj(objs, parent_ref);
    }
    None
}
fn qpdf_text(v: &Value) -> Option<String> {
    let s = v.as_str()?;
    if let Some(u) = s.strip_prefix("u:") {
        Some(u.to_string())
    } else if s.starts_with("b:") {
        None
    } else {
        Some(s.to_string())
    }
}
fn extract_metadata(objs: &Value) -> Value {
    let mut map = serde_json::Map::new();
    let info = match objs
        .get("trailer")
        .and_then(|t| t.get("value"))
        .and_then(|t| t.get("/Info"))
    {
        Some(Value::String(r)) => resolve_obj(objs, r),
        Some(v @ Value::Object(_)) => Some(v),
        _ => None,
    };
    if let Some(info) = info {
        for (pdf_key, json_key) in [
            ("/Title", "title"),
            ("/Author", "author"),
            ("/Subject", "subject"),
            ("/Creator", "creator"),
            ("/Producer", "producer"),
        ] {
            if let Some(text) = info.get(pdf_key).and_then(qpdf_text) {
                map.insert(json_key.to_string(), Value::String(text));
            }
        }
    }
    Value::Object(map)
}
fn extract_pages(v: &Value, objs: &Value) -> Vec<Value> {
    let Some(pages) = v["pages"].as_array() else {
        return vec![];
    };
    let mut out = Vec::with_capacity(pages.len().min(2000));
    for (idx, page) in pages.iter().take(2000).enumerate() {
        let num = page["pageposfrom1"].as_u64().unwrap_or((idx + 1) as u64);
        let mut entry = serde_json::Map::new();
        entry.insert("pageNumber".into(), json!(num));
        let dict = page["object"].as_str().and_then(|r| resolve_obj(objs, r));
        if let Some(dict) = dict {
            let annots = dict.get("/Annots");
            let annots =
                annots.and_then(|a| a.as_str().and_then(|r| resolve_obj(objs, r)).or(Some(a)));
            if annots.is_none() || annots.is_some_and(Value::is_array) {
                entry.insert(
                    "annotationCount".into(),
                    json!(annots.and_then(Value::as_array).map_or(0, Vec::len)),
                );
            }
        }
        let rot = dict
            .and_then(|d| inherited_attr(objs, d, "/Rotate"))
            .and_then(Value::as_i64)
            .map(|r| r.rem_euclid(360))
            .filter(|r| matches!(r, 0 | 90 | 180 | 270))
            .unwrap_or(0);
        entry.insert("rotation".into(), json!(rot));
        if let Some(arr) = dict
            .and_then(|d| inherited_attr(objs, d, "/MediaBox"))
            .and_then(Value::as_array)
            .filter(|a| a.len() == 4)
            && let (Some(x0), Some(y0), Some(x1), Some(y1)) = (
                arr[0].as_f64(),
                arr[1].as_f64(),
                arr[2].as_f64(),
                arr[3].as_f64(),
            )
        {
            let w = (x1 - x0).abs();
            let h = (y1 - y0).abs();
            if w.is_finite() && h.is_finite() {
                entry.insert("widthPoints".into(), json!(w));
                entry.insert("heightPoints".into(), json!(h));
            }
        }
        if let Some(label) = page["label"].as_str() {
            entry.insert("label".into(), json!(label));
        }
        out.push(Value::Object(entry));
    }
    out
}
fn inspect_json(v: &Value, warn_code: i32) -> Result<(Value, Vec<Value>)> {
    let pages = v["pages"]
        .as_array()
        .ok_or(Error("PRESERVATION_UNKNOWN", None))?
        .len();
    if pages > 2000 {
        return Err(Error("LIMIT_EXCEEDED", None));
    }
    let objs = v["qpdf"]
        .as_array()
        .and_then(|a| a.get(1))
        .unwrap_or(&Value::Null);
    if objs.as_object().is_some_and(|m| m.len() > MAX_JSON_OBJECTS) {
        return Err(Error("LIMIT_EXCEEDED", None));
    }
    let known = objs.is_object();
    let pdf_version = v["qpdf"]
        .as_array()
        .and_then(|a| a.first())
        .and_then(|m| m["pdfversion"].as_str())
        .unwrap_or("");
    let forms = if walk(&v["qpdf"], "/AcroForm")?
        || walk(&v["qpdf"], "/XFA")?
        || v["acroform"]["hasacroform"] == true
    {
        "present"
    } else if known && v["acroform"]["hasacroform"] == false {
        "absent"
    } else {
        "unknown"
    };
    let signatures = if sig(&v["qpdf"])? {
        "present"
    } else if known {
        "absent"
    } else {
        "unknown"
    };
    let actions = if walk(&v["qpdf"], "/OpenAction")? || walk(&v["qpdf"], "/AA")? {
        "present"
    } else if known {
        "absent"
    } else {
        "unknown"
    };
    let has_outline = v["outlines"].as_array().is_some_and(|a| !a.is_empty());
    let has_attachments = v["attachments"].as_object().is_some_and(|m| !m.is_empty());
    let metadata = extract_metadata(objs);
    let page_infos = extract_pages(v, objs);
    let warnings = warning(warn_code);
    Ok((
        json!({
            "pageCount": pages,
            "pdfVersion": pdf_version,
            "structuralStatus": if warnings.is_empty() { "valid" } else { "warnings" },
            "encrypted": v["encrypt"]["encrypted"].as_bool() == Some(true),
            "forms": forms,
            "signatures": signatures,
            "signatureValidity": "notChecked",
            "actions": actions,
            "hasOutline": has_outline,
            "hasAttachments": has_attachments,
            "metadata": metadata,
            "pages": page_infos,
        }),
        warnings,
    ))
}
impl Backend for Qpdf {
    fn inspect(
        &self,
        path: &Path,
        password: Option<&str>,
        control: &Control,
        deep: bool,
    ) -> Result<(Value, Vec<Value>)> {
        let mut warnings = self.qualified(control)?;
        let path = path
            .to_str()
            .ok_or(Error("ACCESS_DENIED", None))?
            .to_string();
        // The password reaches qpdf inside a job JSON document carried on
        // an inherited descriptor or private staging file — argv stays
        // clean and line breaks in the password survive (a password file
        // would carry only the first line).
        let dir = Path::new(&path)
            .parent()
            .ok_or(Error("ACCESS_DENIED", None))?;
        let mut job = json!({
            "inputFile": path,
            "json": "latest",
            "jsonStreamData": "none",
        });
        if let Some(p) = password {
            job["password"] = p.into();
        }
        let (code, data, stderr) = self.run_job(dir, &job, control)?;
        if code != 0 && code != 3 {
            if invalid_password(&stderr) {
                // No password supplied means the document is protected, not
                // that a supplied secret was wrong.
                return Err(Error(
                    if password.is_none() {
                        "ENCRYPTED_UNSUPPORTED"
                    } else {
                        "INVALID_PASSWORD"
                    },
                    None,
                ));
            }
            // A rejected dump is either an invalid document or one needing a
            // password; --is-encrypted distinguishes (0 encrypted, 2 not).
            let (encrypted, _, _) = self.run(&["--is-encrypted".into(), path.clone()], control)?;
            return Err(Error(
                if encrypted == 0 {
                    "ENCRYPTED_UNSUPPORTED"
                } else {
                    "INVALID_PDF"
                },
                None,
            ));
        }
        let v: Value = serde_json::from_slice(&data).map_err(|_| Error("INVALID_PDF", None))?;
        if v["encrypt"]["encrypted"].as_bool() == Some(true) && password.is_none() {
            return Err(Error("ENCRYPTED_UNSUPPORTED", None));
        }
        // `deep` re-runs qpdf --check, which decodes and validates every
        // stream payload — the slow part on large documents. The default
        // pass parses the object graph only; the dispatcher discloses that
        // scope to callers.
        let mut warn_code = if code == 3 { 3 } else { 0 };
        if deep {
            let mut check_job = json!({"inputFile": path, "check": ""});
            if let Some(p) = password {
                check_job["password"] = p.into();
            }
            let (check, _, check_err) = self.run_job(dir, &check_job, control)?;
            if check != 0 && check != 3 {
                return Err(Error(
                    if invalid_password(&check_err) {
                        "INVALID_PASSWORD"
                    } else {
                        "INVALID_PDF"
                    },
                    None,
                ));
            }
            if check == 3 {
                warn_code = 3;
            }
        }
        let (mut report, w) = inspect_json(&v, warn_code)?;
        // --check rejects degenerate documents the dump tolerates; a page
        // tree with zero pages fails there, so keep the shallow pass equal.
        if report["pageCount"].as_u64() == Some(0) {
            return Err(Error("INVALID_PDF", None));
        }
        let (has_metadata, evidence) = self.preservation_evidence(&v, &path, password, control)?;
        report["hasMetadata"] = json!(has_metadata);
        report["preservationEvidence"] = evidence;
        warnings.extend(w);
        Ok((report, warnings))
    }
    fn transform(
        &self,
        paths: &[PathBuf],
        pages: Option<&[u64]>,
        out: &Path,
        control: &Control,
    ) -> Result<Vec<Value>> {
        let mut args = vec![
            paths
                .first()
                .ok_or(Error("INVALID_REQUEST", None))?
                .to_str()
                .ok_or(Error("ACCESS_DENIED", None))?
                .into(),
            "--deterministic-id".into(),
            "--pages".into(),
        ];
        for path in paths {
            args.push(path.to_str().ok_or(Error("ACCESS_DENIED", None))?.into());
            args.push(
                pages
                    .map(|p| p.iter().map(u64::to_string).collect::<Vec<_>>().join(","))
                    .unwrap_or("1-z".into()),
            );
        }
        args.push("--".into());
        args.push(out.to_str().ok_or(Error("ACCESS_DENIED", None))?.into());
        let (code, _, _) = self.run(&args, control)?;
        if code != 0 && code != 3 {
            return Err(Error("INVALID_PDF", None));
        }
        Ok(warning(code))
    }
    fn compress(
        &self,
        path: &Path,
        mode: CompressMode,
        out: &Path,
        control: &Control,
    ) -> Result<Vec<Value>> {
        let mut args = vec![
            "--deterministic-id".into(),
            "--compress-streams=y".into(),
            "--recompress-flate".into(),
            "--compression-level=9".into(),
            "--object-streams=generate".into(),
            "--remove-unreferenced-resources=yes".into(),
        ];
        if let CompressMode::Lossy { jpeg_quality } = mode {
            args.push("--optimize-images".into());
            args.push(format!("--jpeg-quality={jpeg_quality}"));
        }
        args.push("--".into());
        args.push(path.to_str().ok_or(Error("ACCESS_DENIED", None))?.into());
        args.push(out.to_str().ok_or(Error("ACCESS_DENIED", None))?.into());
        let (code, _, _) = self.run(&args, control)?;
        if code != 0 && code != 3 {
            return Err(Error("INVALID_PDF", None));
        }
        Ok(warning(code))
    }
    fn encrypt(
        &self,
        input: &Path,
        out: &Path,
        user: &str,
        owner: &str,
        control: &Control,
    ) -> Result<Vec<Value>> {
        let mut warnings = self.qualified(control)?;
        let dir = out.parent().ok_or(Error("ACCESS_DENIED", None))?;
        // The job JSON carries both passwords; it reaches qpdf as an
        // inherited descriptor or private staging file — argv stays clean.
        let (code, _, stderr) = self.run_job(
            dir,
            &json!({
                "inputFile": input.to_str().ok_or(Error("ACCESS_DENIED", None))?,
                "outputFile": out.to_str().ok_or(Error("ACCESS_DENIED", None))?,
                "encrypt": {
                    "userPassword": user,
                    "ownerPassword": owner,
                    "256bit": {}
                }
            }),
            control,
        )?;
        if code != 0 && code != 3 {
            return Err(Error(
                if invalid_password(&stderr) {
                    "INVALID_PASSWORD"
                } else {
                    "INVALID_PDF"
                },
                None,
            ));
        }
        warnings.extend(warning(code));
        Ok(warnings)
    }
    fn decrypt(
        &self,
        input: &Path,
        out: &Path,
        password: &str,
        control: &Control,
    ) -> Result<Vec<Value>> {
        let mut warnings = self.qualified(control)?;
        let dir = out.parent().ok_or(Error("ACCESS_DENIED", None))?;
        let (code, _, stderr) = self.run_job(
            dir,
            &json!({
                "inputFile": input.to_str().ok_or(Error("ACCESS_DENIED", None))?,
                "outputFile": out.to_str().ok_or(Error("ACCESS_DENIED", None))?,
                "password": password,
                "decrypt": "",
            }),
            control,
        )?;
        if code != 0 && code != 3 {
            return Err(Error(
                if invalid_password(&stderr) {
                    "INVALID_PASSWORD"
                } else {
                    "INVALID_PDF"
                },
                None,
            ));
        }
        warnings.extend(warning(code));
        Ok(warnings)
    }
}

#[cfg(test)]
mod tests {
    use super::{extract_metadata, extract_pages};
    use serde_json::{Value, json};

    #[test]
    fn attachment_evidence_is_best_effort_with_bounded_processes() {
        use super::{Control, Qpdf};
        use std::{
            fs,
            os::unix::fs::PermissionsExt,
            time::{Duration, Instant},
        };

        let root = tempfile::tempdir().unwrap();
        let executable = root.path().join("backend");
        fs::write(&executable, "#!/bin/sh\nprintf synthetic\n").unwrap();
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
        let qpdf = Qpdf {
            executable: executable.clone(),
            exact: false,
            as_limit: 768 * 1024 * 1024,
            cpu_scale: 1,
        };
        let report = json!({"outlines":[], "pages":[], "attachments":{"sample.txt":{"streams":{"/F":"1 0 R"}}}, "qpdf":[{},{}]});
        let (_, evidence) = qpdf
            .preservation_evidence(&report, "input.pdf", None, &Control::default())
            .unwrap();
        assert!(evidence["attachments"].is_string());
        for script in [
            "#!/bin/sh\ndd if=/dev/zero bs=1048576 count=33 2>/dev/null\n",
            "#!/bin/sh\nexit 2\n",
        ] {
            fs::write(&executable, script).unwrap();
            let (_, evidence) = qpdf
                .preservation_evidence(&report, "input.pdf", None, &Control::default())
                .unwrap();
            assert!(evidence.get("attachments").is_none());
        }
        let empty = json!({"attachments":{}, "outlines":[], "pages":[], "qpdf":[{},{}]});
        assert!(
            qpdf.preservation_evidence(&empty, "input.pdf", None, &Control::default())
                .unwrap()
                .1["attachments"]
                .is_string()
        );
        fs::write(&executable, "#!/bin/sh\nexec sleep 10\n").unwrap();
        let control = Control {
            deadline: Instant::now() + Duration::from_millis(40),
            ..Control::default()
        };
        let start = Instant::now();
        assert_eq!(
            qpdf.preservation_evidence(&report, "input.pdf", None, &control)
                .unwrap_err()
                .0,
            "TIMEOUT"
        );
        assert!(start.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn version_range_and_strict_mode() {
        use super::{qpdf_version, supported_version};
        for (text, expected) in [
            ("12.4.1", false),
            ("12.4.2", true),
            ("12.4.10", true),
            ("12.10.0", true),
            ("13.0.0", false),
            ("11.9.9", false),
        ] {
            let version =
                qpdf_version(format!("qpdf version {text}\nRun qpdf --copyright\n").as_bytes())
                    .unwrap();
            assert_eq!(supported_version(version, false), expected);
            assert_eq!(supported_version(version, true), text == "12.4.2");
        }
        for text in [
            "",
            "12.4.2",
            "qpdf version 12.4",
            "qpdf version 12.4.2.1",
            "qpdf version 12.4.2-dev",
            "qpdf version +12.4.2",
            "qpdf version 12.4.2 extra",
            "qpdf version 99999999999999999999999.4.2",
        ] {
            assert!(qpdf_version(text.as_bytes()).is_none(), "{text}");
        }
    }

    #[test]
    fn annotation_counts_resolve_arrays_and_leave_unknown_unreported() {
        let v = json!({"pages":[{"object":"1 0 R"},{"object":"2 0 R"},{"object":"3 0 R"},{"object":"4 0 R"}]});
        let objs = json!({
            "obj:1 0 R":{"value":{"/Annots":["8 0 R","9 0 R"]}},
            "obj:2 0 R":{"value":{"/Annots":"7 0 R"}},
            "obj:3 0 R":{"value":{"/Annots":false}},
            "obj:4 0 R":{"value":{}},
            "obj:7 0 R":{"value":["9 0 R"]}
        });
        let pages = extract_pages(&v, &objs);
        assert_eq!(pages[0]["annotationCount"], 2);
        assert_eq!(pages[1]["annotationCount"], 1);
        assert!(pages[2].get("annotationCount").is_none());
        assert_eq!(pages[3]["annotationCount"], 0);
    }

    #[test]
    fn metadata_and_inherited_page_geometry() {
        let v = json!({
            "pages": [
                {"object": "3 0 R", "pageposfrom1": 1, "label": "i"},
                {"object": "4 0 R", "pageposfrom1": 2, "label": null}
            ],
            "qpdf": [
                {"pdfversion": "1.7"},
                {
                    "obj:2 0 R": {"value": {"/Type": "/Pages", "/MediaBox": [0, 0, 612, 792], "/Rotate": -90}},
                    "obj:3 0 R": {"value": {"/Type": "/Page", "/Parent": "2 0 R"}},
                    "obj:4 0 R": {"value": {"/Type": "/Page", "/Parent": "2 0 R", "/MediaBox": [10, 20, 310, 420], "/Rotate": 180}},
                    "obj:5 0 R": {"value": {"/Title": "u:Report", "/Author": "u:Alice", "/Subject": "b:deadbeef", "/Producer": "anPDF"}},
                    "trailer": {"value": {"/Info": "5 0 R"}}
                }
            ]
        });
        let objs = &v["qpdf"][1];
        assert_eq!(
            extract_metadata(objs),
            json!({"title": "Report", "author": "Alice", "producer": "anPDF"})
        );
        assert_eq!(
            extract_pages(&v, objs),
            vec![
                json!({"pageNumber": 1, "rotation": 270, "widthPoints": 612.0, "heightPoints": 792.0, "label": "i", "annotationCount":0}),
                json!({"pageNumber": 2, "rotation": 180, "widthPoints": 300.0, "heightPoints": 400.0, "annotationCount":0})
            ]
        );
    }

    #[test]
    fn circular_parent_does_not_loop() {
        let v = json!({
            "pages": [{"object": "1 0 R", "pageposfrom1": 1, "label": null}],
            "qpdf": [{}, {"obj:1 0 R": {"value": {"/Parent": "2 0 R"}}, "obj:2 0 R": {"value": {"/Parent": "1 0 R"}}}]
        });
        assert_eq!(
            extract_pages(&v, &v["qpdf"][1]),
            vec![json!({"pageNumber": 1, "rotation": 0, "annotationCount":0})]
        );
    }

    #[test]
    fn deep_nesting_nonfinite_mediabox_and_object_bomb_limits() {
        use super::{MAX_JSON_OBJECTS, inspect_json, walk};
        let mut nested = json!({"/Sig": true});
        for _ in 0..300 {
            nested = json!({"child": nested});
        }
        assert!(walk(&nested, "/Sig").unwrap());

        let huge_box = json!({
            "pages": [{"object": "1 0 R", "pageposfrom1": 1, "label": null}],
            "acroform": {"hasacroform": false},
            "outlines": [],
            "attachments": {},
            "qpdf": [
                {"pdfversion": "1.7"},
                {"obj:1 0 R": {"value": {"/Type": "/Page", "/MediaBox": [-1e308, 0.0, 1e308, 100.0]}}}
            ]
        });
        let (res, _) = inspect_json(&huge_box, 0).unwrap();
        assert_eq!(
            res["pages"],
            json!([{"pageNumber": 1, "rotation": 0, "annotationCount":0}])
        );

        let mut objs = serde_json::Map::new();
        for i in 0..=MAX_JSON_OBJECTS {
            objs.insert(format!("obj:{i} 0 R"), json!({}));
        }
        let bomb = json!({"pages": [], "qpdf": [{"pdfversion": "1.7"}, Value::Object(objs)]});
        assert_eq!(inspect_json(&bomb, 0).unwrap_err().0, "LIMIT_EXCEEDED");
    }
}
