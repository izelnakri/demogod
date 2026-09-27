//! The Chrome DevTools Protocol, over the one kind of WebSocket it needs: unencrypted, on
//! localhost, text frames of JSON. A few dozen lines instead of a WebSocket crate and its TLS.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};

use crate::{Error, Result};

/// How long any one command may take before the browser is taken to have hung.
const COMMAND_TIMEOUT: Duration = Duration::from_secs(60);

/// A message the browser sent without being asked: `method`, `params`, and the session it is for.
pub(crate) type Handler = Box<dyn FnMut(&Connection, &str, &Value, Option<&str>) + Send>;

/// A command's answer: its result, or the browser's error message.
type Answer = std::result::Result<Value, String>;

/// What the reader thread and the callers share.
struct Shared {
    writer: Mutex<TcpStream>,
    next_id: AtomicU64,
    /// The calls waiting for an answer — or `None` once the browser has gone, so no call waits
    /// for an answer that cannot come.
    pending: Mutex<Option<HashMap<u64, Sender<Answer>>>>,
}

/// A DevTools connection. Cloning it is cheap, and every clone talks to the same browser.
#[derive(Clone)]
pub(crate) struct Connection(Arc<Shared>);

impl Connection {
    /// Opens the WebSocket at `ws://127.0.0.1:<port><path>` and starts reading from it, handing
    /// every event to `handler`.
    pub fn open(port: u16, path: &str, mut handler: Handler) -> Result<Connection> {
        let mut stream = TcpStream::connect(("127.0.0.1", port))
            .map_err(|error| Error::new(format!("could not reach the browser on port {port}: {error}")))?;
        stream.set_nodelay(true).ok();
        write!(
            stream,
            "GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\
             Sec-WebSocket-Key: ZGVtb2dvZC1kZW1vZ29kIQ==\r\nSec-WebSocket-Version: 13\r\n\r\n"
        )?;
        let mut reader = BufReader::new(stream.try_clone()?);
        let mut status = String::new();
        reader.read_line(&mut status)?;
        if !status.contains(" 101 ") {
            return Err(Error::new(format!("the browser refused the connection: {}", status.trim())));
        }
        loop {
            let mut header = String::new();
            if reader.read_line(&mut header)? == 0 || header == "\r\n" {
                break;
            }
        }

        let connection = Connection(Arc::new(Shared {
            writer: Mutex::new(stream),
            next_id: AtomicU64::new(1),
            pending: Mutex::new(Some(HashMap::new())),
        }));
        let reading = connection.clone();
        std::thread::spawn(move || {
            while let Ok(message) = read_message(&mut reader, &reading) {
                let Ok(value) = serde_json::from_slice::<Value>(&message) else { continue };
                match value.get("id").and_then(Value::as_u64) {
                    Some(id) => {
                        let result = match value.get("error") {
                            Some(error) => Err(error["message"].as_str().unwrap_or("unknown error").to_string()),
                            None => Ok(value.get("result").cloned().unwrap_or(Value::Null)),
                        };
                        let waiting = reading
                            .0
                            .pending
                            .lock()
                            .expect("never poisoned")
                            .as_mut()
                            .and_then(|pending| pending.remove(&id));
                        if let Some(waiting) = waiting {
                            let _ = waiting.send(result);
                        }
                    }
                    None => {
                        let method = value["method"].as_str().unwrap_or("");
                        handler(&reading, method, &value["params"], value["sessionId"].as_str());
                    }
                }
            }
            // The browser is gone: whoever is still waiting hears so rather than timing out.
            *reading.0.pending.lock().expect("never poisoned") = None;
        });

        Ok(connection)
    }

    /// Sends a command and waits for its result.
    pub fn call(&self, method: &str, params: Value, session: Option<&str>) -> Result<Value> {
        let (id, receiver) = self.send(method, params, session)?;
        let result = receiver.recv_timeout(COMMAND_TIMEOUT).map_err(|_| {
            self.forget(id);
            Error::new(format!("the browser did not answer {method}"))
        })?;

        result.map_err(|message| Error::new(format!("{method}: {message}")))
    }

    /// Sends a command without waiting for its result.
    pub fn notify(&self, method: &str, params: Value, session: Option<&str>) {
        let _ = self.send(method, params, session);
    }

    fn send(
        &self,
        method: &str,
        params: Value,
        session: Option<&str>,
    ) -> Result<(u64, std::sync::mpsc::Receiver<Answer>)> {
        let id = self.0.next_id.fetch_add(1, Ordering::Relaxed);
        let mut message = json!({ "id": id, "method": method, "params": params });
        if let Some(session) = session {
            message["sessionId"] = json!(session);
        }
        let (sender, receiver) = channel();
        match self.0.pending.lock().expect("never poisoned").as_mut() {
            Some(pending) => pending.insert(id, sender),
            None => return Err(Error::new(format!("the browser has closed, so it cannot {method}"))),
        };
        let frame = encode_frame(0x1, message.to_string().as_bytes(), id as u32);
        let written = self.0.writer.lock().expect("never poisoned").write_all(&frame);
        if let Err(error) = written {
            self.forget(id);
            return Err(Error::new(format!("the browser closed the connection: {error}")));
        }

        Ok((id, receiver))
    }

