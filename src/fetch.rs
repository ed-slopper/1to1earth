//! Downloads, the disk cache and a small shared memory cache that loads each key once.
use std::{
    collections::{HashMap, VecDeque},
    path::Path,
    sync::{Arc, Mutex, OnceLock},
    time::Duration,
};

/// At most this many downloads at once, across every generation worker.
const TRANSFERS: usize = 8;
static BUSY: Mutex<usize> = Mutex::new(0);
static FREED: std::sync::Condvar = std::sync::Condvar::new();

/// `url`, or the bytes `range` (first, last inclusive) of it. Ok(None): the server has no such
/// file (404 or 403, as the buckets answer for open sea). Err: it couldn't be read now.
pub fn get(url: &str, range: Option<(u64, u64)>) -> Result<Option<Vec<u8>>, String> {
    {
        let mut busy = BUSY.lock().unwrap();
        while *busy >= TRANSFERS {
            busy = FREED.wait(busy).unwrap();
        }
        *busy += 1;
    }
    let out = get_now(url, range);
    *BUSY.lock().unwrap() -= 1;
    FREED.notify_one();
    out
}

fn get_now(url: &str, range: Option<(u64, u64)>) -> Result<Option<Vec<u8>>, String> {
    // The system's curl (Windows 10 and later, macOS and Linux have it): the game already links
    // its own TLS, which a mod can't link a second time.
    let mut last = String::new();
    for attempt in 0..3 {
        if attempt > 0 {
            std::thread::sleep(Duration::from_millis(500 << attempt));
        }
        let mut cmd = std::process::Command::new("curl");
        cmd.args(["-sS", "-L", "--connect-timeout", "15", "--max-time", "120", "-A", "1to1earth/0.2 (+https://github.com/ed-slopper/1to1earth)"]);
        cmd.args(["-w", "\n%{http_code}"]);
        if let Some((a, b)) = range {
            cmd.args(["-r", &format!("{a}-{b}")]);
        }
        cmd.arg(url);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000); // no console window
        }
        match cmd.output() {
            Ok(out) if out.status.success() => {
                let body = out.stdout;
                let split = body.iter().rposition(|b| *b == b'\n').unwrap_or(0);
                let code: u32 = std::str::from_utf8(&body[split..]).unwrap_or("").trim().parse().unwrap_or(0);
                let data = body[..split].to_vec();
                match code {
                    200 | 206 => return Ok(Some(data)),
                    404 | 403 => return Ok(None),
                    c => last = format!("HTTP {c}"),
                }
            }
            Ok(out) => last = String::from_utf8_lossy(&out.stderr).trim().to_string(),
            Err(e) => last = format!("curl: {e}"),
        }
    }
    Err(format!("{url}: {last}"))
}

pub fn read(path: &Path) -> Option<Vec<u8>> {
    std::fs::read(path).ok()
}

/// Written to a temporary name and moved into place, so a crash never leaves half a file.
pub fn write(path: &Path, bytes: &[u8]) {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let tmp = path.with_extension(format!("tmp{}", std::process::id()));
    if std::fs::write(&tmp, bytes).is_ok() && std::fs::rename(&tmp, path).is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
}

/// A bounded map of loaded values. Each key is loaded once even when many workers ask at the
/// same time; a failed load is forgotten so the next asker tries again.
pub struct Memo<V: Clone> {
    inner: Mutex<Option<(HashMap<u64, Arc<OnceLock<Option<V>>>>, VecDeque<u64>)>>,
    cap: usize,
}

impl<V: Clone> Memo<V> {
    pub const fn new(cap: usize) -> Self {
        Self {
            inner: Mutex::new(None),
            cap,
        }
    }

    /// The value of `key`: `load`'s answer, None when it failed.
    pub fn get(&self, key: u64, load: impl FnOnce() -> Option<V>) -> Option<V> {
        let cell = {
            let mut g = self.inner.lock().unwrap();
            let (map, order) = g.get_or_insert_with(Default::default);
            if let Some(c) = map.get(&key) {
                c.clone()
            } else {
                let c = Arc::new(OnceLock::new());
                map.insert(key, c.clone());
                order.push_back(key);
                while map.len() > self.cap {
                    match order.pop_front() {
                        Some(old) => {
                            map.remove(&old);
                        }
                        None => break,
                    }
                }
                c
            }
        };
        let v = cell.get_or_init(load).clone();
        if v.is_none() {
            let mut g = self.inner.lock().unwrap();
            if let Some((map, _)) = g.as_mut() {
                if map.get(&key).is_some_and(|c| Arc::ptr_eq(c, &cell)) {
                    map.remove(&key);
                }
            }
        }
        v
    }
}
