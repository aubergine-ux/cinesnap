//! Uploading a finished video to a Cinesnap share server.

use std::io::Read;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::export::Cancelled;

/// What the server answers after an upload.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Shared {
    pub id: String,
    pub url: String,
    pub embed_url: String,
    pub embed_html: String,
    pub media_url: String,
}

/// Counts what has been read and stops when the upload is cancelled.
struct Watched<'a, R> {
    inner: R,
    done: u64,
    total: u64,
    progress: &'a mut dyn FnMut(f64),
    cancel: &'a AtomicBool,
}

impl<R: Read> Read for Watched<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.cancel.load(Ordering::Relaxed) {
            return Err(std::io::Error::other("cancelled"));
        }
        let n = self.inner.read(buf)?;
        self.done += n as u64;
        (self.progress)((self.done as f64 / self.total.max(1) as f64).min(1.0));
        Ok(n)
    }
}

/// Percent-encode everything outside the unreserved set.
pub fn encode_title(title: &str) -> String {
    title
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

fn content_type(file: &Path) -> Result<&'static str> {
    match file.extension().and_then(|e| e.to_str()) {
        Some(e) if e.eq_ignore_ascii_case("mp4") => Ok("video/mp4"),
        Some(e) if e.eq_ignore_ascii_case("gif") => Ok("image/gif"),
        _ => bail!("{}: only .mp4 and .gif files can be shared", file.display()),
    }
}

/// The upload address for whatever the user typed as the server.
pub fn endpoint(server: &str) -> Result<String> {
    let server = server.trim().trim_end_matches('/');
    anyhow::ensure!(
        server.starts_with("https://") || server.starts_with("http://"),
        "the share server address must start with http:// or https://"
    );
    Ok(format!("{server}/api/videos"))
}

/// Upload `file` and return its links. `progress` receives 0..1.
pub fn upload(
    server: &str,
    token: &str,
    file: &Path,
    title: &str,
    progress: &mut dyn FnMut(f64),
    cancel: &AtomicBool,
) -> Result<Shared> {
    let url = endpoint(server)?;
    let kind = content_type(file)?;
    let handle =
        std::fs::File::open(file).with_context(|| format!("opening {}", file.display()))?;
    let total = handle.metadata()?.len();
    anyhow::ensure!(total > 0, "{} is empty", file.display());
    let mut body = Watched {
        inner: handle,
        done: 0,
        total,
        progress,
        cancel,
    };
    let result = ureq::post(&url)
        .config()
        .http_status_as_error(false)
        .build()
        .header("Authorization", &format!("Bearer {}", token.trim()))
        .header("Content-Type", kind)
        .header("Content-Length", &total.to_string())
        .header("X-Title", &encode_title(title))
        .send(ureq::SendBody::from_reader(&mut body));
    if cancel.load(Ordering::Relaxed) {
        return Err(Cancelled.into());
    }
    let mut response = result.with_context(|| format!("could not reach {server}"))?;
    let status = response.status().as_u16();
    let text = response.body_mut().read_to_string().unwrap_or_default();
    if status != 201 {
        let reason = serde_json::from_str::<serde_json::Value>(&text)
            .ok()
            .and_then(|v| v["error"].as_str().map(str::to_owned))
            .unwrap_or_else(|| text.chars().take(200).collect());
        match status {
            401 | 403 => bail!("the share server rejected the token"),
            413 => bail!("the file is larger than the share server accepts"),
            _ => bail!("the share server answered {status}: {reason}"),
        }
    }
    serde_json::from_str(&text).context("the share server sent an unexpected answer")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;

    #[test]
    fn titles_and_addresses() {
        assert_eq!(encode_title("My demo ✓.mp4"), "My%20demo%20%E2%9C%93.mp4");
        assert_eq!(
            endpoint(" https://share.example.com/ ").unwrap(),
            "https://share.example.com/api/videos"
        );
        assert!(endpoint("share.example.com").is_err());
        assert!(content_type(Path::new("a.MP4")).is_ok());
        assert!(content_type(Path::new("a.webm")).is_err());
    }

    /// A one-shot HTTP server that records the request and answers `reply`.
    fn serve(
        status: &'static str,
        reply: &'static str,
    ) -> (String, std::thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = format!("http://{}", listener.local_addr().unwrap());
        let handle = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut head = String::new();
            let mut length = 0;
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if line == "\r\n" || line.is_empty() {
                    break;
                }
                if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    length = v.trim().parse().unwrap();
                }
                head += &line;
            }
            let mut body = vec![0u8; length];
            reader.read_exact(&mut body).unwrap();
            let mut stream = stream;
            write!(
                stream,
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}",
                reply.len()
            )
            .unwrap();
            format!("{head}\n{}", body.len())
        });
        (addr, handle)
    }

    #[test]
    fn uploads_and_reports_errors() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("demo.mp4");
        std::fs::write(&file, vec![7u8; 50_000]).unwrap();
        let cancel = AtomicBool::new(false);

        let (addr, server) = serve(
            "201 Created",
            r#"{"id":"abc","url":"u","embedUrl":"e","embedHtml":"<iframe>","mediaUrl":"m","views":0}"#,
        );
        let mut last = 0.0;
        let shared = upload(
            &addr,
            " secret ",
            &file,
            "My demo",
            &mut |f| last = f,
            &cancel,
        )
        .unwrap();
        assert_eq!((shared.id.as_str(), shared.url.as_str()), ("abc", "u"));
        assert_eq!(last, 1.0);
        let seen = server.join().unwrap().to_ascii_lowercase();
        assert!(seen.contains("authorization: bearer secret"), "{seen}");
        assert!(seen.contains("content-type: video/mp4"));
        assert!(seen.contains("x-title: my%20demo"));
        assert!(seen.ends_with("\n50000"));

        let (addr, server) = serve("401 Unauthorized", r#"{"error":"bad token"}"#);
        let err = upload(&addr, "x", &file, "t", &mut |_| {}, &cancel).unwrap_err();
        assert!(err.to_string().contains("rejected the token"), "{err:#}");
        server.join().unwrap();

        let (addr, server) = serve("500 Oops", r#"{"error":"disk full"}"#);
        let err = upload(&addr, "x", &file, "t", &mut |_| {}, &cancel).unwrap_err();
        assert!(err.to_string().contains("disk full"), "{err:#}");
        server.join().unwrap();

        // Nothing listening, an empty file, a file of the wrong kind.
        assert!(upload("http://127.0.0.1:1", "x", &file, "t", &mut |_| {}, &cancel).is_err());
        let empty = dir.path().join("empty.gif");
        std::fs::write(&empty, b"").unwrap();
        assert!(upload(&addr, "x", &empty, "t", &mut |_| {}, &cancel).is_err());
        assert!(upload(
            &addr,
            "x",
            &dir.path().join("a.txt"),
            "t",
            &mut |_| {},
            &cancel
        )
        .is_err());
    }
}