    fn forget(&self, id: u64) {
        if let Some(pending) = self.0.pending.lock().expect("never poisoned").as_mut() {
            pending.remove(&id);
        }
    }

    fn pong(&self, payload: &[u8]) {
        let frame = encode_frame(0xA, payload, 0);
        let _ = self.0.writer.lock().expect("never poisoned").write_all(&frame);
    }
}

/// A client frame: final, masked (as clients must), with the length in however many bytes it needs.
fn encode_frame(opcode: u8, payload: &[u8], mask_seed: u32) -> Vec<u8> {
    let mask = mask_seed.wrapping_mul(2_654_435_761).to_be_bytes();
    let mut frame = vec![0x80 | opcode];
    match payload.len() {
        length @ 0..=125 => frame.push(0x80 | length as u8),
        length @ 126..=0xffff => {
            frame.push(0x80 | 126);
            frame.extend((length as u16).to_be_bytes());
        }
        length => {
            frame.push(0x80 | 127);
            frame.extend((length as u64).to_be_bytes());
        }
    }
    frame.extend(mask);
    frame.extend(payload.iter().enumerate().map(|(index, byte)| byte ^ mask[index % 4]));

    frame
}

/// Reads one whole message, joining fragments and answering pings on the way.
fn read_message(reader: &mut impl Read, connection: &Connection) -> std::io::Result<Vec<u8>> {
    let mut message = Vec::new();
    loop {
        let mut head = [0u8; 2];
        reader.read_exact(&mut head)?;
        let (fin, opcode) = (head[0] & 0x80 != 0, head[0] & 0x0f);
        let length = match head[1] & 0x7f {
            126 => {
                let mut bytes = [0u8; 2];
                reader.read_exact(&mut bytes)?;
                u16::from_be_bytes(bytes) as u64
            }
            127 => {
                let mut bytes = [0u8; 8];
                reader.read_exact(&mut bytes)?;
                u64::from_be_bytes(bytes)
            }
            length => length as u64,
        };
        let mask = if head[1] & 0x80 != 0 {
            let mut bytes = [0u8; 4];
            reader.read_exact(&mut bytes)?;
            Some(bytes)
        } else {
            None
        };
        let mut payload = vec![0u8; length as usize];
        reader.read_exact(&mut payload)?;
        if let Some(mask) = mask {
            payload.iter_mut().enumerate().for_each(|(index, byte)| *byte ^= mask[index % 4]);
        }

        match opcode {
            0x8 => return Err(std::io::ErrorKind::ConnectionAborted.into()),
            0x9 => connection.pong(&payload),
            0xA => {}
            _ => {
                message.extend(payload);
                if fin {
                    return Ok(message);
                }
            }
        }
    }
}

