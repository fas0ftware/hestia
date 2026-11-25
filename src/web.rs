//! `hestia web`: a read-only page on 127.0.0.1 that runs the checks on every refresh.
//! GET only, one request per connection, the Host header must name this machine.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::time::Duration;

use crate::checks::{self, Limits, Sys};
use crate::report::{self, Report};

const PAGE: &str = include_str!("../assets/index.html");
const JS: &str = include_str!("../assets/app.js");
const CSS: &str = include_str!("../assets/style.css");
const ICON: &str = include_str!("../assets/hestia.svg");
const CSP: &str = "default-src 'none'; script-src 'self'; style-src 'self'; connect-src 'self'; img-src 'self' data:; \
                   frame-ancestors 'none'; base-uri 'none'; form-action 'none'";

pub struct Response {
    pub status: u16,
    pub ctype: &'static str,
    pub body: Vec<u8>,
}

fn resp(status: u16, ctype: &'static str, body: impl Into<Vec<u8>>) -> Response {
    Response { status, ctype, body: body.into() }
}

/// Route one request (method, path, Host header) - kept apart from the socket so it can be tested.
pub fn route(method: &str, path: &str, host: &str, report_json: impl Fn() -> String) -> Response {
    let host = host.rsplit_once(':').map(|(h, _)| h).unwrap_or(host);
    if !["127.0.0.1", "localhost", "[::1]"].contains(&host) {
        return resp(400, "text/plain", "bad host"); // DNS rebinding: no other name may read this
    }
    if method != "GET" {
        return resp(405, "text/plain", "read-only");
    }
    match path.split('?').next().unwrap_or("") {
        "/" => resp(200, "text/html; charset=utf-8", PAGE),
        "/static/app.js" => resp(200, "text/javascript; charset=utf-8", JS),
        "/static/style.css" => resp(200, "text/css; charset=utf-8", CSS),
        "/static/hestia.svg" => resp(200, "image/svg+xml", ICON),
        "/api/report" => resp(200, "application/json", report_json()),
        _ => resp(404, "text/plain", "not found"),
    }
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        405 => "Method Not Allowed",
        _ => "Error",
    }
}

fn handle(stream: TcpStream, lim: &Limits, demo: bool) -> std::io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    let mut r = BufReader::new(stream.try_clone()?);
    let mut first = String::new();
    r.by_ref().take(8192).read_line(&mut first)?;
    let mut host = String::new();
    let mut total = first.len();
    loop {
        let mut line = String::new();
        let n = r.by_ref().take(8192).read_line(&mut line)?;
        total += n;
        if n == 0 || line == "\r\n" || line == "\n" || total > 16 * 1024 {
            break;
        }
        if let Some((k, v)) = line.split_once(':') {
            if k.eq_ignore_ascii_case("host") {
                host = v.trim().to_string();
            }
        }
    }
    let mut parts = first.split_whitespace();
    let (method, path) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
    let res = route(method, path, &host, || {
        let found = if demo {
            crate::demo::checks()
        } else {
            let sys = Sys::real();
            let mut found = checks::all(&sys, lim);
            crate::history::apply(&sys, &crate::cli::state_dir().join("history.jsonl"), report::now_secs(), &mut found);
            found
        };
        let rep = Report::new(found, report::now_secs());
        serde_json::to_string(&rep.json()).unwrap_or_default()
    });
    let mut w = stream;
    write!(
        w,
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nContent-Security-Policy: {CSP}\r\n\
         X-Content-Type-Options: nosniff\r\nReferrer-Policy: no-referrer\r\nX-Frame-Options: DENY\r\n\
         Cache-Control: no-store\r\nConnection: close\r\n\r\n",
        res.status,
        reason(res.status),
        res.ctype,
        res.body.len()
    )?;
    w.write_all(&res.body)?;
    w.flush()
}

pub fn serve(port: u16, lim: Limits, demo: bool) -> std::io::Result<()> {
    let listener = TcpListener::bind(("127.0.0.1", port))?;
    println!("Hestia on http://127.0.0.1:{}/", listener.local_addr()?.port());
    for stream in listener.incoming().flatten() {
        let _ = handle(stream, &lim, demo);
    }
    Ok(())
}
