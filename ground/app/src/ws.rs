// Receiver lines go out to whatever browser tabs are open. Nothing upstream comes
// back this way: commands are ordinary POSTs, so a dead tab can only cost us a write.
use crate::json::esc;
use std::sync::{Arc, Mutex};
use tiny_http::ReadWrite;
use tungstenite::{Message, WebSocket};

pub type Socket = WebSocket<Box<dyn ReadWrite + Send>>;

// Each socket keeps the seq it is up to: a page counts them to see what it missed, and a
// tab that joined late starts from 0 with its own snapshot
#[derive(Clone)]
pub struct Broadcast {
    clients: Arc<Mutex<Vec<(Socket, u64)>>>,
}

// type is what the 1.3 page reads and kind what the 1.4 page reads, so one frame does
// for both until the old page is gone
pub fn line_frame(rx: &str, t_ms: u128, text: &str) -> String {
    format!(
        "{{\"type\":\"line\",\"kind\":\"line\",\"rx\":\"{}\",\"t\":{},\"text\":\"{}\"}}",
        esc(rx),
        t_ms,
        esc(text)
    )
}

// {"seq":n, ...the rest of msg}. msg is an object, so it starts with {
fn with_seq(msg: &str, seq: u64) -> String {
    let rest = &msg[1..];
    if rest.starts_with('}') {
        format!("{{\"seq\":{}}}", seq)
    } else {
        format!("{{\"seq\":{},{}", seq, rest)
    }
}

pub fn ports_frame() -> String {
    String::from("{\"type\":\"ports\"}")
}

impl Broadcast {
    pub fn new() -> Self {
        Broadcast { clients: Arc::new(Mutex::new(Vec::new())) }
    }

    // The snapshot goes to this socket alone, numbered from 0, before it hears anything
    // else. The caller holds whatever lock keeps the model still while it does.
    pub fn add_with(&self, mut socket: Socket, first: &[String]) {
        let mut seq = 0;
        for m in first {
            if socket.send(Message::Text(with_seq(m, seq).into())).is_err() {
                return;
            }
            seq += 1;
        }
        if let Ok(mut c) = self.clients.lock() {
            c.push((socket, seq));
        }
    }

    pub fn count(&self) -> usize {
        self.clients.lock().map(|c| c.len()).unwrap_or(0)
    }

    // A tab that has gone away is dropped; the others must not notice. No seq: ports and
    // flash frames are not part of the model and the page does not count them.
    pub fn send(&self, msg: &str) {
        if let Ok(mut c) = self.clients.lock() {
            c.retain_mut(|(ws, _)| ws.send(Message::Text(msg.to_string().into())).is_ok());
        }
    }

    // A model message, numbered per socket
    pub fn send_seq(&self, msg: &str) {
        if let Ok(mut c) = self.clients.lock() {
            c.retain_mut(|(ws, seq)| {
                let ok = ws.send(Message::Text(with_seq(msg, *seq).into())).is_ok();
                *seq += 1;
                ok
            });
        }
    }

    // A replay that is closed lets its tabs go
    pub fn close_all(&self) {
        if let Ok(mut c) = self.clients.lock() {
            for (ws, _) in c.iter_mut() {
                let _ = ws.close(None);
                let _ = ws.flush();
            }
            c.clear();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_frame_is_json_that_survives_quotes_and_tabs() {
        assert_eq!(
            line_frame("rx1", 1790000000000, "PKT,55,00FF,-50.0,13.75,-12"),
            "{\"type\":\"line\",\"kind\":\"line\",\"rx\":\"rx1\",\"t\":1790000000000,\"text\":\"PKT,55,00FF,-50.0,13.75,-12\"}"
        );
        let f = line_frame("rx2", 1, "he said \"hi\"\tthen\\left");
        assert!(f.contains("\\\"hi\\\"\\tthen\\\\left"), "{}", f);
    }

    #[test]
    fn a_dead_client_is_dropped_and_the_others_still_get_lines() {
        let b = Broadcast::new();
        assert_eq!(b.count(), 0);
        b.send("{\"type\":\"ports\"}");   // no clients: must not panic
        b.send_seq("{\"kind\":\"unit\"}");
    }

    #[test]
    fn seq_goes_first_in_the_object() {
        assert_eq!(with_seq("{\"kind\":\"unit\",\"csid\":9}", 4), "{\"seq\":4,\"kind\":\"unit\",\"csid\":9}");
        assert_eq!(with_seq("{}", 0), "{\"seq\":0}");
    }
}
