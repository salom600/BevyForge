//! Length-prefixed TCP transport for [`crate::Message`]s.
//!
//! Wire format per message: `[u32 big-endian payload length][postcard payload]`.
//! A single 128 KiB read buffer cap guards against hostile/corrupt streams.

use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream};

use crate::Message;

/// 64 MiB — generous ceiling for a 4K RGB frame (~24 MiB) with headroom.
const MAX_MESSAGE_BYTES: u32 = 64 * 1024 * 1024;

/// A bidirectional connection carrying [`Message`]s.
pub struct Connection {
    stream: TcpStream,
}

impl Connection {
    /// Wrap an already-connected socket.
    pub fn new(stream: TcpStream) -> io::Result<Self> {
        stream.set_nodelay(true)?;
        Ok(Self { stream })
    }

    /// Duplicate the underlying socket so a second thread can write while this
    /// one reads (TCP streams are internally synchronised per direction).
    pub fn clone_stream(&self) -> io::Result<TcpStream> {
        self.stream.try_clone()
    }

    /// Serialise and blocking-send one message.
    pub fn send(&mut self, msg: &Message) -> io::Result<()> {
        let payload = postcard::to_allocvec(msg)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        let len = payload.len() as u32;
        if len > MAX_MESSAGE_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("message too large: {len} bytes"),
            ));
        }
        self.stream.write_all(&len.to_be_bytes())?;
        self.stream.write_all(&payload)?;
        self.stream.flush()
    }

    /// Blocking-receive one message.
    pub fn recv(&mut self) -> io::Result<Message> {
        let mut len_buf = [0u8; 4];
        self.read_exact_interruptible(&mut len_buf)?;
        let len = u32::from_be_bytes(len_buf);
        if len > MAX_MESSAGE_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("announced message too large: {len} bytes"),
            ));
        }
        let mut payload = vec![0u8; len as usize];
        self.read_exact_interruptible(&mut payload)?;
        postcard::from_bytes(&payload)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
    }

    /// `read_exact`, but aborts cleanly on connection close.
    fn read_exact_interruptible(&mut self, buf: &mut [u8]) -> io::Result<()> {
        self.stream.read_exact(buf)
    }

    /// Switch the socket to non-blocking mode (used by editor poll loops).
    pub fn set_nonblocking(&mut self, nonblocking: bool) -> io::Result<()> {
        self.stream.set_nonblocking(nonblocking)
    }

    /// One non-blocking receive; `Ok(None)` when no data is ready yet.
    pub fn try_recv(&mut self) -> io::Result<Option<Message>> {
        match self.recv() {
            Ok(msg) => Ok(Some(msg)),
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => Ok(None),
            Err(e) => Err(e),
        }
    }
}

/// Frame + send one message on a raw stream (used by pump threads that split
/// the socket via [`Connection::clone_stream`]).
pub fn send_on_stream(stream: &mut TcpStream, msg: &Message) -> io::Result<()> {
    let payload = postcard::to_allocvec(msg)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    let len = payload.len() as u32;
    if len > MAX_MESSAGE_BYTES {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "message too large"));
    }
    stream.write_all(&len.to_be_bytes())?;
    stream.write_all(&payload)?;
    stream.flush()
}

/// Bind the runtime's listener. Port `0` asks the OS for a free port —
/// the runtime then reports the chosen port on stdout (`FORGE_PORT=<n>`)
/// so the spawner does not need to guess.
pub fn listen(port: u16) -> io::Result<TcpListener> {
    TcpListener::bind(("127.0.0.1", port))
}

/// Editor-side connect with timeout.
pub fn connect(port: u16, timeout: std::time::Duration) -> io::Result<Connection> {
    let addr = format!("127.0.0.1:{port}");
    let stream = connect_timeout(&addr, timeout)?;
    Connection::new(stream)
}

