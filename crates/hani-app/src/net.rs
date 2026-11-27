// SPDX-License-Identifier: AGPL-3.0-only

//! The application's network requests, without an HTTP library.
//!
//! There are two, and they want different things:
//!
//! - **The printer** (`printer.rs`) is asked over plain HTTP on the local
//!   network. [`get_local`] is a GET over `std::net::TcpStream`: one request,
//!   `Connection: close`, the reply read to its length, its chunks or its end.
//! - **The updater** (`update.rs`) fetches from GitHub over HTTPS, which wants
//!   TLS. [`Curl`] runs the system's `curl`, which every supported platform
//!   ships (Windows 10 1803 and later, macOS, and nearly every Linux), one hop
//!   at a time: `--max-redirs 0`, so the updater still checks the host of every
//!   redirect itself, and `--include`, so the status and the `Location` come
//!   back ahead of the body and the body is streamed, never buffered.
//!
//! This replaced `ureq` (with rustls, its root certificates and ring's use of
//! them) on 2026-11-27, when the bug report to TinkerAtlas, its third user,
//! was retired. See `docs/DEPENDENCIES.md`, proposal 4. The trust store is now
//! the system's, as a browser's is. What `curl` hands back is trusted no more
//! than `ureq`'s was: an update is checked against its signature and its
//! digest before anything is made executable.

use std::io::{BufRead, BufReader, Read, Write};
use std::time::Duration;

/// The longest a reply's status line and headers may be.
const MOST_HEAD_BYTES: usize = 64 * 1024;

/// Named so a log on someone's own network says who is calling; GitHub also
/// refuses a request with none.
pub const USER_AGENT: &str = "HaniSculpt";

/// A reply's status and the headers that matter here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Head {
    pub status: u16,
    pub location: Option<String>,
    pub length: Option<u64>,
    pub chunked: bool,
}

/// Read a status line and headers, up to and including the blank line.
///
/// An HTTP/1.x or HTTP/2 status line (`curl --include` writes `HTTP/2 302`).
/// Interim `100 Continue` replies are skipped.
pub fn read_head(reader: &mut impl BufRead) -> std::io::Result<Head> {
    let bad = |why: &str| std::io::Error::new(std::io::ErrorKind::InvalidData, why.to_string());
    let mut taken = 0usize;
    loop {
        let mut line = String::new();
        let mut head = Head { status: 0, location: None, length: None, chunked: false };
        loop {
            line.clear();
            let read = reader.read_line(&mut line)?;
            taken += read;
            if read == 0 {
                return Err(bad("the reply ended before its headers did"));
            }
            if taken > MOST_HEAD_BYTES {
                return Err(bad("the reply's headers are too long"));
            }
            let trimmed = line.trim_end_matches(['\r', '\n']);
            if head.status == 0 {
                let mut parts = trimmed.split_whitespace();
                let version = parts.next().unwrap_or_default();
                if !version.starts_with("HTTP/") {
                    return Err(bad("the reply has no status line"));
                }
                head.status = parts
                    .next()
                    .and_then(|code| code.parse().ok())
                    .ok_or_else(|| bad("the reply's status is not a number"))?;
                continue;
            }
            if trimmed.is_empty() {
                break;
            }
            let Some((name, value)) = trimmed.split_once(':') else {
                continue;
            };
            let value = value.trim();
            match name.trim().to_ascii_lowercase().as_str() {
                "location" => head.location = Some(value.to_string()),
                "content-length" => head.length = value.parse().ok(),
                "transfer-encoding" => {
                    head.chunked = value.to_ascii_lowercase().contains("chunked");
                }
                _ => {}
            }
        }
        if head.status != 100 {
            return Ok(head);
        }
    }
}

/// A chunked body, decoded, up to `cap` bytes.
fn read_chunked(reader: &mut impl BufRead, cap: usize) -> std::io::Result<Vec<u8>> {
    let bad = |why: &str| std::io::Error::new(std::io::ErrorKind::InvalidData, why.to_string());
    let mut body = Vec::new();
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            return Err(bad("the reply ended inside its chunks"));
        }
        let size = line.trim().split(';').next().unwrap_or_default();
        let size = usize::from_str_radix(size, 16).map_err(|_| bad("a chunk with no size"))?;
        if size == 0 {
            // Trailers, if any, to the blank line.
            loop {
                line.clear();
                if reader.read_line(&mut line)? == 0 || line.trim().is_empty() {
                    return Ok(body);
                }
            }
        }
        if body.len() + size > cap {
            return Err(bad("the reply is too long"));
        }
        let start = body.len();
        body.resize(start + size, 0);
        reader.read_exact(&mut body[start..])?;
        line.clear();
        reader.read_line(&mut line)?;
    }
}

