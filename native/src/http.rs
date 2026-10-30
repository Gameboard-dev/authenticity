//! HTTP/1.1 on this computer only: a client that speaks to 127.0.0.1 (SlopTotal, the helper, the optional embedding
//! search, the other services' GPU release) and the helper's server side (one request per connection). No address is ever taken
//! from anywhere but a port number: every connection goes to 127.0.0.1. Sizes are capped (headers 16 KB, bodies 4 MB
//! in, 64 MB out) so a misbehaving program on a port cannot make this one hold more.
//!
//! References: RFC 9112 (HTTP/1.1 message syntax, chunked transfer coding).

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::time::Duration;

/// The most a request body may hold, and an answer.
pub const MAX_BODY: usize = 4 << 20;
const MAX_ANSWER: usize = 64 << 20;
const MAX_HEAD: usize = 16 << 10;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Reply {
    pub status: u16,
    pub body: String,
}

/// A request to 127.0.0.1:`port`. Err: nothing answered (the reason).
pub fn request(port: u16, method: &str, path: &str, body: Option<&str>, timeout: Duration) -> Result<Reply, String> {
    request_with(port, method, path, body, timeout, &[])
}

/// `request` with extra header lines (`Authorization: Bearer <token>` for Quarry's service). A header with a line
/// break is refused: it would end the request's head.
pub fn request_with(port: u16, method: &str, path: &str, body: Option<&str>, timeout: Duration, headers: &[(&str, &str)]) -> Result<Reply, String> {
    if headers.iter().any(|(k, v)| format!("{k}{v}").contains(['\r', '\n'])) {
        return Err("a header holds a line break".into());
    }
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let mut s = TcpStream::connect_timeout(&addr, Duration::from_millis(1500)).map_err(|e| e.to_string())?;
    s.set_read_timeout(Some(timeout)).ok();
    s.set_write_timeout(Some(Duration::from_secs(30))).ok();
    let body = body.unwrap_or("");
    // No `Connection: close`: asked to close, a local service (hyper) reset the connection part-way through a large
    // answer on Windows (2026-10-29: 587 of 658 KB arrived, then error 10054). The answer is read up to its length
    // instead, and the connection dropped then.
    let mut head = format!("{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAccept: application/json\r\n");
    for (k, v) in headers {
        head += &format!("{k}: {v}\r\n");
    }
    if method != "GET" {
        head += &format!("Content-Type: application/json\r\nContent-Length: {}\r\n", body.len());
    }
    head += "\r\n";
    s.write_all(head.as_bytes()).and_then(|_| s.write_all(body.as_bytes())).map_err(|e| e.to_string())?;
    let mut raw = Vec::new();
    let mut buf = vec![0u8; 64 << 10];
    loop {
        match s.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                raw.extend_from_slice(&buf[..n]);
                if raw.len() >= MAX_ANSWER || complete(&raw) {
                    break;
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            // a connection closed abruptly once the whole answer is in has done its work
            Err(_) if complete(&raw) => break,
            Err(e) => return Err(e.to_string()),
        }
    }
    raw.truncate(MAX_ANSWER);
    parse_reply(&raw)
}

/// Whether `raw` holds a whole answer: its header and the body its Content-Length gives, or a chunked body's last
/// chunk. An answer with neither ends when the connection closes.
fn complete(raw: &[u8]) -> bool {
    let Some(split) = raw.windows(4).position(|w| w == b"\r\n\r\n") else { return false };
    let head = String::from_utf8_lossy(&raw[..split]);
    let rest = &raw[split + 4..];
    if header(&head, "transfer-encoding").is_some_and(|v| v.eq_ignore_ascii_case("chunked")) {
        return dechunk(rest).is_ok();
    }
    header(&head, "content-length").and_then(|v| v.parse::<usize>().ok()).is_some_and(|n| rest.len() >= n)
}

fn header<'a>(head: &'a str, name: &str) -> Option<&'a str> {
    head.lines().skip(1).find_map(|l| l.split_once(':').filter(|(k, _)| k.trim().eq_ignore_ascii_case(name)).map(|(_, v)| v.trim()))
}

/// An answer's status and body (chunked or not).
pub fn parse_reply(raw: &[u8]) -> Result<Reply, String> {
    let split = raw.windows(4).position(|w| w == b"\r\n\r\n").ok_or("an answer without a header")?;
    let head = String::from_utf8_lossy(&raw[..split]).into_owned();
    let status: u16 = head.split_whitespace().nth(1).and_then(|s| s.parse().ok()).ok_or("an answer without a status")?;
    let rest = &raw[split + 4..];
    let body = if header(&head, "transfer-encoding").is_some_and(|v| v.eq_ignore_ascii_case("chunked")) {
        dechunk(rest)?
    } else {
        match header(&head, "content-length").and_then(|v| v.parse::<usize>().ok()) {
            Some(n) => rest[..n.min(rest.len())].to_vec(),
            None => rest.to_vec(),
        }
    };
    Ok(Reply { status, body: String::from_utf8_lossy(&body).into_owned() })
}

