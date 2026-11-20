use crate::error::{Error, Result};
use sha2::{Digest, Sha256};
use std::{
    ffi::CString,
    fs::File,
    io::{Read, Seek, SeekFrom, Write},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::fs::{MetadataExt, OpenOptionsExt},
    },
    path::{Component, Path},
};

pub const FILE_LIMIT: u64 = 100 * 1024 * 1024;
pub const TOTAL_LIMIT: u64 = 250 * 1024 * 1024;
pub struct Workspace {
    dir: File,
}
pub struct Input {
    pub file: File,
    pub hash: String,
    pub bytes: u64,
    pub path: String,
    dev: u64,
    ino: u64,
}
pub struct Destination {
    dir: File,
    name: CString,
    replace: bool,
}
fn c(s: &str) -> Result<CString> {
    CString::new(s)
        .map_err(|_| Error("ACCESS_DENIED", None).with(format!("path '{s}' contains a NUL byte")))
}
fn open_at(dir: &File, name: &CString, flags: i32, mode: u32) -> Result<File> {
    let fd = unsafe {
        libc::openat(
            dir.as_raw_fd(),
            name.as_ptr(),
            flags | libc::O_CLOEXEC | libc::O_NOFOLLOW,
            mode,
        )
    };
    if fd < 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(unsafe { File::from_raw_fd(fd) })
}
fn trusted_dir(f: &File) -> Result<()> {
    let m = f.metadata()?;
    if !m.is_dir()
        || m.mode() & 0o022 != 0
        || (m.uid() != unsafe { libc::geteuid() } && m.uid() != 0)
    {
        return Err(Error("ACCESS_DENIED", None));
    }
    Ok(())
}
fn parts(path: &str) -> Result<Vec<&str>> {
    if path.contains(['\0', '\\', ':'])
        || path.is_empty()
        || path.starts_with('/')
        || path
            .split('/')
            .any(|x| x.is_empty() || x == "." || x == "..")
    {
        return Err(Error("ACCESS_DENIED", None).with(format!(
            "path '{path}' must be workspace-relative with no '..', '.', ':' or '\\\\' components"
        )));
    }
    if Path::new(path)
        .components()
        .any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err(Error("ACCESS_DENIED", None).with(format!(
            "path '{path}' must be workspace-relative with no '..', '.', ':' or '\\\\' components"
        )));
    }
    Ok(path.split('/').collect())
}
pub fn hash(file: &mut File) -> Result<String> {
    file.seek(SeekFrom::Start(0))?;
    let mut h = Sha256::new();
    let mut buf = [0; 65536];
    let mut total = 0u64;
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        total += n as u64;
        if total > TOTAL_LIMIT {
            return Err(Error("LIMIT_EXCEEDED", None));
        }
        h.update(&buf[..n]);
    }
    file.seek(SeekFrom::Start(0))?;
    Ok(format!("{:x}", h.finalize()))
}
impl Workspace {
    pub fn new(root: &Path) -> Result<Self> {
        let dir = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(root)?;
        trusted_dir(&dir)?;
        Ok(Self { dir })
    }
    fn parent(&self, path: &str) -> Result<(File, CString)> {
        let p = parts(path)?;
        let mut dir = self.dir.try_clone()?;
        for name in &p[..p.len() - 1] {
            dir = open_at(&dir, &c(name)?, libc::O_RDONLY | libc::O_DIRECTORY, 0)?;
            trusted_dir(&dir)?;
        }
        Ok((dir, c(p[p.len() - 1])?))
    }
    /// Attach `input '<path>':` context to errors; when the inner error
    /// already carries a detail it is appended so the file is always named.
    fn input_ctx(path: &str, e: Error) -> Error {
        match e.1 {
            Some(d) => Error(e.0, Some(format!("input '{path}': {d}"))),
            None if e.0 == "NOT_FOUND" => e.with(format!("input '{path}' does not exist")),
            None => e.with(format!("input '{path}'")),
        }
    }
    pub fn input(&self, path: &str) -> Result<Input> {
        let (dir, name) = self.parent(path).map_err(|e| Self::input_ctx(path, e))?;
        // NONBLOCK prevents blocking on FIFOs before regular-file validation.
        let mut file = open_at(&dir, &name, libc::O_RDONLY | libc::O_NONBLOCK, 0)
            .map_err(|e| Self::input_ctx(path, e))?;
        let m = file
            .metadata()
            .map_err(|e| Self::input_ctx(path, e.into()))?;
        if !m.is_file() {
            return Err(
                Error("ACCESS_DENIED", None).with(format!("input '{path}' is not a regular file"))
            );
        }
        if m.len() > FILE_LIMIT {
            return Err(Error(
                "LIMIT_EXCEEDED",
                Some(format!(
                    "input '{path}' exceeds the {} MiB file limit",
                    FILE_LIMIT / 1024 / 1024
                )),
            ));
        }
        let digest = hash(&mut file).map_err(|e| Self::input_ctx(path, e))?;
        if file.metadata()?.len() != m.len() {
            return Err(Error("INPUT_CHANGED", None)
                .with(format!("input '{path}' changed during processing")));
        }
        Ok(Input {
            file,
            hash: digest,
            bytes: m.len(),
            path: path.into(),
            dev: m.dev(),
            ino: m.ino(),
        })
    }
    pub fn destination(&self, path: &str, replace: bool, inputs: &[Input]) -> Result<Destination> {
        let ctx = |e: Error| match e.1 {
            Some(d) => Error(e.0, Some(format!("output '{path}': {d}"))),
            None if e.0 == "NOT_FOUND" => e.with(format!("output '{path}' does not exist")),
            None => e.with(format!("output '{path}'")),
        };
        let (dir, name) = self.parent(path).map_err(ctx)?;
        let d = Destination { dir, name, replace };
        d.check(inputs).map_err(ctx)?;
        Ok(d)
    }
    pub fn unchanged(&self, inputs: &mut [Input]) -> Result<()> {
        for i in inputs {
            let current = self
                .input(&i.path)
                .map_err(|_| Error("INPUT_CHANGED", None))?;
            if current.dev != i.dev
                || current.ino != i.ino
                || current.hash != i.hash
                || hash(&mut i.file)? != i.hash
            {
                return Err(Error("INPUT_CHANGED", None));
            }
        }
        Ok(())
    }
}
impl Input {
    pub fn stage(&mut self, path: &Path) -> Result<()> {
        self.file.seek(SeekFrom::Start(0))?;
        let mut out = std::fs::OpenOptions::new()
            .write(true)
            .read(true)
            .create_new(true)
            .mode(0o600)
            .open(path)?;
        let n = std::io::copy(&mut (&mut self.file).take(FILE_LIMIT + 1), &mut out)?;
        if n != self.bytes || hash(&mut self.file)? != self.hash || hash(&mut out)? != self.hash {
            return Err(Error("INPUT_CHANGED", None));
        }
        out.sync_all()?;
        Ok(())
    }
}
impl Destination {
    pub fn same_target(&self, other: &Self) -> Result<bool> {
        let a_dir = self.dir.metadata()?;
        let b_dir = other.dir.metadata()?;
        if a_dir.dev() == b_dir.dev() && a_dir.ino() == b_dir.ino() {
            let a_name = self.name.to_string_lossy();
            let b_name = other.name.to_string_lossy();
            #[cfg(target_os = "macos")]
            let same_name = a_name.eq_ignore_ascii_case(&b_name);
            #[cfg(not(target_os = "macos"))]
            let same_name = a_name == b_name;
            if same_name {
                return Ok(true);
            }
        }
        if let (Ok(a_file), Ok(b_file)) = (
            open_at(&self.dir, &self.name, libc::O_RDONLY | libc::O_NONBLOCK, 0),
            open_at(
                &other.dir,
                &other.name,
                libc::O_RDONLY | libc::O_NONBLOCK,
                0,
            ),
        ) {
            let (am, bm) = (a_file.metadata()?, b_file.metadata()?);
            if am.dev() == bm.dev() && am.ino() == bm.ino() {
                return Ok(true);
            }
        }
        Ok(false)
    }
    pub fn rollback_new(&self) {
        if !self.replace {
            let _ = unsafe { libc::unlinkat(self.dir.as_raw_fd(), self.name.as_ptr(), 0) };
        }
    }
    fn check(&self, inputs: &[Input]) -> Result<()> {
        match open_at(&self.dir, &self.name, libc::O_RDONLY | libc::O_NONBLOCK, 0) {
            Ok(f) => {
                let m = f.metadata()?;
                if !m.is_file() || inputs.iter().any(|i| i.dev == m.dev() && i.ino == m.ino()) {
                    return Err(Error("ACCESS_DENIED", None));
                }
                if !self.replace {
                    return Err(Error(
                        "OUTPUT_EXISTS",
                        Some("pass --replace or set replaceExisting to overwrite".into()),
                    ));
                }
                Ok(())
            }
            Err(Error("NOT_FOUND", None)) => Ok(()),
            Err(_) => Err(Error("ACCESS_DENIED", None)),
        }
    }
    pub fn commit(&self, source: &Path, inputs: &[Input]) -> Result<()> {
        self.commit_file(&mut File::open(source)?, inputs, TOTAL_LIMIT)
    }
    pub(crate) fn commit_file(
        &self,
        source: &mut File,
        inputs: &[Input],
        limit: u64,
    ) -> Result<()> {
        self.check(inputs)?;
        let name = c(&format!(
            ".anpdf-{}",
            tempfile::Builder::new()
                .rand_bytes(16)
                .prefix("")
                .tempfile()?
                .path()
                .file_name()
                .unwrap()
                .to_string_lossy()
        ))?;
        let mut temp = open_at(
            &self.dir,
            &name,
            libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL,
            0o600,
        )?;
        let result = (|| {
            let n = std::io::copy(&mut source.take(limit + 1), &mut temp)?;
            if n > limit {
                return Err(Error("LIMIT_EXCEEDED", None));
            }
            temp.flush()?;
            temp.sync_all()?;
            self.check(inputs)?;
            let rc = unsafe {
                if self.replace {
                    libc::renameat(
                        self.dir.as_raw_fd(),
                        name.as_ptr(),
                        self.dir.as_raw_fd(),
                        self.name.as_ptr(),
                    )
                } else {
                    libc::linkat(
                        self.dir.as_raw_fd(),
                        name.as_ptr(),
                        self.dir.as_raw_fd(),
                        self.name.as_ptr(),
                        0,
                    )
                }
            };
            if rc != 0 {
                return Err(std::io::Error::last_os_error().into());
            }
            self.dir.sync_all()?;
            Ok(())
        })();
        unsafe { libc::unlinkat(self.dir.as_raw_fd(), name.as_ptr(), 0) };
        result
    }
}
