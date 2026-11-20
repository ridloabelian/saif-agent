//! Pure-Rust WASM dashboard frontend (`wasm32-unknown-unknown`) sharing
//! `saif-rpc-contract` wire types with `saif-web` and `saif-tui`.

use saif_rpc_contract::{
    default_command_catalog, CronJobView, DashboardTab, GatewayEvent, KanbanCardView,
    PromptSubmitParams, RpcRequest, SkillSummaryView, StatusSnapshot,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::{Mutex, OnceLock};

/// Pure-Rust terminal cell grid supporting basic ANSI cursor/clear sequences
/// and generating Hermes-compatible `\x1b[RESIZE:<cols>;<rows>]` frames.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TerminalScreenBuffer {
    pub cols: u16,
    pub rows: u16,
    pub cursor_col: u16,
    pub cursor_row: u16,
    pub cells: Vec<Vec<char>>,
}

impl Default for TerminalScreenBuffer {
    fn default() -> Self {
        Self::new(80, 24)
    }
}

impl TerminalScreenBuffer {
    pub fn new(cols: u16, rows: u16) -> Self {
        let c = cols.max(1);
        let r = rows.max(1);
        Self {
            cols: c,
            rows: r,
            cursor_col: 0,
            cursor_row: 0,
            cells: vec![vec![' '; usize::from(c)]; usize::from(r)],
        }
    }

    pub fn resize(&mut self, cols: u16, rows: u16) -> String {
        let c = cols.max(1);
        let r = rows.max(1);
        let mut next = vec![vec![' '; usize::from(c)]; usize::from(r)];
        for (ri, row) in next.iter_mut().enumerate() {
            if let Some(old_row) = self.cells.get(ri) {
                let copy_len = row.len().min(old_row.len());
                row[..copy_len].copy_from_slice(&old_row[..copy_len]);
            }
        }
        self.cols = c;
        self.rows = r;
        self.cursor_col = self.cursor_col.min(c - 1);
        self.cursor_row = self.cursor_row.min(r - 1);
        self.cells = next;
        format!("\x1b[RESIZE:{c};{r}]")
    }

    pub fn clear(&mut self) {
        for row in &mut self.cells {
            row.fill(' ');
        }
        self.cursor_col = 0;
        self.cursor_row = 0;
    }

    pub fn feed_text(&mut self, raw: &str) {
        let mut chars = raw.chars().peekable();
        while let Some(ch) = chars.next() {
            if ch == '\x1b' && chars.peek() == Some(&'[') {
                chars.next();
                let mut seq = String::new();
                for esc_ch in chars.by_ref() {
                    seq.push(esc_ch);
                    if esc_ch.is_ascii_alphabetic() {
                        break;
                    }
                }
                if seq == "2J" {
                    self.clear();
                } else if seq == "H" {
                    self.cursor_col = 0;
                    self.cursor_row = 0;
                }
                continue;
            }
            match ch {
                '\n' => self.newline(),
                '\r' => self.cursor_col = 0,
                '\x08' => {
                    self.cursor_col = self.cursor_col.saturating_sub(1);
                }
                _ => {
                    let r = usize::from(self.cursor_row);
                    let c = usize::from(self.cursor_col);
                    if r < self.cells.len() && c < self.cells[r].len() {
                        self.cells[r][c] = ch;
                    }
                    self.cursor_col += 1;
                    if self.cursor_col >= self.cols {
                        self.newline();
                    }
                }
            }
        }
    }

    fn newline(&mut self) {
        self.cursor_col = 0;
        if self.cursor_row + 1 < self.rows {
            self.cursor_row += 1;
        } else if !self.cells.is_empty() {
            self.cells.remove(0);
            self.cells.push(vec![' '; usize::from(self.cols)]);
        }
    }