fn dechunk(mut b: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    loop {
        let line_end = b.windows(2).position(|w| w == b"\r\n").ok_or("a damaged chunked answer")?;
        let size_text = String::from_utf8_lossy(&b[..line_end]);
        let size = usize::from_str_radix(size_text.split(';').next().unwrap_or("").trim(), 16).map_err(|_| "a damaged chunked answer")?;
        b = &b[line_end + 2..];
        if size == 0 {
            return Ok(out);
        }
        if b.len() < size {
            return Err("a chunked answer cut short".into());
        }
        out.extend_from_slice(&b[..size]);
        b = b.get(size + 2..).unwrap_or(&[]);
    }
}

// ----- the server side

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Request {
    pub method: String,
    pub path: String,
    /// Header names in lower case.
    pub headers: Vec<(String, String)>,
    pub body: String,
}

impl Request {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }
}

/// One request from a connection; Err with the status to answer.
pub fn read_request(s: &mut TcpStream) -> Result<Request, (u16, String)> {
    s.set_read_timeout(Some(Duration::from_secs(30))).ok();
    let mut r = BufReader::new(s.try_clone().map_err(|e| (500, e.to_string()))?);
    let mut head = Vec::new();
    loop {
        let mut line = Vec::new();
        let n = (&mut r).take((MAX_HEAD - head.len().min(MAX_HEAD)) as u64 + 1).read_until(b'\n', &mut line).map_err(|e| (400, e.to_string()))?;
        if n == 0 || head.len() + line.len() > MAX_HEAD {
            return Err((431, "the request's header is too long or cut short".into()));
        }
        let end = line == b"\r\n" || line == b"\n";
        head.extend_from_slice(&line);
        if end {
            break;
        }
    }
    let head = String::from_utf8_lossy(&head).into_owned();
    let mut first = head.lines().next().unwrap_or("").split_whitespace();
    let (method, path) = (first.next().unwrap_or("").to_string(), first.next().unwrap_or("").to_string());
    let headers: Vec<(String, String)> = head.lines().skip(1).filter_map(|l| l.split_once(':').map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string()))).collect();
    let req = Request { method, path, headers, body: String::new() };
    let len: usize = req.header("content-length").map_or(Ok(0), |v| v.parse()).map_err(|_| (400, "a Content-Length that is not a number".to_string()))?;
    if len > MAX_BODY {
        return Err((413, format!("the body is over {} MB", MAX_BODY >> 20)));
    }
    if req.header("transfer-encoding").is_some() {
        return Err((411, "send a Content-Length".into()));
    }
    let mut body = vec![0u8; len];
    r.read_exact(&mut body).map_err(|e| (400, e.to_string()))?;
    Ok(Request { body: String::from_utf8(body).map_err(|_| (400, "the body is not UTF-8".to_string()))?, ..req })
}

pub fn respond(s: &mut TcpStream, status: u16, body: &str) {
    let reason = match status {
        200 => "OK",
        202 => "Accepted",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        409 => "Conflict",
        411 => "Length Required",
        413 => "Content Too Large",
        429 => "Too Many Requests",
        431 => "Request Header Fields Too Large",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        _ => "Error",
    };
    let head = format!("HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\n\r\n", body.len());
    let _ = s.write_all(head.as_bytes()).and_then(|_| s.write_all(body.as_bytes()));
}

