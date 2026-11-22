//! Generic phone-pushed fields. Reading the mailbox is local; only subscribing
//! or reconnecting sends a BLE request. A complete field batch applies atomically.
// every app that compiles this module uses a subset of it, so per-app dead code is expected.
#![allow(dead_code)]
#[link(wasm_import_module = "mira")]
extern "C" {
    fn ble_request(method: i32, payload: *const u8, length: i32) -> i32;
    fn ble_response_len(id: i32) -> i32;
    fn ble_response_read(id: i32, out: *mut u8, capacity: i32) -> i32;
    fn ble_stream_read(token: i32, out: *mut u8, capacity: i32) -> i32;
}

/// Packets and bytes read from the phone, for Casa's frame report.
pub static mut PACKETS: u32 = 0;
pub static mut PACKET_BYTES: u32 = 0;

/// Milliseconds between subscription attempts.
const RETRY_MS: u32 = 1000;

/// Small independent updates (dashboard fields) need no duplicate state buffers.
/// Each batch must fit one transport packet; the renderer owns the stored values.
/// `cooldown` counts down to the next subscription attempt; zero (the initial
/// state) subscribes now, and keeps these all-zero statics out of the module's data.
pub struct Events { token: i32, pending: bool, cooldown: u32, revision: u32 }
impl Events {
    pub const fn new() -> Self { Self { token: 0, pending: false, cooldown: 0, revision: 0 } }
    pub fn reset(&mut self) { *self = Self::new(); }
    pub unsafe fn next(&mut self, topic: u16, mode: u8, elapsed: u32, out: &mut [u8;384]) -> i32 {
        self.cooldown = self.cooldown.saturating_sub(elapsed);
        if self.token <= 0 {
            if self.cooldown > 0 { return -1; }
            let request = [(topic & 255) as u8, (topic >> 8) as u8, mode];
            self.token = ble_request(0x70, request.as_ptr(), 3);
            self.pending = self.token > 0; self.cooldown = RETRY_MS; self.revision = 0;
            return -1;
        }
        if self.pending {
            let n = ble_response_len(self.token);
            if n == -1 { return -1; }
            if n < 0 || ble_response_read(self.token, out.as_mut_ptr(), 384) < 0 { self.token = 0; return -1; }
            self.pending = false;
        }
        let n = ble_stream_read(self.token, out.as_mut_ptr(), 384);
        if n > 0 { PACKETS += 1; PACKET_BYTES += n as u32; }
        if n == -2 { self.reset(); return -1; }
        if n < 8 || n > 384 { return -1; }
        let revision = u32::from_le_bytes(out[..4].try_into().unwrap());
        if self.revision != 0 && revision.wrapping_sub(self.revision) as i32 <= 0 { return 0; }
        if out[4] != 0 || out[5] != 0 || u16::from_le_bytes([out[6],out[7]]) as i32 != n - 8 { self.reset(); return -1; }
        self.revision = revision;
        n
    }
}

