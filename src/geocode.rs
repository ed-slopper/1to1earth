//! `/earth tp <address>`: places looked up with OpenStreetMap's Nominatim (data © OpenStreetMap
//! contributors, ODbL), as Tellus's search does. One lookup at a time and at most one a second,
//! as Nominatim's usage policy asks, on a thread of its own so the server never waits; the answer
//! is picked up by the server tick.
use crate::fetch;
use mod_api::serde_json::{self, Value};
use std::{
    sync::Mutex,
    time::{Duration, Instant},
};

/// A finished lookup: who asked, what for, and where it is (or why not).
pub struct Answer {
    pub player: u128,
    pub query: String,
    pub place: Result<(f64, f64, String), String>,
}

static ANSWERS: Mutex<Vec<Answer>> = Mutex::new(Vec::new());
static LAST: Mutex<Option<Instant>> = Mutex::new(None);

/// Look `query` up for `player`; the answer arrives in [`answers`].
pub fn start(player: u128, query: String) {
    std::thread::spawn(move || {
        let place = lookup(&query);
        ANSWERS.lock().unwrap().push(Answer { player, query, place });
    });
}

/// The lookups finished since the last call.
pub fn answers() -> Vec<Answer> {
    std::mem::take(&mut *ANSWERS.lock().unwrap())
}

fn lookup(query: &str) -> Result<(f64, f64, String), String> {
    {
        // One request a second, all players together.
        let mut last = LAST.lock().unwrap();
        if let Some(t) = *last {
            let wait = Duration::from_secs(1).saturating_sub(t.elapsed());
            std::thread::sleep(wait);
        }
        *last = Some(Instant::now());
    }
    let url = format!(
        "https://nominatim.openstreetmap.org/search?q={}&format=jsonv2&limit=1",
        encode(query)
    );
    let body = fetch::get(&url, None)?.ok_or_else(|| "the place search isn't answering".to_string())?;
    let v: Value = serde_json::from_slice(&body).map_err(|e| format!("the place search answered oddly ({e})"))?;
    let first = v.as_array().and_then(|a| a.first()).ok_or_else(|| "no such place found".to_string())?;
    let num = |k: &str| first[k].as_str().and_then(|s| s.parse::<f64>().ok());
    match (num("lat"), num("lon")) {
        (Some(lat), Some(lon)) => {
            let name = first["display_name"].as_str().unwrap_or(query).to_string();
            Ok((lat, lon, name))
        }
        _ => Err("no such place found".into()),
    }
}

/// Percent-encoding for a URL's query.
fn encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.trim().bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            b' ' => out.push_str("%20"),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}