/// The body that follows `head`, up to `cap` bytes.
fn read_body(reader: &mut impl BufRead, head: &Head, cap: usize) -> std::io::Result<Vec<u8>> {
    let too_long = || std::io::Error::new(std::io::ErrorKind::InvalidData, "the reply is too long");
    if head.chunked {
        return read_chunked(reader, cap);
    }
    let mut body = Vec::new();
    match head.length {
        Some(length) => {
            if length > cap as u64 {
                return Err(too_long());
            }
            reader.take(length).read_to_end(&mut body)?;
        }
        None => {
            reader.take(cap as u64 + 1).read_to_end(&mut body)?;
            if body.len() > cap {
                return Err(too_long());
            }
        }
    }
    Ok(body)
}

/// GET `path` from `host:port` over plain HTTP, and the body as text, or why
/// not. For a machine on the local network; nothing here is encrypted.
pub fn get_local(
    host: &str,
    port: u16,
    path: &str,
    timeout: Duration,
    cap: usize,
) -> Result<String, String> {
    use std::net::ToSocketAddrs;
    let address = (host, port)
        .to_socket_addrs()
        .map_err(|why| format!("could not find {host} ({why})"))?
        .next()
        .ok_or_else(|| format!("could not find {host}"))?;
    let mut stream = std::net::TcpStream::connect_timeout(&address, timeout)
        .map_err(|why| format!("could not reach {host} ({why})"))?;
    let unreadable = |why: std::io::Error| format!("{host} answered something unreadable ({why})");
    stream.set_read_timeout(Some(timeout)).map_err(unreadable)?;
    stream.set_write_timeout(Some(timeout)).map_err(unreadable)?;
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: {host}:{port}\r\nUser-Agent: {USER_AGENT}\r\n\
         Accept: application/json\r\nConnection: close\r\n\r\n"
    )
    .map_err(|why| format!("could not reach {host} ({why})"))?;
    let mut reader = BufReader::new(stream);
    let head = read_head(&mut reader).map_err(unreadable)?;
    if head.status != 200 {
        return Err(format!("{host} answered {}", head.status));
    }
    let body = read_body(&mut reader, &head, cap).map_err(unreadable)?;
    String::from_utf8(body).map_err(|_| format!("{host} answered something that is not text"))
}

/// How long a fetch may take.
#[derive(Debug, Clone, Copy)]
pub enum Patience {
    /// The whole request within this.
    Total(Duration),
    /// No deadline, but given up if nothing at all arrives for this long:
    /// a slow line may finish, a dead one may not.
    Stall(Duration),
}

/// One HTTPS GET through the system's `curl`, its status and headers read,
/// its body still to come. Redirects are not followed: the caller checks the
/// next hop and asks again.
pub struct Curl {
    child: std::process::Child,
    pub head: Head,
    body: BufReader<std::process::ChildStdout>,
}