pub struct Fields<const N: usize> {
    data: [u8; N],
    lengths: [usize; 32],
    // Where each field starts, so reading one is not a sum of those before it.
    starts: [usize; 32],
    // Counts applied changes, so a reader can keep what it worked out from the same fields;
    // each field also notes the change that last touched it.
    version: u32,
    versions: [u32; 32],
    staging: [u8; N],
    offset: usize,
    total: usize,
    loading_revision: u32,
    revision: u32,
    token: i32,
    pending: bool,
    cooldown: u32,
    pub ready: bool,
}
impl<const N: usize> Fields<N> {
    pub const fn new() -> Self {
        Self { data: [0; N], lengths: [0; 32], starts: [0; 32], version: 0, versions: [0; 32], staging: [0; N], offset: 0, total: 0,
            loading_revision: 0, revision: 0, token: 0, pending: false,
            cooldown: 0, ready: false }
    }
    pub fn field(&self, index: usize) -> &[u8] {
        if index >= 32 { return &[]; }
        let start = self.starts[index];
        &self.data[start..start + self.lengths[index]]
    }
    pub fn version(&self) -> u32 { self.version }
    /// `part` lies in these fields, so it stays the same until the version changes.
    pub fn holds(&self, part: &[u8]) -> bool {
        let (start, at) = (self.data.as_ptr() as usize, part.as_ptr() as usize);
        at >= start && at + part.len() <= start + N
    }
    /// The last change to any field from `first` to `last`.
    pub fn fields_version(&self, first: usize, last: usize) -> u32 {
        self.versions[first.min(31)..=last.min(31)].iter().copied().max().unwrap_or(0)
    }
    pub fn refresh(&mut self) {
        self.token = 0; self.pending = false; self.offset = 0; self.revision = 0;
        self.loading_revision = 0; self.cooldown = 0;
    }
    pub fn reset(&mut self) {
        self.refresh(); self.ready = false; self.lengths = [0; 32]; self.starts = [0; 32];
        self.version = self.version.wrapping_add(1);
        self.versions = [self.version; 32];
    }
    #[inline(never)]
    fn apply(&mut self) -> Result<(), ()> {
        let mut positions = [usize::MAX; 32];
        let mut sizes = self.lengths;
        let mut cursor = 0;
        while cursor < self.total {
            if self.total - cursor < 3 { return Err(()); }
            let key = self.staging[cursor] as usize;
            let length = u16::from_le_bytes([self.staging[cursor+1], self.staging[cursor+2]]) as usize;
            cursor += 3;
            if key >= 32 || positions[key] != usize::MAX || length > self.total - cursor { return Err(()); }
            positions[key] = cursor; sizes[key] = length; cursor += length;
        }
        if sizes.iter().sum::<usize>() > N { return Err(()); }
        // Merge in place: a merge buffer the size of the feed overflowed Casa's
        // 8 KiB stack. Kept fields sliding toward the start move in key order,
        // then those sliding toward the end in reverse, so none overwrites another.
        let (mut old_at, mut new_at, mut old, mut new) = ([0usize; 32], [0usize; 32], 0, 0);
        for key in 0..32 { old_at[key] = old; new_at[key] = new; old += self.lengths[key]; new += sizes[key]; }
        let kept = |key: usize| positions[key] == usize::MAX;
        for key in (0..32).filter(|&k| kept(k) && new_at[k] < old_at[k]).chain((0..32).rev().filter(|&k| kept(k) && new_at[k] > old_at[k])) {
            self.data.copy_within(old_at[key]..old_at[key]+sizes[key], new_at[key]);
        }
        for key in (0..32).filter(|&k| !kept(k)) {
            self.data[new_at[key]..new_at[key]+sizes[key]].copy_from_slice(&self.staging[positions[key]..positions[key]+sizes[key]]);
        }
        self.lengths = sizes; self.ready = true;
        self.starts = new_at;
        self.version = self.version.wrapping_add(1);
        for key in (0..32).filter(|&k| !kept(k)) { self.versions[key] = self.version; }
        Ok(())
    }
    fn accept(&mut self, packet: &[u8]) -> Result<bool, ()> {
        if packet.len() <= 8 { return Err(()); }
        let revision = u32::from_le_bytes(packet[..4].try_into().unwrap());
        let offset = u16::from_le_bytes(packet[4..6].try_into().unwrap()) as usize;
        let total = u16::from_le_bytes(packet[6..8].try_into().unwrap()) as usize;
        let bytes = &packet[8..];
        if revision == 0 || total > N || offset >= total || bytes.len() > total-offset { return Err(()); }
        if self.revision != 0 && revision.wrapping_sub(self.revision) as i32 <= 0 { return Ok(false); }
        if revision == self.loading_revision && offset < self.offset {
            // An acknowledgement can be lost after delivery; retries are inert.
            if total == self.total && offset+bytes.len() <= self.offset
                && &self.staging[offset..offset+bytes.len()] == bytes { return Ok(false); }
            return Err(());
        }
        if offset == 0 { self.offset = 0; self.total = total; self.loading_revision = revision; }
        if revision != self.loading_revision || total != self.total || offset != self.offset { return Err(()); }
        self.staging[offset..offset+bytes.len()].copy_from_slice(bytes);
        self.offset += bytes.len();
        if self.offset != total { return Ok(false); }
        self.apply()?;
        self.revision = revision; self.offset = 0;
        Ok(true)
    }
    pub unsafe fn tick(&mut self, topic: u16, mode: u8, elapsed: u32) -> bool {
        self.cooldown = self.cooldown.saturating_sub(elapsed);
        if self.token <= 0 {
            if self.cooldown > 0 { return false; }
            let request = [(topic & 255) as u8, (topic >> 8) as u8, mode];
            self.token = ble_request(0x0070, request.as_ptr(), 3);
            self.pending = self.token > 0; self.cooldown = RETRY_MS;
            return false;
        }
        if self.pending {
            let length = ble_response_len(self.token);
            if length >= 0 {
                let mut reply = [0u8; 384];
                if ble_response_read(self.token, reply.as_mut_ptr(), 384) < 0 { self.token = 0; return false; }
                self.pending = false;
            } else if length != -1 { self.token = 0; self.pending = false; return false; }
            else { return false; }
        }
        let mut changed = false;
        for _ in 0..4 {
            let mut packet = [0u8; 384];
            let length = ble_stream_read(self.token, packet.as_mut_ptr(), 384);
            if length > 0 { PACKETS += 1; PACKET_BYTES += length as u32; }
            if length == -2 { self.refresh(); break; }
            if length < 0 { break; }
            if length as usize > packet.len() { self.refresh(); break; }
            match self.accept(&packet[..length as usize]) {
                Ok(value) => changed |= value,
                Err(()) => { self.refresh(); break; }
            }
        }
        changed
    }
}
