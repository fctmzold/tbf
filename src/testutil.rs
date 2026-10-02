//! Loopback HTTP servers for tests.
//!
//! Tests never touch the real network; these helpers plus
//! `Prober::with_hosts` are the project's mocking strategy.
use std::io::{Read, Write};
use std::net::TcpListener;

/// Serve canned HTTP statuses from a background thread, one per connection.
///
/// # Arguments
///
/// * `statuses` - Status per accepted connection, in order.
pub(crate) fn serve(statuses: Vec<u16>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
    let addr = listener.local_addr().expect("read local addr");
    std::thread::spawn(move || {
        for status in statuses {
            let Ok((mut stream, _)) = listener.accept() else {
                break;
            };
            let mut request = [0_u8; 1024];
            let _ = stream.read(&mut request);
            let reason = match status {
                200 => "OK",
                404 => "Not Found",
                429 => "Too Many Requests",
                500 => "Internal Server Error",
                _ => "Error",
            };
            let _ = write!(
                stream,
                "HTTP/1.1 {status} {reason}\r\ncontent-length: 0\r\nconnection: close\r\n\r\n"
            );
        }
    });
    format!("http://{addr}")
}

/// Serve 200 for playlist paths of `target`, 404 for everything else.
///
/// Paths carry `{hash}_{user}_{id}_{timestamp}`, so matching `_{target}/`
/// selects exactly one timestamp.
///
/// # Arguments
///
/// * `target` - Unix epoch seconds that hit.
pub(crate) fn serve_gated(target: i64) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
    let addr = listener.local_addr().expect("read local addr");
    let marker = format!("_{target}/");
    std::thread::spawn(move || {
        for _ in 0..400 {
            let Ok((mut stream, _)) = listener.accept() else {
                break;
            };
            let marker = marker.clone();
            std::thread::spawn(move || {
                let mut request = [0_u8; 2048];
                let _ = stream.read(&mut request);
                let head = String::from_utf8_lossy(&request);
                let path = head.split_whitespace().nth(1).unwrap_or_default();
                let (status, reason) = if path.contains(&marker) {
                    ("200", "OK")
                } else {
                    ("404", "Not Found")
                };
                let _ = write!(
                    stream,
                    "HTTP/1.1 {status} {reason}\r\ncontent-length: 0\r\nconnection: close\r\n\r\n"
                );
            });
        }
    });
    format!("http://{addr}")
}

/// Serve a playlist plus one muted segment from a background thread.
///
/// `GET /*.m3u8` answers a one-segment unmuted playlist; `HEAD` of the
/// muted rewrite answers `muted_status`; everything else is 404.
///
/// # Arguments
///
/// * `muted_status` - Status for the muted-segment probe (200 hits,
///   404 misses, 500 fails after retries).
pub(crate) fn serve_fix_playlist(muted_status: u16) -> String {
    const PLAYLIST: &str = "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:6\n#EXTINF:6.0,\n1-unmuted.ts\n#EXT-X-ENDLIST\n";
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
    let addr = listener.local_addr().expect("read local addr");
    std::thread::spawn(move || {
        // One playlist fetch plus up to three muted-segment attempts.
        for _ in 0..8 {
            let Ok((mut stream, _)) = listener.accept() else {
                break;
            };
            let mut request = [0_u8; 1024];
            let _ = stream.read(&mut request);
            let head = String::from_utf8_lossy(&request);
            let mut parts = head.split_whitespace();
            let (method, path) = (
                parts.next().unwrap_or_default(),
                parts.next().unwrap_or_default(),
            );
            let muted = match muted_status {
                200 => "200 OK",
                404 => "404 Not Found",
                _ => "500 Internal Server Error",
            };
            let (status, body) = if method == "GET" && path.ends_with(".m3u8") {
                ("200 OK", PLAYLIST)
            } else if path.ends_with("1-muted.ts") {
                (muted, "")
            } else {
                ("404 Not Found", "")
            };
            let _ = write!(
                stream,
                "HTTP/1.1 {status}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
        }
    });
    format!("http://{addr}")
}
