//! A PostgreSQL protocol proxy that loses commit acknowledgments.
//!
//! It forwards a `COMMIT` to the server, waits until the server has answered,
//! and then drops both connections without relaying the answer. The server has
//! committed; the client only sees a dead connection. This is the one failure
//! that makes an application retry work that already happened.
//!
//! Only the simple query protocol and `sslmode=disable` are understood, which is
//! what Diesel's transaction manager uses for `COMMIT`. Use
//! [`AckProxy::url`] to point a connection at it.
use std::io::{self, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::thread;

struct State {
    upstream: (String, u16),
    /// Commits whose acknowledgment is still to be withheld.
    to_cut: AtomicUsize,
    /// Commits whose acknowledgment really was withheld.
    cut: AtomicUsize,
}

pub struct AckProxy {
    port: u16,
    state: Arc<State>,
}

impl AckProxy {
    /// Withholds the acknowledgment of the next `commits` COMMIT statements,
    /// across all connections, then passes everything through.
    pub fn start(database_url: &str, commits: usize) -> Self {
        let state = Arc::new(State {
            upstream: host_and_port(database_url),
            to_cut: AtomicUsize::new(commits),
            cut: AtomicUsize::new(0),
        });
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind proxy");
        let port = listener.local_addr().expect("proxy address").port();
        let shared = state.clone();
        thread::spawn(move || {
            for client in listener.incoming().flatten() {
                let state = shared.clone();
                thread::spawn(move || {
                    let _ = session(client, &state);
                });
            }
        });
        Self { port, state }
    }

    /// How many commit acknowledgments have been withheld so far.
    pub fn cuts(&self) -> usize {
        self.state.cut.load(Ordering::SeqCst)
    }

    /// `database_url` with its host and port replaced by this proxy's, and the
    /// negotiation the proxy cannot parse switched off.
    pub fn url(&self, database_url: &str) -> String {
        let (scheme, rest) = database_url.split_once("://").expect("URL with a scheme");
        let end = rest.find(['/', '?']).unwrap_or(rest.len());
        let (authority, tail) = rest.split_at(end);
        let credentials = authority.rsplit_once('@').map(|(c, _)| format!("{c}@"));
        let sep = if tail.contains('?') { '&' } else { '?' };
        format!(
            "{scheme}://{}127.0.0.1:{}{tail}{sep}sslmode=disable&gssencmode=disable",
            credentials.unwrap_or_default(),
            self.port
        )
    }
}

fn host_and_port(database_url: &str) -> (String, u16) {
    let rest = database_url.split_once("://").expect("URL with a scheme").1;
    let end = rest.find(['/', '?']).unwrap_or(rest.len());
    let authority = &rest[..end];
    let host_port = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    match host_port.rsplit_once(':') {
        Some((host, port)) if !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit()) => {
            (host.to_string(), port.parse().expect("port"))
        }
        _ => (host_port.to_string(), 5432),
    }
}

fn session(client: TcpStream, state: &Arc<State>) -> io::Result<()> {
    let server = TcpStream::connect((state.upstream.0.as_str(), state.upstream.1))?;
    let commit_seen = Arc::new(AtomicBool::new(false));
    let (client_back, server_back) = (client.try_clone()?, server.try_clone()?);

    let up_state = state.clone();
    let up_flag = commit_seen.clone();
    let up = thread::spawn(move || {
        let _ = client_to_server(client, server, &up_state, &up_flag);
    });
    let _ = server_to_client(server_back, client_back, state, &commit_seen);
    let _ = up.join();
    Ok(())
}

fn read_message(from: &mut TcpStream) -> io::Result<(u8, Vec<u8>)> {
    let mut kind = [0u8; 1];
    from.read_exact(&mut kind)?;
    let mut length = [0u8; 4];
    from.read_exact(&mut length)?;
    let mut body = vec![0u8; (u32::from_be_bytes(length) as usize).saturating_sub(4)];
    from.read_exact(&mut body)?;
    Ok((kind[0], body))
}

fn write_message(to: &mut TcpStream, kind: u8, body: &[u8]) -> io::Result<()> {
    to.write_all(&[kind])?;
    to.write_all(&((body.len() as u32 + 4).to_be_bytes()))?;
    to.write_all(body)
}

fn close(a: &TcpStream, b: &TcpStream) {
    let _ = a.shutdown(Shutdown::Both);
    let _ = b.shutdown(Shutdown::Both);
}

fn client_to_server(
    mut client: TcpStream,
    mut server: TcpStream,
    state: &State,
    commit_seen: &AtomicBool,
) -> io::Result<()> {
    let result = (|| {
        // The startup packet has no type byte.
        let mut length = [0u8; 4];
        client.read_exact(&mut length)?;
        let mut rest = vec![0u8; (u32::from_be_bytes(length) as usize).saturating_sub(4)];
        client.read_exact(&mut rest)?;
        server.write_all(&length)?;
        server.write_all(&rest)?;
        loop {
            let (kind, body) = read_message(&mut client)?;
            let is_commit =
                kind == b'Q' && body.len() >= 6 && body[..6].eq_ignore_ascii_case(b"COMMIT");
            if is_commit
                && state
                    .to_cut
                    .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
                    .is_ok()
            {
                commit_seen.store(true, Ordering::SeqCst);
            }
            write_message(&mut server, kind, &body)?;
        }
    })();
    close(&client, &server);
    result
}

fn server_to_client(
    mut server: TcpStream,
    mut client: TcpStream,
    state: &State,
    commit_seen: &AtomicBool,
) -> io::Result<()> {
    let result = (|| loop {
        let (kind, body) = read_message(&mut server)?;
        if commit_seen.load(Ordering::SeqCst) {
            // ReadyForQuery ends the answer to the COMMIT: the server is done.
            if kind == b'Z' {
                state.cut.fetch_add(1, Ordering::SeqCst);
                return Ok(());
            }
            continue;
        }
        write_message(&mut client, kind, &body)?;
    })();
    close(&client, &server);
    result
}
