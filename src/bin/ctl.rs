use anyhow::{Context, Result};
use niri_shelf::{get_control_socket_path, get_thumbnail_path, ClientCommand, ClientResponse};
use std::io::Read;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream, UnixStream};
use tokio::time::{sleep, Duration, Instant};

const OVERVIEW_HTML: &str = r##"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<meta name="color-scheme" content="dark">
<title>Niri Shelf</title>
<style>
:root { color-scheme: dark; font: 16px/1.45 system-ui, sans-serif; background: #10131b; color: #eef1f8; }
* { box-sizing: border-box; }
body { margin: 0; min-height: 100vh; padding: clamp(24px, 5vw, 72px); }
header { display: flex; align-items: end; justify-content: space-between; gap: 24px; max-width: 1200px; margin: 0 auto 32px; }
h1 { margin: 0; font-size: clamp(28px, 4vw, 44px); letter-spacing: -.04em; }
.hint, #message { color: #9da8bd; }
.hint { margin: 8px 0 0; }
#message { min-height: 1.5em; text-align: right; }
main { max-width: 1200px; margin: auto; display: grid; grid-template-columns: repeat(3, minmax(0, 1fr)); gap: clamp(12px, 2vw, 24px); }
button { min-height: clamp(150px, 25vh, 250px); padding: 22px; border: 1px solid #30394b; border-radius: 18px; text-align: left; color: inherit; background: linear-gradient(145deg, #1d2432, #171c27); cursor: pointer; transition: border-color .15s, transform .15s, background .15s; }
button:hover:not(:disabled), button:focus-visible { outline: none; border-color: #9b8cff; background: linear-gradient(145deg, #292844, #1b2030); transform: translateY(-2px); }
button:disabled { cursor: default; opacity: .58; }
.number { display: inline-grid; place-items: center; width: 36px; height: 36px; border-radius: 10px; background: #30394b; color: #d9d3ff; font-weight: 700; }
.preview { display: block; width: 100%; aspect-ratio: 16 / 9; margin-top: 16px; border-radius: 10px; object-fit: cover; background: #10131b; }
.preview[hidden] { display: none; }
.title { display: block; margin-top: 20px; font-size: clamp(16px, 2vw, 22px); font-weight: 600; overflow-wrap: anywhere; }
.empty { color: #78849a; font-weight: 400; }
@media (max-width: 700px) { body { padding: 20px; } main { grid-template-columns: repeat(2, minmax(0, 1fr)); } header { align-items: start; flex-direction: column; } #message { text-align: left; } }
</style>
</head>
<body>
<header><div><h1>Niri Shelf</h1><p class="hint">Click a window to bring it to your main monitor.</p></div><div id="message" role="status">Connecting…</div></header>
<main id="slots" aria-label="Stored windows"></main>
<script>
const slotsElement = document.querySelector("#slots");
const messageElement = document.querySelector("#message");
const requestToken = "__REQUEST_TOKEN__";
let busy = false;
const cards = new Map();
function renderSlots(slots) {
  for (const slot of slots) {
    let card = cards.get(slot.slot);
    if (!card) {
      const button = document.createElement("button");
      button.type = "button";
      const number = document.createElement("span");
      number.className = "number";
      number.textContent = slot.slot;
      const preview = document.createElement("img");
      preview.className = "preview";
      preview.alt = "";
      preview.hidden = true;
      const title = document.createElement("span");
      button.append(number, preview, title);
      button.addEventListener("click", () => fetchWindow(slot.slot));
      card = { button, preview, title, windowId: null };
      cards.set(slot.slot, card);
      slotsElement.append(button);
    }

    card.button.disabled = slot.window_id === null;
    card.button.setAttribute("aria-label", slot.window_id === null
      ? `Slot ${slot.slot}, empty`
      : `Fetch ${slot.title || "untitled window"} from slot ${slot.slot}`);
    card.title.className = slot.window_id === null ? "title empty" : "title";
    card.title.textContent = slot.window_id === null ? "Empty slot" : (slot.title || "Untitled window");
    if (card.windowId !== slot.window_id) {
      card.windowId = slot.window_id;
      card.preview.hidden = slot.window_id === null;
      if (slot.window_id === null) {
        card.preview.removeAttribute("src");
      } else {
        card.preview.src = `/api/preview/${slot.window_id}?token=${requestToken}`;
      }
    }
  }
  messageElement.textContent = "Click a window to restore it";
}
async function fetchWindow(slot) {
  if (busy) return;
  busy = true;
  messageElement.textContent = `Restoring slot ${slot}…`;
  try {
    const response = await fetch(`/api/fetch/${slot}`, {
      method: "POST",
      headers: { "X-Niri-Shelf-Token": requestToken }
    });
    const result = await response.json();
    if (!response.ok) throw new Error(result.error || `Restore failed (${response.status})`);
    messageElement.textContent = `Restored slot ${slot}`;
  } catch (error) {
    messageElement.textContent = error.message;
  } finally {
    busy = false;
  }
}
const updates = new EventSource(`/events?token=${requestToken}`);
updates.onopen = () => { messageElement.textContent = "Overview connected; waiting for shelf…"; };
updates.addEventListener("connection", event => {
  const status = JSON.parse(event.data);
  messageElement.textContent = status.state === "connected"
    ? "Connected to shelf"
    : "Waiting for shelf daemon…";
});
updates.onmessage = event => {
  try {
    renderSlots(JSON.parse(event.data));
  } catch (error) {
    messageElement.textContent = `Invalid shelf update: ${error.message}`;
  }
};
updates.onerror = () => { messageElement.textContent = "Reconnecting to shelf…"; };
</script>
</body>
</html>"##;

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: niri-shelf-ctl <store|fetch|swap|status|overview> [slot 1-9]");
        std::process::exit(1);
    }

    if args[1] == "overview" {
        return overview().await;
    }

    let cmd = match args[1].as_str() {
        "store" => {
            let slot: usize = args.get(2).context("Missing slot number")?.parse()?;
            ClientCommand::Store { slot }
        }
        "fetch" => {
            let slot: usize = args.get(2).context("Missing slot number")?.parse()?;
            ClientCommand::Fetch { slot }
        }
        "swap" => {
            let slot: usize = args.get(2).context("Missing slot number")?.parse()?;
            ClientCommand::Swap { slot }
        }
        "status" => ClientCommand::Status,
        other => {
            eprintln!("Unknown command: {}", other);
            std::process::exit(1);
        }
    };

    match send_command(cmd).await? {
        ClientResponse::Success => Ok(()),
        ClientResponse::Status(slots) => {
            for slot in slots {
                match slot.title {
                    Some(title) => println!("[{}] {}", slot.slot, title),
                    None => println!("[{}] -", slot.slot),
                }
            }
            Ok(())
        }
        ClientResponse::Error { message } => anyhow::bail!("{message}"),
    }
}

async fn write_connection_status(stream: &mut TcpStream, state: &str) -> Result<()> {
    let data = serde_json::json!({ "state": state }).to_string();
    stream.write_all(b"event: connection\ndata: ").await?;
    stream.write_all(data.as_bytes()).await?;
    stream.write_all(b"\n\n").await?;
    stream.flush().await?;
    Ok(())
}

async fn send_command(cmd: ClientCommand) -> Result<ClientResponse> {
    let socket_path = get_control_socket_path();
    let mut stream = UnixStream::connect(&socket_path)
        .await
        .context("Failed to connect to niri-shelf-daemon. Is it running?")?;

    let mut payload = serde_json::to_string(&cmd)?;
    payload.push('\n');
    stream.write_all(payload.as_bytes()).await?;

    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line).await?;

    Ok(serde_json::from_str(&line)?)
}

async fn overview() -> Result<()> {
    wait_for_daemon().await?;
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let url = format!("http://{address}/");
    let token = random_token()?;
    let browser = std::env::var("NIRI_SHELF_BROWSER").unwrap_or_else(|_| "firefox".to_string());
    std::process::Command::new(&browser)
        .args(["--new-window", "--kiosk", &url])
        .spawn()
        .with_context(|| format!("failed to launch browser {browser:?}"))?;

    eprintln!("Niri Shelf graphical overview running at {url}");
    loop {
        let (stream, _) = listener.accept().await?;
        let token = token.clone();
        tokio::spawn(async move {
            if let Err(error) = handle_http(stream, &token).await {
                eprintln!("Overview request failed: {error}");
            }
        });
    }
}

async fn wait_for_daemon() -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut next_report = Instant::now();
    loop {
        let status = match send_command(ClientCommand::Status).await {
            Ok(ClientResponse::Status(_)) => return Ok(()),
            Ok(ClientResponse::Error { message }) => message,
            Ok(ClientResponse::Success) => "unexpected status response".to_string(),
            Err(error) => error.to_string(),
        };
        if Instant::now() >= next_report {
            eprintln!("Waiting for niri-shelf-daemon: {status}");
            next_report = Instant::now() + Duration::from_secs(5);
        }

        if Instant::now() >= deadline {
            anyhow::bail!("niri-shelf-daemon did not become ready within 30 seconds");
        }
        sleep(Duration::from_millis(250)).await;
    }
}

async fn handle_http(mut stream: TcpStream, token: &str) -> Result<()> {
    let mut request = Vec::new();
    let mut chunk = [0; 1024];
    while request.len() < 8192 {
        let read = stream.read(&mut chunk).await?;
        if read == 0 {
            break;
        }
        request.extend_from_slice(&chunk[..read]);
        if request.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
    }

    let request = String::from_utf8_lossy(&request);
    let mut parts = request
        .lines()
        .next()
        .unwrap_or_default()
        .split_whitespace();
    let method = parts.next().unwrap_or_default();
    let target = parts.next().unwrap_or_default();
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let request_token = request.lines().skip(1).find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.eq_ignore_ascii_case("x-niri-shelf-token")
            .then(|| value.trim())
    });
    let token_is_valid = request_token == Some(token);
    if method == "GET" && path == "/events" {
        let query_token = query
            .split('&')
            .find_map(|parameter| parameter.strip_prefix("token="));
        if query_token != Some(token) {
            return write_http_response(
                stream,
                "403 Forbidden",
                "application/json",
                json_error("missing or invalid request token").as_bytes(),
            )
            .await;
        }
        return stream_events(stream).await;
    }

    if method == "GET" && path.starts_with("/api/preview/") {
        let query_token = query
            .split('&')
            .find_map(|parameter| parameter.strip_prefix("token="));
        if query_token != Some(token) {
            return write_http_response(
                stream,
                "403 Forbidden",
                "application/json",
                json_error("missing or invalid request token").as_bytes(),
            )
            .await;
        }

        let image_response = match path["/api/preview/".len()..].parse::<u64>() {
            Ok(window_id) => match get_thumbnail_path(window_id) {
                Ok(path) => match tokio::fs::read(path).await {
                    Ok(image) => ("200 OK", "image/png", image),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => (
                        "404 Not Found",
                        "application/json",
                        json_error("preview not found").into_bytes(),
                    ),
                    Err(error) => (
                        "500 Internal Server Error",
                        "application/json",
                        json_error(&error.to_string()).into_bytes(),
                    ),
                },
                Err(error) => (
                    "500 Internal Server Error",
                    "application/json",
                    json_error(&error).into_bytes(),
                ),
            },
            Err(_) => (
                "400 Bad Request",
                "application/json",
                json_error("invalid window id").into_bytes(),
            ),
        };
        return write_http_response(
            stream,
            image_response.0,
            image_response.1,
            &image_response.2,
        )
        .await;
    }

    let (status, content_type, body) = match (method, path) {
        ("GET", "/") => (
            "200 OK",
            "text/html; charset=utf-8",
            OVERVIEW_HTML.replace("__REQUEST_TOKEN__", token),
        ),
        ("GET", "/api/status") => match send_command(ClientCommand::Status).await {
            Ok(ClientResponse::Status(slots)) => {
                ("200 OK", "application/json", serde_json::to_string(&slots)?)
            }
            Ok(ClientResponse::Error { message }) => (
                "503 Service Unavailable",
                "application/json",
                json_error(&message),
            ),
            Ok(ClientResponse::Success) => (
                "500 Internal Server Error",
                "application/json",
                json_error("unexpected daemon response"),
            ),
            Err(error) => (
                "503 Service Unavailable",
                "application/json",
                json_error(&error.to_string()),
            ),
        },
        ("POST", path) if path.starts_with("/api/fetch/") && !token_is_valid => (
            "403 Forbidden",
            "application/json",
            json_error("missing or invalid request token"),
        ),
        ("POST", path) if path.starts_with("/api/fetch/") => {
            let result = path["/api/fetch/".len()..]
                .parse::<usize>()
                .ok()
                .filter(|slot| (1..=9).contains(slot))
                .context("invalid slot number");
            match result {
                Ok(slot) => match send_command(ClientCommand::Fetch { slot }).await {
                    Ok(ClientResponse::Success) => {
                        ("200 OK", "application/json", r#"{"ok":true}"#.to_string())
                    }
                    Ok(ClientResponse::Error { message }) => {
                        ("409 Conflict", "application/json", json_error(&message))
                    }
                    Ok(ClientResponse::Status(_)) => (
                        "500 Internal Server Error",
                        "application/json",
                        json_error("unexpected daemon response"),
                    ),
                    Err(error) => (
                        "503 Service Unavailable",
                        "application/json",
                        json_error(&error.to_string()),
                    ),
                },
                Err(error) => (
                    "400 Bad Request",
                    "application/json",
                    json_error(&error.to_string()),
                ),
            }
        }
        _ => (
            "404 Not Found",
            "text/plain; charset=utf-8",
            "Not found".to_string(),
        ),
    };

    write_http_response(stream, status, content_type, body.as_bytes()).await
}

async fn stream_events(mut stream: TcpStream) -> Result<()> {
    stream_events_at(&mut stream, get_control_socket_path()).await
}

async fn stream_events_at(stream: &mut TcpStream, socket_path: std::path::PathBuf) -> Result<()> {
    let headers = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: keep-alive\r\nX-Content-Type-Options: nosniff\r\n\r\n";
    stream.write_all(headers.as_bytes()).await?;
    loop {
        let mut daemon = match UnixStream::connect(&socket_path).await {
            Ok(daemon) => daemon,
            Err(error) => {
                write_connection_status(stream, "waiting").await?;
                eprintln!("Shelf update stream waiting for daemon: {error}");
                sleep(Duration::from_secs(1)).await;
                continue;
            }
        };
        let mut command = serde_json::to_string(&ClientCommand::Subscribe)?;
        command.push('\n');
        if let Err(error) = daemon.write_all(command.as_bytes()).await {
            eprintln!("Shelf update stream could not subscribe to daemon: {error}");
            write_connection_status(stream, "waiting").await?;
            sleep(Duration::from_secs(1)).await;
            continue;
        }

        write_connection_status(stream, "connected").await?;
        let mut daemon = BufReader::new(daemon);
        let mut line = String::new();
        loop {
            line.clear();
            match daemon.read_line(&mut line).await {
                Ok(0) => break,
                Ok(_) => {}
                Err(error) => {
                    eprintln!("Shelf update stream disconnected from daemon: {error}");
                    break;
                }
            }
            let event_data = match serde_json::from_str::<ClientResponse>(&line) {
                Ok(ClientResponse::Status(slots)) => serde_json::to_string(&slots)?,
                Ok(ClientResponse::Error { message }) => {
                    serde_json::json!({ "error": message }).to_string()
                }
                Ok(ClientResponse::Success) => continue,
                Err(error) => {
                    eprintln!("Invalid shelf update from daemon: {error}");
                    break;
                }
            };
            if stream.write_all(b"data: ").await.is_err()
                || stream.write_all(event_data.as_bytes()).await.is_err()
                || stream.write_all(b"\n\n").await.is_err()
                || stream.flush().await.is_err()
            {
                return Ok(());
            }
        }
        stream
            .write_all(b": reconnecting to shelf daemon\n\n")
            .await?;
        stream.flush().await?;
        sleep(Duration::from_secs(1)).await;
    }
}

async fn write_http_response(
    mut stream: TcpStream,
    status: &str,
    content_type: &str,
    body: &[u8],
) -> Result<()> {
    let headers = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\nX-Content-Type-Options: nosniff\r\n\r\n",
        body.len()
    );
    stream.write_all(headers.as_bytes()).await?;
    stream.write_all(body).await?;
    Ok(())
}

fn json_error(message: &str) -> String {
    serde_json::json!({ "error": message }).to_string()
}

fn random_token() -> Result<String> {
    let mut bytes = [0; 32];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::UnixListener;

    async fn request_overview(request: &str, token: &str) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server_token = token.to_string();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            handle_http(stream, &server_token).await.unwrap();
        });

        let mut client = TcpStream::connect(address).await.unwrap();
        client.write_all(request.as_bytes()).await.unwrap();
        let mut response = String::new();
        client.read_to_string(&mut response).await.unwrap();
        server.await.unwrap();
        response
    }

    #[tokio::test]
    async fn overview_page_embeds_its_request_token() {
        let response =
            request_overview("GET / HTTP/1.1\r\nHost: localhost\r\n\r\n", "test-token").await;

        assert!(response.starts_with("HTTP/1.1 200 OK"));
        assert!(response.contains("const requestToken = \"test-token\";"));
        assert!(response.contains("new EventSource(`/events?token=${requestToken}`)"));
        assert!(response.contains("/api/preview/${slot.window_id}?token=${requestToken}"));
        assert!(!response.contains("setInterval"));
        assert!(!response.contains("replaceChildren"));
        assert!(!response.contains("__REQUEST_TOKEN__"));
    }

    #[tokio::test]
    async fn fetch_requires_the_request_token() {
        let response = request_overview(
            "POST /api/fetch/1 HTTP/1.1\r\nHost: localhost\r\n\r\n",
            "test-token",
        )
        .await;

        assert!(response.starts_with("HTTP/1.1 403 Forbidden"));
    }

    #[tokio::test]
    async fn preview_requires_the_request_token() {
        let response = request_overview(
            "GET /api/preview/12 HTTP/1.1\r\nHost: localhost\r\n\r\n",
            "test-token",
        )
        .await;

        assert!(response.starts_with("HTTP/1.1 403 Forbidden"));
    }

    #[tokio::test]
    async fn event_stream_requires_the_request_token() {
        let response = request_overview(
            "GET /events?token=wrong HTTP/1.1\r\nHost: localhost\r\n\r\n",
            "test-token",
        )
        .await;

        assert!(response.starts_with("HTTP/1.1 403 Forbidden"));
    }

    #[tokio::test]
    async fn event_stream_recovers_when_daemon_starts_late() {
        let socket_path = std::env::temp_dir().join(format!(
            "niri-shelf-test-{}-{}.sock",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server_path = socket_path.clone();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            stream_events_at(&mut stream, server_path).await.unwrap();
        });

        let mut client = TcpStream::connect(address).await.unwrap();
        let mut reader = BufReader::new(&mut client);
        let mut line = String::new();
        loop {
            line.clear();
            reader.read_line(&mut line).await.unwrap();
            if line == "\r\n" {
                break;
            }
        }
        line.clear();
        reader.read_line(&mut line).await.unwrap();
        assert_eq!(line, "event: connection\n");
        line.clear();
        reader.read_line(&mut line).await.unwrap();
        assert_eq!(line, "data: {\"state\":\"waiting\"}\n");
        line.clear();
        reader.read_line(&mut line).await.unwrap();
        assert_eq!(line, "\n");

        let daemon = UnixListener::bind(&socket_path).unwrap();
        let (mut daemon_stream, _) = daemon.accept().await.unwrap();
        let mut daemon_reader = BufReader::new(&mut daemon_stream);
        line.clear();
        daemon_reader.read_line(&mut line).await.unwrap();
        assert!(matches!(
            serde_json::from_str::<ClientCommand>(&line).unwrap(),
            ClientCommand::Subscribe
        ));

        let status = ClientResponse::Status(vec![niri_shelf::SlotInfo {
            slot: 1,
            window_id: Some(23),
            title: Some("Ready".to_string()),
        }]);
        let mut status_line = serde_json::to_string(&status).unwrap();
        status_line.push('\n');
        daemon_reader
            .get_mut()
            .write_all(status_line.as_bytes())
            .await
            .unwrap();

        line.clear();
        reader.read_line(&mut line).await.unwrap();
        assert_eq!(line, "event: connection\n");
        line.clear();
        reader.read_line(&mut line).await.unwrap();
        assert_eq!(line, "data: {\"state\":\"connected\"}\n");
        line.clear();
        reader.read_line(&mut line).await.unwrap();
        assert_eq!(line, "\n");
        line.clear();
        reader.read_line(&mut line).await.unwrap();
        assert!(line.starts_with("data: "));
        assert!(line.contains("\"window_id\":23"));

        server.abort();
        drop(daemon_reader);
        drop(daemon);
        let _ = tokio::fs::remove_file(socket_path).await;
    }
}
