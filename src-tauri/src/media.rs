//! A tiny loopback HTTP server for media files.
//!
//! WebKitGTK plays `<video>` through GStreamer, which cannot read from the
//! custom `asset://` scheme the other platforms use, so on Linux the preview
//! loads recordings from here instead. It only listens on 127.0.0.1, only
//! serves media file types, and every URL carries a random per-launch token.

use std::fs::File;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;

fn content_type(path: &std::path::Path) -> Option<&'static str> {
    Some(
        match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
            "mp4" => "video/mp4",
            "wav" => "audio/wav",
            "png" => "image/png",
            "jpg" | "jpeg" => "image/jpeg",
            "webp" => "image/webp",
            "gif" => "image/gif",
            _ => return None,
        },
    )
}

fn percent_decode(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = std::str::from_utf8(bytes.get(i + 1..i + 3)?).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// Parse `bytes=a-b`, `bytes=a-` or `bytes=-n` against a file length.
fn parse_range(header: &str, len: u64) -> Option<(u64, u64)> {
    let (a, b) = header.trim().strip_prefix("bytes=")?.split_once('-')?;
    let (start, end) = match (a.is_empty(), b.is_empty()) {
        (false, false) => (
            a.parse().ok()?,
            b.parse::<u64>().ok()?.min(len.saturating_sub(1)),
        ),
        (false, true) => (a.parse().ok()?, len.saturating_sub(1)),
        (true, false) => (len.saturating_sub(b.parse().ok()?), len.saturating_sub(1)),
        (true, true) => return None,
    };
    (start <= end && start < len).then_some((start, end))
}

fn respond(stream: &mut TcpStream, status: &str, headers: &str) -> std::io::Result<()> {
    write!(
        stream,
        "HTTP/1.1 {status}\r\nAccess-Control-Allow-Origin: *\r\nConnection: close\r\n{headers}\r\n"
    )
}

fn handle(mut stream: TcpStream, token: &str) -> std::io::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut request = String::new();
    reader.read_line(&mut request)?;
    let mut range = None;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 || line.trim().is_empty() {
            break;
        }
        if let Some((name, value)) = line.split_once(':') {
            if name.eq_ignore_ascii_case("range") {
                range = Some(value.trim().to_owned());
            }
        }
    }

    let mut parts = request.split_whitespace();
    let (method, target) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
    if std::env::var_os("CINESNAP_LOG_MEDIA").is_some() {
        eprintln!(
            "media: {method} {} range={range:?}",
            target.rsplit("%2F").next().unwrap_or(target)
        );
    }
    let path = target
        .strip_prefix('/')
        .and_then(|t| t.strip_prefix(token))
        .and_then(|t| t.strip_prefix('/'))
        .and_then(|t| percent_decode(t.split('?').next().unwrap_or(t)))
        .map(PathBuf::from);
    let (Some(path), true) = (path, method == "GET" || method == "HEAD") else {
        return respond(&mut stream, "403 Forbidden", "Content-Length: 0\r\n");
    };
    let (Some(mime), true) = (content_type(&path), path.is_absolute()) else {
        return respond(&mut stream, "403 Forbidden", "Content-Length: 0\r\n");
    };
    let Ok(mut file) = File::open(&path) else {
        return respond(&mut stream, "404 Not Found", "Content-Length: 0\r\n");
    };
    let len = file.metadata()?.len();

    let (status, start, end) = match range.as_deref().map(|r| parse_range(r, len)) {
        Some(Some((s, e))) => ("206 Partial Content", s, e),
        Some(None) => {
            return respond(
                &mut stream,
                "416 Range Not Satisfiable",
                &format!("Content-Range: bytes */{len}\r\n"),
            );
        }
        None => ("200 OK", 0, len.saturating_sub(1)),
    };
    let count = if len == 0 { 0 } else { end - start + 1 };
    let mut headers =
        format!("Content-Type: {mime}\r\nAccept-Ranges: bytes\r\nContent-Length: {count}\r\n");
    if status.starts_with("206") {
        headers.push_str(&format!("Content-Range: bytes {start}-{end}/{len}\r\n"));
    }
    respond(&mut stream, status, &headers)?;
    if method == "GET" && count > 0 {
        file.seek(SeekFrom::Start(start))?;
        std::io::copy(&mut file.take(count), &mut stream)?;
    }
    stream.flush()
}

/// Start the server; returns the base URL (`http://127.0.0.1:port/token`).
pub fn serve() -> Option<String> {
    let listener = TcpListener::bind(("127.0.0.1", 0)).ok()?;
    let port = listener.local_addr().ok()?.port();
    // Unpredictable enough for a loopback-only capability URL, without
    // pulling in a random number crate.
    let token = {
        use std::hash::{BuildHasher, Hasher};
        let mut t = String::new();
        for _ in 0..2 {
            let mut h = std::collections::hash_map::RandomState::new().build_hasher();
            h.write_u64(std::process::id() as u64);
            t.push_str(&format!("{:016x}", h.finish()));
        }
        t
    };
    let base = format!("http://127.0.0.1:{port}/{token}");
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let token = token.clone();
            std::thread::spawn(move || {
                let _ = handle(stream, &token);
            });
        }
    });
    Some(base)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranges() {
        assert_eq!(parse_range("bytes=0-99", 1000), Some((0, 99)));
        assert_eq!(parse_range("bytes=500-", 1000), Some((500, 999)));
        assert_eq!(parse_range("bytes=-100", 1000), Some((900, 999)));
        assert_eq!(parse_range("bytes=0-5000", 1000), Some((0, 999)));
        assert_eq!(parse_range("bytes=1000-", 1000), None);
        assert_eq!(parse_range("lines=1-2", 1000), None);
    }

    #[test]
    fn serves_ranges_and_rejects_everything_else() {
        let dir = std::env::temp_dir().join(format!("cinesnap-media-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("clip one.mp4");
        std::fs::write(&file, b"0123456789").unwrap();
        std::fs::write(dir.join("secret.txt"), b"no").unwrap();
        let base = serve().unwrap();
        let (host, token) = base
            .strip_prefix("http://")
            .unwrap()
            .split_once('/')
            .unwrap();

        let get = |target: &str, extra: &str| -> String {
            let mut s = TcpStream::connect(host).unwrap();
            write!(s, "GET {target} HTTP/1.1\r\nHost: x\r\n{extra}\r\n").unwrap();
            let mut out = String::new();
            s.read_to_string(&mut out).unwrap();
            out
        };
        let url = |p: &std::path::Path| {
            format!("/{token}/{}", p.display().to_string().replace(' ', "%20"))
        };

        let full = get(&url(&file), "");
        assert!(
            full.starts_with("HTTP/1.1 200") && full.ends_with("0123456789"),
            "{full}"
        );
        assert!(full.contains("Content-Type: video/mp4"));
        let part = get(&url(&file), "Range: bytes=2-5\r\n");
        assert!(
            part.starts_with("HTTP/1.1 206") && part.ends_with("2345"),
            "{part}"
        );
        assert!(part.contains("Content-Range: bytes 2-5/10"));

        assert!(
            get(&url(&dir.join("secret.txt")), "").starts_with("HTTP/1.1 403"),
            "not a media type"
        );
        assert!(get(&format!("/wrong-token{}", file.display()), "").starts_with("HTTP/1.1 403"));
        assert!(get(&url(&dir.join("missing.mp4")), "").starts_with("HTTP/1.1 404"));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