/// Whether a request may be served: addressed to this port on this computer (no other host name: a web page that
/// renamed itself to 127.0.0.1 cannot reach it) and from no browser origin but Obsidian's. A POST other than the GPU
/// release must be JSON, which a web page cannot send here without a preflight this server never answers.
pub fn allowed(req: &Request, port: u16) -> Result<(), String> {
    let host = req.header("host").unwrap_or("");
    if host != format!("127.0.0.1:{port}") && host != format!("localhost:{port}") {
        return Err("only requests to 127.0.0.1 are served".into());
    }
    if let Some(o) = req.header("origin") {
        if o != "app://obsidian.md" {
            return Err("requests from web pages are refused".into());
        }
    }
    if req.method == "POST" && req.path != "/gpu/release" && !req.header("content-type").is_some_and(|c| c.starts_with("application/json")) {
        return Err("send JSON (Content-Type: application/json)".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(headers: &[(&str, &str)], method: &str, path: &str) -> Request {
        Request { method: method.into(), path: path.into(), headers: headers.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(), body: String::new() }
    }

    #[test]
    fn answers_plain_and_chunked() {
        let plain = b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}extra";
        assert_eq!(parse_reply(plain).unwrap(), Reply { status: 200, body: "{}".into() });
        let chunked = b"HTTP/1.1 202 Accepted\r\nTransfer-Encoding: chunked\r\n\r\n4\r\n{\"a\"\r\n3\r\n:1}\r\n0\r\n\r\n";
        assert_eq!(parse_reply(chunked).unwrap().body, "{\"a\":1}");
        assert!(parse_reply(b"garbage").is_err());
    }

    #[test]
    fn only_this_computer_and_obsidian() {
        assert!(allowed(&req(&[("host", "127.0.0.1:8791")], "GET", "/health"), 8791).is_ok());
        assert!(allowed(&req(&[("host", "evil.example:8791")], "GET", "/health"), 8791).is_err(), "DNS rebinding");
        assert!(allowed(&req(&[("host", "127.0.0.1:8791"), ("origin", "https://evil.example")], "GET", "/health"), 8791).is_err());
        assert!(allowed(&req(&[("host", "127.0.0.1:8791"), ("origin", "app://obsidian.md"), ("content-type", "application/json")], "POST", "/sloptotal/start"), 8791).is_ok());
        assert!(allowed(&req(&[("host", "127.0.0.1:8791"), ("content-type", "text/plain")], "POST", "/sloptotal/start"), 8791).is_err(), "a form post");
        // the other services' release, as native-kit sends it (no body, no content type)
        assert!(allowed(&req(&[("host", "127.0.0.1:8791")], "POST", "/gpu/release"), 8791).is_ok());
    }

    #[test]
    fn a_round_trip_on_loopback() {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let t = std::thread::spawn(move || {
            let (mut s, _) = l.accept().unwrap();
            let r = read_request(&mut s).unwrap();
            respond(&mut s, 200, &format!("{{\"got\":{}}}", r.body));
            r
        });
        let reply = request(port, "POST", "/x", Some("{\"text\":\"hi\"}"), Duration::from_secs(5)).unwrap();
        assert_eq!(reply, Reply { status: 200, body: "{\"got\":{\"text\":\"hi\"}}".into() });
        let r = t.join().unwrap();
        assert_eq!((r.method.as_str(), r.path.as_str(), r.header("host")), ("POST", "/x", Some(format!("127.0.0.1:{port}").as_str())));
    }

    #[test]
    fn extra_headers_are_sent_and_a_line_break_is_refused() {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let t = std::thread::spawn(move || {
            let (mut s, _) = l.accept().unwrap();
            let r = read_request(&mut s).unwrap();
            respond(&mut s, 200, "{}");
            r
        });
        request_with(port, "POST", "/similar", Some("{}"), Duration::from_secs(5), &[("Authorization", "Bearer t")]).unwrap();
        assert_eq!(t.join().unwrap().header("authorization"), Some("Bearer t"));
        let e = request_with(port, "GET", "/", None, Duration::from_secs(1), &[("Authorization", "Bearer t\r\nX-Evil: 1")]);
        assert_eq!(e.unwrap_err(), "a header holds a line break", "refused before connecting");
    }

    /// A server that keeps the connection open after a large answer (as a keep-alive server does): the answer is read
    /// up to its length, without waiting for the connection to close.
    #[test]
    fn an_answer_is_read_to_its_length() {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let big = "x".repeat(700_000);
        let body = format!("{{\"a\":\"{big}\"}}");
        let sent = body.clone();
        let t = std::thread::spawn(move || {
            let (mut s, _) = l.accept().unwrap();
            let _ = read_request(&mut s).unwrap();
            let head = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", sent.len());
            s.write_all(head.as_bytes()).and_then(|_| s.write_all(sent.as_bytes())).unwrap();
            std::thread::sleep(Duration::from_secs(3)); // open, as keep-alive leaves it
        });
        let started = std::time::Instant::now();
        let reply = request(port, "GET", "/big", None, Duration::from_secs(10)).unwrap();
        assert_eq!(reply.body.len(), body.len());
        assert!(started.elapsed() < Duration::from_secs(2), "read to its length, not to the close");
        t.join().unwrap();
        assert!(complete(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}") && !complete(b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\n\r\n{}"));
        assert!(complete(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n2\r\n{}\r\n0\r\n\r\n") && !complete(b"HTTP/1.1 200 OK\r\n\r\n{}"));
    }
}