    pub fn render_lines(&self) -> Vec<String> {
        self.cells
            .iter()
            .map(|row| row.iter().collect::<String>().trim_end().to_string())
            .collect()
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DashboardUiState {
    pub connected: bool,
    pub active_tab: DashboardTab,
    pub active_session_id: Option<String>,
    pub profile: String,
    pub skin_name: String,
    pub transcript: Vec<String>,
    pub status: Option<StatusSnapshot>,
    pub terminal: TerminalScreenBuffer,
    pub kanban_cards: Vec<KanbanCardView>,
    pub cron_jobs: Vec<CronJobView>,
    pub skills: Vec<SkillSummaryView>,
}

impl DashboardUiState {
    pub fn apply_event(&mut self, event_json: &str) -> Result<(), String> {
        let ev: GatewayEvent =
            serde_json::from_str(event_json).map_err(|e| format!("invalid event: {e}"))?;
        match ev {
            GatewayEvent::GatewayReady {
                profile, skin_name, ..
            } => {
                self.connected = true;
                self.profile = profile;
                self.skin_name = skin_name;
            }
            GatewayEvent::MessageDelta { delta, .. } => {
                if let Some(last) = self.transcript.last_mut() {
                    last.push_str(&delta);
                } else {
                    self.transcript.push(delta);
                }
            }
            GatewayEvent::MessageComplete { content, .. } => {
                self.transcript.push(content);
            }
            GatewayEvent::ToolStart { name, .. } => {
                self.transcript.push(format!("[tool start: {name}]"));
            }
            GatewayEvent::ToolComplete { name, ok, .. } => {
                self.transcript.push(format!(
                    "[tool {}: {}]",
                    name,
                    if ok { "ok" } else { "err" }
                ));
            }
            GatewayEvent::ThinkingDelta { .. } | GatewayEvent::RequestCancel { .. } => {}
        }
        Ok(())
    }

    pub fn build_prompt_rpc(&self, id: u64, prompt: &str) -> String {
        let req = RpcRequest::new(
            id,
            "prompt.submit",
            serde_json::to_value(PromptSubmitParams {
                session_id: self
                    .active_session_id
                    .clone()
                    .unwrap_or_else(|| "default-web".to_string()),
                prompt: prompt.to_string(),
                steer: false,
            })
            .unwrap_or(serde_json::Value::Null),
        );
        serde_json::to_string(&req).unwrap_or_default()
    }

    pub fn kanban_by_status(&self) -> BTreeMap<String, Vec<KanbanCardView>> {
        let mut grouped: BTreeMap<String, Vec<KanbanCardView>> = BTreeMap::new();
        for card in &self.kanban_cards {
            grouped
                .entry(card.status.clone())
                .or_default()
                .push(card.clone());
        }
        grouped
    }

    pub fn slash_commands_json(&self) -> String {
        serde_json::to_string(&default_command_catalog()).unwrap_or_else(|_| "[]".to_string())
    }

    pub fn render_html(&self) -> String {
        let term = self.terminal.render_lines().join("\n");
        let transcript = self.transcript.join("\n---\n");
        format!(
            "<div data-profile=\"{}\" data-tab=\"{:?}\" data-connected=\"{}\">\
             <pre class=\"transcript\">{}</pre>\
             <pre class=\"terminal\">{}</pre>\
             <div class=\"counts\">kanban={} cron={} skills={}</div>\
             </div>",
            self.profile,
            self.active_tab,
            self.connected,
            transcript,
            term,
            self.kanban_cards.len(),
            self.cron_jobs.len(),
            self.skills.len()
        )
    }
}

#[derive(Default)]
pub struct WasmRuntime {
    pub ui: DashboardUiState,
    pub last_output: Vec<u8>,
}

impl WasmRuntime {
    pub fn feed_pty_str(&mut self, text: &str) -> usize {
        self.ui.terminal.feed_text(text);
        let rendered = self.ui.terminal.render_lines().join("\n");
        self.last_output = rendered.into_bytes();
        self.last_output.len()
    }

    pub fn resize_terminal(&mut self, cols: u16, rows: u16) -> usize {
        let frame = self.ui.terminal.resize(cols, rows);
        self.last_output = frame.into_bytes();
        self.last_output.len()
    }

    pub fn apply_event_json(&mut self, json_str: &str) -> usize {
        if let Err(err) = self.ui.apply_event(json_str) {
            self.last_output = err.into_bytes();
            return 0;
        }
        self.last_output = self.ui.transcript.join("\n").into_bytes();
        self.last_output.len()
    }
}

fn runtime_cell() -> &'static Mutex<WasmRuntime> {
    static RT: OnceLock<Mutex<WasmRuntime>> = OnceLock::new();
    RT.get_or_init(|| Mutex::new(WasmRuntime::default()))
}

#[cfg(target_arch = "wasm32")]
#[no_mangle]
pub extern "C" fn saif_wasm_alloc(len: usize) -> *mut u8 {
    let mut buf = Vec::<u8>::with_capacity(len);
    let ptr = buf.as_mut_ptr();
    std::mem::forget(buf);
    ptr
}

/// # Safety
/// `ptr` must have been allocated by `saif_wasm_alloc` with identical `cap` and not yet freed.
#[cfg(target_arch = "wasm32")]
#[no_mangle]
pub unsafe extern "C" fn saif_wasm_dealloc(ptr: *mut u8, cap: usize) {
    if !ptr.is_null() && cap > 0 {
        drop(unsafe { Vec::from_raw_parts(ptr, 0, cap) });
    }
}

#[cfg(target_arch = "wasm32")]
#[no_mangle]
pub extern "C" fn saif_ui_init() -> u32 {
    if let Ok(mut rt) = runtime_cell().lock() {
        *rt = WasmRuntime::default();
        return 1;
    }
    0
}

#[cfg(target_arch = "wasm32")]
#[no_mangle]
pub extern "C" fn saif_ui_resize_pack(cols: u16, rows: u16) -> u32 {
    (u32::from(cols) << 16) | u32::from(rows)
}

#[cfg(target_arch = "wasm32")]
#[no_mangle]
pub extern "C" fn saif_ui_resize(cols: u16, rows: u16) -> usize {
    let Ok(mut rt) = runtime_cell().lock() else {
        return 0;
    };
    rt.resize_terminal(cols, rows)
}

/// # Safety
/// `ptr` must point to `len` valid readable bytes in WASM linear memory.
#[cfg(target_arch = "wasm32")]
#[no_mangle]
pub unsafe extern "C" fn saif_ui_feed_pty(ptr: *const u8, len: usize) -> usize {
    if ptr.is_null() {
        return 0;
    }
    let slice = unsafe { std::slice::from_raw_parts(ptr, len) };
    let text = String::from_utf8_lossy(slice);
    let Ok(mut rt) = runtime_cell().lock() else {
        return 0;
    };
    rt.feed_pty_str(&text)
}

/// # Safety
/// `ptr` must point to `len` valid readable bytes in WASM linear memory.
#[cfg(target_arch = "wasm32")]
#[no_mangle]
pub unsafe extern "C" fn saif_ui_apply_event(ptr: *const u8, len: usize) -> usize {
    if ptr.is_null() {
        return 0;
    }
    let slice = unsafe { std::slice::from_raw_parts(ptr, len) };
    let text = String::from_utf8_lossy(slice);
    let Ok(mut rt) = runtime_cell().lock() else {
        return 0;
    };
    rt.apply_event_json(&text)
}

#[cfg(target_arch = "wasm32")]
#[no_mangle]
pub extern "C" fn saif_ui_output_ptr() -> *const u8 {
    let Ok(rt) = runtime_cell().lock() else {
        return std::ptr::null();
    };
    rt.last_output.as_ptr()
}

#[cfg(target_arch = "wasm32")]
#[no_mangle]
pub extern "C" fn saif_ui_output_len() -> usize {
    let Ok(rt) = runtime_cell().lock() else {
        return 0;
    };
    rt.last_output.len()
}

/// Touch runtime cell on non-wasm targets so host tests/clippy keep zero dead-code warnings.
pub fn init_host_runtime() -> usize {
    runtime_cell()
        .lock()
        .map(|rt| rt.ui.terminal.cols as usize)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wasm_ui_terminal_buffer_and_tabs_work() {
        let mut buf = TerminalScreenBuffer::new(20, 4);
        buf.feed_text("hello\r\nworld");
        assert_eq!(buf.render_lines()[0], "hello");
        assert_eq!(buf.render_lines()[1], "world");
        assert_eq!(buf.resize(120, 40), "\x1b[RESIZE:120;40]");
        buf.feed_text("\x1b[2Jfresh");
        assert_eq!(buf.render_lines()[0], "fresh");

        let mut rt = WasmRuntime::default();
        let len = rt.feed_pty_str("prompt> ls\r\nCargo.toml");
        assert!(len > 0);
        let out_str = String::from_utf8(rt.last_output.clone()).unwrap();
        assert!(out_str.contains("prompt> ls"));
        assert!(out_str.contains("Cargo.toml"));

        let rlen = rt.resize_terminal(100, 30);
        assert_eq!(&rt.last_output[..rlen], b"\x1b[RESIZE:100;30]");

        let ev_len =
            rt.apply_event_json(r#"{"type":"message_delta","session_id":"s1","delta":"hi wasm"}"#);
        assert!(ev_len > 0);
        assert!(String::from_utf8_lossy(&rt.last_output).contains("hi wasm"));
        assert!(init_host_runtime() >= 80);

        let mut ui = DashboardUiState {
            active_tab: DashboardTab::Kanban,
            profile: "default".into(),
            ..DashboardUiState::default()
        };
        ui.kanban_cards.push(KanbanCardView {
            id: "1".into(),
            title: "WASM grid".into(),
            status: "ready".into(),
            assignee: None,
            priority: 0,
        });
        assert_eq!(ui.kanban_by_status()["ready"].len(), 1);
        assert!(ui.render_html().contains("kanban=1"));
    }
}