impl Curl {
    /// Ask for `url`, which must be `https://`.
    pub fn get(url: &str, patience: Patience) -> Result<Curl, String> {
        if !url.starts_with("https://") {
            return Err("only https is fetched".to_string());
        }
        let mut command =
            std::process::Command::new(if cfg!(windows) { "curl.exe" } else { "curl" });
        // `-q` first: no `.curlrc` of the user's changes what is asked.
        command.args([
            "-q",
            "--silent",
            "--show-error",
            "--include",
            "--suppress-connect-headers",
            "--max-redirs",
            "0",
            "--proto",
            "=https",
            "--tlsv1.2",
            "--connect-timeout",
            "10",
            "--user-agent",
            USER_AGENT,
        ]);
        match patience {
            Patience::Total(limit) => {
                command.args(["--max-time", &limit.as_secs().max(1).to_string()]);
            }
            Patience::Stall(limit) => {
                command.args([
                    "--speed-limit",
                    "1",
                    "--speed-time",
                    &limit.as_secs().max(1).to_string(),
                ]);
            }
        }
        command
            .arg("--")
            .arg(url)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        // No console window flashing up on Windows.
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x0800_0000);
        }
        let mut child = command.spawn().map_err(|why| match why.kind() {
            std::io::ErrorKind::NotFound => "curl is not installed".to_string(),
            _ => format!("could not run curl ({why})"),
        })?;
        let stdout = child.stdout.take().ok_or("curl gave no output")?;
        let mut body = BufReader::new(stdout);
        match read_head(&mut body) {
            Ok(head) => Ok(Curl { child, head, body }),
            Err(_) => {
                let why = Self::finish(child);
                Err(why.err().unwrap_or_else(|| "the reply could not be read".to_string()))
            }
        }
    }

    /// The body, to read; [`Curl::done`] afterwards says whether it all came.
    pub fn body(&mut self) -> &mut BufReader<std::process::ChildStdout> {
        &mut self.body
    }

    /// The body whole, up to `cap` bytes, and whether `curl` finished well.
    pub fn read_all(mut self, cap: u64) -> Result<Vec<u8>, String> {
        let mut body = Vec::new();
        (&mut self.body).take(cap + 1).read_to_end(&mut body).map_err(|why| why.to_string())?;
        if body.len() as u64 > cap {
            let _ = self.child.kill();
            let _ = self.child.wait();
            return Err(format!("the reply is longer than {cap} bytes"));
        }
        self.done()?;
        Ok(body)
    }

    /// Wait for `curl`, and its complaint if it stopped short.
    pub fn done(self) -> Result<(), String> {
        drop(self.body);
        Self::finish(self.child)
    }

    /// Stop early: a redirect, or a reply not wanted.
    pub fn abandon(mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }

    fn finish(mut child: std::process::Child) -> Result<(), String> {
        let mut complaint = String::new();
        if let Some(mut stderr) = child.stderr.take() {
            let _ = stderr.read_to_string(&mut complaint);
        }
        let status = child.wait().map_err(|why| why.to_string())?;
        if status.success() {
            Ok(())
        } else {
            let complaint = complaint.trim().trim_start_matches("curl: ").to_string();
            Err(if complaint.is_empty() { format!("curl stopped ({status})") } else { complaint })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn head_of(text: &str) -> Head {
        read_head(&mut text.as_bytes()).expect("a head")
    }

    #[test]
    fn a_head_gives_its_status_location_length_and_chunking() {
        let head = head_of(
            "HTTP/2 302\r\nlocation: https://release-assets.githubusercontent.com/x\r\n\
             content-length: 0\r\n\r\nrest",
        );
        assert_eq!(head.status, 302);
        assert_eq!(
            head.location.as_deref(),
            Some("https://release-assets.githubusercontent.com/x")
        );
        assert_eq!(head.length, Some(0));
        let head = head_of("HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n");
        assert!(head.chunked && head.status == 200);
        let head = head_of("HTTP/1.1 100 Continue\r\n\r\nHTTP/1.1 404 Not Found\r\n\r\n");
        assert_eq!(head.status, 404, "an interim reply was taken for the answer");
        assert!(read_head(&mut "garbage\r\n\r\n".as_bytes()).is_err());
        assert!(read_head(&mut "HTTP/1.1 200 OK\r\nx: y".as_bytes()).is_err(), "cut short");
    }

    #[test]
    fn a_body_is_read_by_length_by_chunks_or_to_the_end_and_capped() {
        let by_length = Head { status: 200, location: None, length: Some(5), chunked: false };
        assert_eq!(read_body(&mut "helloEXTRA".as_bytes(), &by_length, 64).unwrap(), b"hello");
        let chunked = Head { chunked: true, length: None, ..by_length.clone() };
        let text = "5\r\nhello\r\n6;x=y\r\n world\r\n0\r\nTrailer: z\r\n\r\n";
        assert_eq!(read_body(&mut text.as_bytes(), &chunked, 64).unwrap(), b"hello world");
        let to_end = Head { length: None, ..by_length.clone() };
        assert_eq!(read_body(&mut "all of it".as_bytes(), &to_end, 64).unwrap(), b"all of it");
        assert!(read_body(&mut "too long".as_bytes(), &to_end, 3).is_err());
        assert!(read_body(&mut "hello".as_bytes(), &by_length, 3).is_err());
        assert!(read_body(&mut text.as_bytes(), &chunked, 7).is_err());
    }

    /// A real socket: a server on this machine answering once, by length and
    /// then by chunks, and an answer that is not 200.
    #[test]
    fn a_local_get_reads_the_reply_over_a_real_socket() {
        let serve = |reply: &'static str| {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let port = listener.local_addr().unwrap().port();
            let server = std::thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                let mut asked = [0u8; 1024];
                let read = stream.read(&mut asked).unwrap();
                stream.write_all(reply.as_bytes()).unwrap();
                String::from_utf8_lossy(&asked[..read]).into_owned()
            });
            (port, server)
        };
        let timeout = Duration::from_secs(5);
        let (port, server) = serve("HTTP/1.1 200 OK\r\nContent-Length: 11\r\n\r\n{\"ok\":true}");
        let body = get_local("127.0.0.1", port, "/printer/objects/query?x", timeout, 1024);
        assert_eq!(body.as_deref(), Ok("{\"ok\":true}"));
        let asked = server.join().unwrap();
        assert!(asked.starts_with("GET /printer/objects/query?x HTTP/1.1\r\n"), "{asked}");
        assert!(asked.contains(&format!("Host: 127.0.0.1:{port}\r\n")), "{asked}");

        let (port, server) =
            serve("HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n2\r\n{}\r\n0\r\n\r\n");
        assert_eq!(get_local("127.0.0.1", port, "/", timeout, 1024).as_deref(), Ok("{}"));
        server.join().unwrap();

        let (port, server) = serve("HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n");
        assert_eq!(
            get_local("127.0.0.1", port, "/", timeout, 1024),
            Err("127.0.0.1 answered 404".into())
        );
        server.join().unwrap();
    }

    #[test]
    fn only_https_is_fetched() {
        assert!(Curl::get("http://github.com/", Patience::Total(Duration::from_secs(1))).is_err());
    }
}