/// Runtime-side relay: accepts the editor's connection on `listener` and pumps
/// messages between the socket and the two channels for the process lifetime.
///
/// * inbound editor commands arrive on `cmd_tx`
/// * outbound runtime events leave via `evt_rx`
///
/// Runs detached on its own threads. Reconnects automatically when the editor
/// drops the socket, but only LINGERS: if no editor connects within
/// [`LINGER_SECS`] after a disconnect (or [`FIRST_CONNECT_SECS`] after boot),
/// the whole runtime process exits so no orphan engine keeps holding the IPC
/// port after the editor died.
pub fn spawn_relay(
    listener: std::net::TcpListener,
    cmd_tx: crossbeam_channel::Sender<crate::EditorToRuntime>,
    evt_rx: crossbeam_channel::Receiver<crate::RuntimeToEditor>,
) {
    std::thread::spawn(move || {
        const LINGER_SECS: f64 = 10.0;
        const FIRST_CONNECT_SECS: f64 = 60.0;
        let mut idle_since = std::time::Instant::now();
        let mut first_wait = true;
        loop {
            // Poll accept so the linger deadline can be enforced.
            listener
                .set_nonblocking(true)
                .expect("listener nonblocking");
            match listener.accept() {
                Ok((stream, _peer)) => {
                    first_wait = false;
                    listener
                        .set_nonblocking(false)
                        .expect("listener blocking");
                    serve_connection(stream, &cmd_tx, &evt_rx);
                    // Editor disconnected — restart the linger window.
                    idle_since = std::time::Instant::now();
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    let budget = if first_wait { FIRST_CONNECT_SECS } else { LINGER_SECS };
                    if idle_since.elapsed().as_secs_f64() > budget {
                        eprintln!(
                            "[relay] no editor connected for {budget:.0}s — engine exiting"
                        );
                        std::process::exit(0);
                    }
                    std::thread::sleep(std::time::Duration::from_millis(150));
                }
                Err(_) => return, // listener closed: runtime shutting down
            }
        }
    });
}

/// Serve one editor connection until the socket drops (blocking).
fn serve_connection(
    stream: std::net::TcpStream,
    cmd_tx: &crossbeam_channel::Sender<crate::EditorToRuntime>,
    evt_rx: &crossbeam_channel::Receiver<crate::RuntimeToEditor>,
) {
    let Ok(conn) = Connection::new(stream) else { return };

    let cmd_tx = cmd_tx.clone();
    // recv thread: socket -> cmd channel
    let read_conn = match conn.stream.try_clone() {
        Ok(s) => s,
        Err(_) => return,
    };
    let mut read_conn = Connection { stream: read_conn };
    let reader = std::thread::spawn(move || loop {
        match read_conn.recv() {
            Ok(Message::ToRuntime(cmd)) => {
                if cmd_tx.send(cmd).is_err() {
                    return;
                }
            }
            Ok(_) => { /* wrong direction; ignore */ }
            Err(e) => {
                eprintln!("[relay] reader exit: {e}");
                return; // disconnected
            }
        }
    });

    // send thread: evt channel -> socket (clone so reconnects keep working)
    let mut write_conn = conn;
    let evt_rx = evt_rx.clone();
    let writer = std::thread::spawn(move || loop {
        match evt_rx.recv_timeout(std::time::Duration::from_millis(250)) {
            Ok(evt) => {
                if write_conn.send(&Message::ToEditor(evt)).is_err() {
                    return;
                }
            }
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => continue,
            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => return,
        }
    });

    let _ = reader.join();
    let _ = writer.join();
    // Editor disconnected — caller restarts the linger window.
}

fn connect_timeout(addr: &str, timeout: std::time::Duration) -> io::Result<TcpStream> {
    use std::net::ToSocketAddrs;
    let sock_addr = addr
        .to_socket_addrs()?
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "no address"))?;
    let stream = TcpStream::connect_timeout(&sock_addr, timeout)?;
    Ok(stream)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{EditorToRuntime, RuntimeToEditor, Stats};

    #[test]
    fn roundtrip_over_socketpair() {
        let listener = listen(0).unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut conn = Connection::new(stream).unwrap();
            conn.send(&Message::ToEditor(RuntimeToEditor::Stats(Stats {
                fps: 120.0,
                ..Default::default()
            })))
            .unwrap();
        });

        let mut client = connect(port, std::time::Duration::from_secs(2)).unwrap();
        handle.join().unwrap();
        let msg = client.recv().unwrap();
        match msg {
            Message::ToEditor(RuntimeToEditor::Stats(s)) => assert_eq!(s.fps, 120.0),
            _ => panic!("wrong message"),
        }

        // And editor -> runtime direction.
        let listener2 = listen(0).unwrap();
        let port2 = listener2.local_addr().unwrap().port();
        let handle2 = std::thread::spawn(move || {
            let (stream, _) = listener2.accept().unwrap();
            let mut conn = Connection::new(stream).unwrap();
            conn.recv().unwrap()
        });
        let mut client2 = connect(port2, std::time::Duration::from_secs(2)).unwrap();
        client2
            .send(&Message::ToRuntime(EditorToRuntime::Ping(7)))
            .unwrap();
        let got = handle2.join().unwrap();
        assert!(matches!(got, Message::ToRuntime(EditorToRuntime::Ping(7))));
    }
}
