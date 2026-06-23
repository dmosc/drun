use crate::config::Config;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::Arc;
use std::thread;

#[derive(Clone, Copy)]
pub(crate) struct EgressProxy {
    pub addr: SocketAddr,
}

impl EgressProxy {
    pub fn start(config: &Config) -> anyhow::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let addr = listener.local_addr()?;
        let config = Arc::new(config.clone());
        thread::spawn(move || {
            while let Ok((conn, _)) = listener.accept() {
                let config = Arc::clone(&config);
                thread::spawn(move || handle(conn, &config));
            }
        });
        Ok(Self { addr })
    }
}

fn handle(stream: TcpStream, config: &Config) {
    let Ok(writer) = stream.try_clone() else {
        return;
    };
    let mut reader = BufReader::new(stream);
    let mut first_line = String::new();
    if reader.read_line(&mut first_line).is_err() || first_line.is_empty() {
        return;
    }
    if first_line.starts_with("CONNECT ") {
        handle_connect(first_line, reader, writer, config);
    } else {
        handle_http(first_line, reader, writer, config);
    }
}

fn read_headers(reader: &mut BufReader<TcpStream>) -> Vec<String> {
    let mut headers = Vec::new();
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).is_err() {
            break;
        }
        let done = line == "\r\n" || line == "\n" || line.is_empty();
        headers.push(line);
        if done {
            break;
        }
    }
    headers
}

fn handle_connect(
    first_line: String,
    mut reader: BufReader<TcpStream>,
    mut writer: TcpStream,
    config: &Config,
) {
    let host_port = first_line.trim().split_whitespace().nth(1).unwrap_or("");
    let host = host_port
        .rsplit_once(':')
        .map(|(h, _)| h)
        .unwrap_or(host_port);
    read_headers(&mut reader);
    if !config.domain_allowed(host) {
        let _ = writer.write_all(b"HTTP/1.1 403 Forbidden\r\n\r\n");
        return;
    }
    let Ok(mut upstream) = TcpStream::connect(host_port) else {
        let _ = writer.write_all(b"HTTP/1.1 502 Bad Gateway\r\n\r\n");
        return;
    };
    let _ = writer.write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n");
    let buffered = reader.buffer().to_vec();
    if !buffered.is_empty() {
        let _ = upstream.write_all(&buffered);
    }
    relay(reader.into_inner(), upstream);
}

fn handle_http(
    first_line: String,
    mut reader: BufReader<TcpStream>,
    mut writer: TcpStream,
    config: &Config,
) {
    let mut parts = first_line.trim().splitn(3, ' ');
    let method = parts.next().unwrap_or("GET");
    let url = parts.next().unwrap_or("/");
    let version = parts.next().unwrap_or("HTTP/1.1");
    let no_scheme = url.trim_start_matches("http://");
    let (host_port, path) = no_scheme
        .split_once('/')
        .map(|(h, p)| (h, format!("/{p}")))
        .unwrap_or((no_scheme, "/".to_string()));
    let host = host_port
        .rsplit_once(':')
        .map(|(h, _)| h)
        .unwrap_or(host_port);
    let headers = read_headers(&mut reader);
    if !config.domain_allowed(host) {
        let _ = writer.write_all(b"HTTP/1.1 403 Forbidden\r\n\r\n");
        return;
    }
    let connect_addr = if host_port.contains(':') {
        host_port.to_string()
    } else {
        format!("{host_port}:80")
    };
    let Ok(mut upstream) = TcpStream::connect(connect_addr.as_str()) else {
        let _ = writer.write_all(b"HTTP/1.1 502 Bad Gateway\r\n\r\n");
        return;
    };
    let _ = upstream.write_all(format!("{method} {path} {version}\r\n").as_bytes());
    for h in headers {
        let _ = upstream.write_all(h.as_bytes());
    }
    let buffered = reader.buffer().to_vec();
    if !buffered.is_empty() {
        let _ = upstream.write_all(&buffered);
    }
    relay(reader.into_inner(), upstream);
}

fn relay(a: TcpStream, b: TcpStream) {
    let Ok(a_write) = a.try_clone() else { return };
    let Ok(b_write) = b.try_clone() else { return };
    let t = thread::spawn(move || copy_half(a, b_write));
    copy_half(b, a_write);
    t.join().ok();
}

fn copy_half(mut src: TcpStream, mut dst: TcpStream) {
    let mut buf = [0u8; 8192];
    loop {
        match src.read(&mut buf) {
            Ok(0) | Err(_) => {
                let _ = dst.shutdown(Shutdown::Write);
                break;
            }
            Ok(n) => {
                if dst.write_all(&buf[..n]).is_err() {
                    break;
                }
            }
        }
    }
}
