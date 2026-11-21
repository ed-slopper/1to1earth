//! Mapbox Vector Tiles: just enough protobuf to read a tile's layers, their features' tags and
//! geometry (points, lines, polygons in tile units, `extent` across).
use std::collections::HashMap;

#[derive(Clone, Debug)]
pub enum Value {
    Str(String),
    Num(f64),
    Bool(bool),
}

impl Value {
    pub fn num(&self) -> Option<f64> {
        match self {
            Value::Num(v) => Some(*v),
            Value::Str(s) => s.trim().parse().ok(),
            Value::Bool(_) => None,
        }
    }
    pub fn str(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }
    pub fn truthy(&self) -> bool {
        match self {
            Value::Bool(b) => *b,
            Value::Num(v) => *v != 0.0,
            Value::Str(s) => !matches!(s.as_str(), "" | "no" | "false" | "0"),
        }
    }
}

pub struct Feature {
    /// 1 point, 2 line, 3 polygon.
    pub kind: u32,
    pub tags: HashMap<String, Value>,
    /// The geometry's parts (rings or lines) in tile units.
    pub parts: Vec<Vec<[i32; 2]>>,
}

pub struct Layer {
    pub name: String,
    pub extent: u32,
    pub features: Vec<Feature>,
}

struct Reader<'a> {
    b: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn varint(&mut self) -> Option<u64> {
        let mut v = 0u64;
        for shift in (0..64).step_by(7) {
            let byte = *self.b.get(self.at)?;
            self.at += 1;
            v |= ((byte & 0x7f) as u64) << shift;
            if byte < 0x80 {
                return Some(v);
            }
        }
        None
    }
    fn done(&self) -> bool {
        self.at >= self.b.len()
    }
    /// The next field: its number, wire type and (for length-delimited fields) its bytes.
    fn field(&mut self) -> Option<(u64, u64, &'a [u8], u64)> {
        let key = self.varint()?;
        let (num, wire) = (key >> 3, key & 7);
        match wire {
            0 => {
                let v = self.varint()?;
                Some((num, wire, &[], v))
            }
            1 => {
                let s = self.b.get(self.at..self.at + 8)?;
                self.at += 8;
                Some((num, wire, s, 0))
            }
            2 => {
                let len = self.varint()? as usize;
                let s = self.b.get(self.at..self.at + len)?;
                self.at += len;
                Some((num, wire, s, 0))
            }
            5 => {
                let s = self.b.get(self.at..self.at + 4)?;
                self.at += 4;
                Some((num, wire, s, 0))
            }
            _ => None,
        }
    }
}

fn packed(b: &[u8]) -> Vec<u32> {
    let mut r = Reader { b, at: 0 };
    let mut out = Vec::new();
    while !r.done() {
        match r.varint() {
            Some(v) => out.push(v as u32),
            None => break,
        }
    }
    out
}

fn value(b: &[u8]) -> Option<Value> {
    let mut r = Reader { b, at: 0 };
    let mut out = None;
    while !r.done() {
        let (num, _, bytes, v) = r.field()?;
        out = Some(match num {
            1 => Value::Str(String::from_utf8_lossy(bytes).into_owned()),
            2 => Value::Num(f32::from_le_bytes(bytes.try_into().ok()?) as f64),
            3 => Value::Num(f64::from_le_bytes(bytes.try_into().ok()?)),
            4 => Value::Num(v as i64 as f64),
            5 => Value::Num(v as f64),
            6 => Value::Num(((v >> 1) as i64 ^ -((v & 1) as i64)) as f64),
            7 => Value::Bool(v != 0),
            _ => continue,
        });
    }
    out
}

fn geometry(cmds: &[u32]) -> Vec<Vec<[i32; 2]>> {
    let zig = |v: u32| ((v >> 1) as i32) ^ -((v & 1) as i32);
    let (mut x, mut y) = (0i32, 0i32);
    let mut parts: Vec<Vec<[i32; 2]>> = Vec::new();
    let mut i = 0;
    while i < cmds.len() {
        let (id, count) = (cmds[i] & 7, cmds[i] >> 3);
        i += 1;
        match id {
            1 | 2 => {
                for _ in 0..count {
                    if i + 1 >= cmds.len() {
                        return parts;
                    }
                    x += zig(cmds[i]);
                    y += zig(cmds[i + 1]);
                    i += 2;
                    if id == 1 {
                        parts.push(Vec::new());
                    }
                    if let Some(p) = parts.last_mut() {
                        p.push([x, y]);
                    }
                }
            }
            7 => {
                if let Some(p) = parts.last_mut() {
                    if let Some(&first) = p.first() {
                        p.push(first);
                    }
                }
            }
            _ => return parts,
        }
    }
    parts
}

/// The layers of a tile named in `wanted`.
pub fn decode(tile: &[u8], wanted: &[&str]) -> Vec<Layer> {
    let mut out = Vec::new();
    let mut r = Reader { b: tile, at: 0 };
    while !r.done() {
        let Some((num, wire, bytes, _)) = r.field() else { break };
        if num != 3 || wire != 2 {
            continue;
        }
        if let Some(l) = layer(bytes, wanted) {
            out.push(l);
        }
    }
    out
}

fn layer(b: &[u8], wanted: &[&str]) -> Option<Layer> {
    let mut r = Reader { b, at: 0 };
    let (mut name, mut extent) = (String::new(), 4096u32);
    let (mut keys, mut values, mut raw) = (Vec::new(), Vec::new(), Vec::new());
    while !r.done() {
        let (num, _, bytes, v) = r.field()?;
        match num {
            1 => {
                name = String::from_utf8_lossy(bytes).into_owned();
                if !wanted.contains(&name.as_str()) {
                    return None;
                }
            }
            2 => raw.push(bytes),
            3 => keys.push(String::from_utf8_lossy(bytes).into_owned()),
            4 => values.push(value(bytes).unwrap_or(Value::Str(String::new()))),
            5 => extent = v as u32,
            _ => {}
        }
    }
    let mut features = Vec::with_capacity(raw.len());
    for f in raw {
        let mut r = Reader { b: f, at: 0 };
        let (mut kind, mut tags_raw, mut geom) = (0u32, Vec::new(), Vec::new());
        while !r.done() {
            let Some((num, _, bytes, v)) = r.field() else { break };
            match num {
                2 => tags_raw = packed(bytes),
                3 => kind = v as u32,
                4 => geom = packed(bytes),
                _ => {}
            }
        }
        let mut tags = HashMap::new();
        for kv in tags_raw.chunks_exact(2) {
            if let (Some(k), Some(v)) = (keys.get(kv[0] as usize), values.get(kv[1] as usize)) {
                tags.insert(k.clone(), v.clone());
            }
        }
        features.push(Feature { kind, tags, parts: geometry(&geom) });
    }
    Some(Layer { name, extent, features })
}