/// Standard base64, which is how the browser sends pictures.
pub(crate) fn base64_decode(text: &str) -> Result<Vec<u8>> {
    let value = |byte: u8| match byte {
        b'A'..=b'Z' => Some(byte - b'A'),
        b'a'..=b'z' => Some(byte - b'a' + 26),
        b'0'..=b'9' => Some(byte - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    };
    let digits: Vec<u8> = text.bytes().filter(|byte| !byte.is_ascii_whitespace() && *byte != b'=').collect();
    let mut bytes = Vec::with_capacity(digits.len() * 3 / 4);
    for chunk in digits.chunks(4) {
        let mut buffer = 0u32;
        for (index, digit) in chunk.iter().enumerate() {
            buffer |= (value(*digit).ok_or_else(|| Error::new("not base64"))? as u32) << (18 - 6 * index);
        }
        bytes.extend(&buffer.to_be_bytes()[1..chunk.len()]);
    }

    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use std::net::TcpListener;

    #[test]
    fn base64_decodes_every_padding() {
        assert_eq!(base64_decode("aGVsbG8=").unwrap(), b"hello");
        assert_eq!(base64_decode("aGVsbG8h").unwrap(), b"hello!");
        assert_eq!(base64_decode("aGk=").unwrap(), b"hi");
        assert_eq!(base64_decode("").unwrap(), b"");
        assert_eq!(base64_decode("+/+/").unwrap(), [0xfb, 0xff, 0xbf]);
        assert!(base64_decode("a$==").is_err());
    }

    #[test]
    fn frames_carry_their_length_in_as_few_bytes_as_fit() {
        assert_eq!(encode_frame(1, &[0; 125], 0)[1], 0x80 | 125);
        assert_eq!(encode_frame(1, &[0; 126], 0)[1..4], [0x80 | 126, 0, 126]);
        assert_eq!(encode_frame(1, &vec![0; 70_000], 0)[1], 0x80 | 127);
    }

    /// A server that answers the handshake, then echoes each call's id back with a result, sends
    /// one event, and pings once.
    fn fake_browser() -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut writer = stream;
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if line == "\r\n" {
                    break;
                }
            }
            writer.write_all(b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\n\r\n").unwrap();
            let server_frame = |opcode: u8, payload: &[u8]| {
                let mut frame = vec![0x80 | opcode, payload.len() as u8];
                frame.extend(payload);
                frame
            };
            writer.write_all(&server_frame(0x9, b"hi")).unwrap();
            let event = json!({ "method": "Page.loadEventFired", "params": { "timestamp": 1 } }).to_string();
            writer.write_all(&server_frame(0x1, event.as_bytes())).unwrap();
            // Answer calls until the client goes away; a fragmented answer on the second.
            let mut calls = 0;
            loop {
                let mut head = [0u8; 2];
                if reader.read_exact(&mut head).is_err() {
                    return;
                }
                let mut length = (head[1] & 0x7f) as usize;
                if length == 126 {
                    let mut bytes = [0u8; 2];
                    reader.read_exact(&mut bytes).unwrap();
                    length = u16::from_be_bytes(bytes) as usize;
                }
                let mut mask = [0u8; 4];
                reader.read_exact(&mut mask).unwrap();
                let mut payload = vec![0u8; length];
                reader.read_exact(&mut payload).unwrap();
                payload.iter_mut().enumerate().for_each(|(index, byte)| *byte ^= mask[index % 4]);
                if head[0] & 0x0f != 1 {
                    continue;
                }
                let call: Value = serde_json::from_slice(&payload).unwrap();
                calls += 1;
                let answer = if call["method"] == "Fail.please" {
                    json!({ "id": call["id"], "error": { "message": "as asked" } })
                } else {
                    json!({ "id": call["id"], "result": { "echo": call["params"] } })
                };
                let bytes = answer.to_string().into_bytes();
                if calls == 2 {
                    let (first, rest) = bytes.split_at(5);
                    writer.write_all(&[0x01, first.len() as u8]).unwrap();
                    writer.write_all(first).unwrap();
                    writer.write_all(&server_frame(0x0, rest)).unwrap();
                } else {
                    writer.write_all(&server_frame(0x1, &bytes)).unwrap();
                }
            }
        });

        port
    }

    #[test]
    fn calls_get_their_answers_and_events_reach_the_handler() {
        let port = fake_browser();
        let (sender, events) = channel();
        let sender = Mutex::new(sender);
        let connection = Connection::open(
            port,
            "/devtools/browser/x",
            Box::new(move |_, method, _, _| sender.lock().unwrap().send(method.to_string()).unwrap()),
        )
        .unwrap();

        assert_eq!(connection.call("Echo.one", json!({ "n": 1 }), None).unwrap(), json!({ "echo": { "n": 1 } }));
        assert_eq!(connection.call("Echo.two", json!({ "n": 2 }), Some("s")).unwrap(), json!({ "echo": { "n": 2 } }));
        let error = connection.call("Fail.please", json!({}), None).unwrap_err();
        assert_eq!(error.message, "Fail.please: as asked");
        assert_eq!(events.recv_timeout(Duration::from_secs(5)).unwrap(), "Page.loadEventFired");
    }

    #[test]
    fn calls_after_the_browser_has_gone_fail_at_once() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buffer = [0u8; 1024];
            let _ = stream.read(&mut buffer);
            stream.write_all(b"HTTP/1.1 101 Switching Protocols\r\n\r\n").unwrap();
            // A close frame: the browser is gone.
            stream.write_all(&[0x88, 0]).unwrap();
            std::thread::sleep(Duration::from_millis(500));
        });
        let connection = Connection::open(port, "/", Box::new(|_, _, _, _| {})).unwrap();
        std::thread::sleep(Duration::from_millis(100));

        let started = std::time::Instant::now();
        assert!(connection.call("Page.enable", json!({}), None).is_err());
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn a_refused_handshake_is_an_error() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buffer = [0u8; 1024];
            let _ = stream.read(&mut buffer);
            stream.write_all(b"HTTP/1.1 404 Not Found\r\n\r\n").unwrap();
        });

        let error = Connection::open(port, "/nope", Box::new(|_, _, _, _| {})).err().unwrap();
        assert!(error.message.contains("refused"), "{}", error.message);
    }

    #[test]
    fn a_long_message_is_read_whole() {
        let payload = vec![b'x'; 70_000];
        let mut frame = vec![0x81, 127];
        frame.extend((payload.len() as u64).to_be_bytes());
        frame.extend(&payload);
        let port = fake_browser();
        let connection = Connection::open(port, "/", Box::new(|_, _, _, _| {})).unwrap();

        assert_eq!(read_message(&mut Cursor::new(frame), &connection).unwrap().len(), 70_000);
    }
}
