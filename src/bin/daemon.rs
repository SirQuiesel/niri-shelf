use anyhow::{Context, Result};
use niri_ipc::{Action, Event, Reply, Request, Response, Window, WorkspaceReferenceArg};
use niri_shelf::{
    get_control_socket_path, get_thumbnail_path, ClientCommand, ClientResponse, SlotInfo,
};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::Mutex;
use tokio::time::{sleep, Duration, Instant};

const MAX_SLOTS: usize = 9;
const SHELF_WORKSPACE_NAME: &str = "shelf";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ShelvedWindow {
    window_id: u64,
    return_workspace_id: u64,
}

struct AppState {
    /// Each slot remembers the window and the workspace it came from.
    slots: [Option<ShelvedWindow>; MAX_SLOTS],
    /// Cache of all currently open windows for fast title lookup
    windows: HashMap<u64, Window>,
}

impl AppState {
    fn new() -> Self {
        Self {
            slots: [None; MAX_SLOTS],
            windows: HashMap::new(),
        }
    }

    fn remove_window(&mut self, id: u64) {
        self.windows.remove(&id);
        self.clear_slot_window(id);
    }

    fn clear_slot_window(&mut self, id: u64) {
        for slot in &mut self.slots {
            if slot.is_some_and(|entry| entry.window_id == id) {
                *slot = None;
            }
        }
    }

    fn assign_slot(&mut self, slot: usize, entry: ShelvedWindow) {
        self.clear_slot_window(entry.window_id);
        self.slots[slot] = Some(entry);
    }
}

/// Helper to send a single request to niri's IPC socket and read the response
async fn send_niri_request(req: Request) -> Result<Response> {
    let socket_path = std::env::var("NIRI_SOCKET").context("NIRI_SOCKET not set")?;
    let mut stream = UnixStream::connect(socket_path).await?;

    let mut payload = serde_json::to_string(&req)?;
    payload.push('\n');
    stream.write_all(payload.as_bytes()).await?;

    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line).await?;

    let reply: Reply = serde_json::from_str(&line)?;
    reply.map_err(|e| anyhow::anyhow!("Niri IPC error: {}", e))
}

/// Background task listening to niri's event stream to keep window state in sync
async fn niri_event_listener(state: Arc<Mutex<AppState>>) -> Result<()> {
    let socket_path = std::env::var("NIRI_SOCKET").context("NIRI_SOCKET not set")?;
    let mut stream = UnixStream::connect(socket_path).await?;

    let mut payload = serde_json::to_string(&Request::EventStream)?;
    payload.push('\n');
    stream.write_all(payload.as_bytes()).await?;

    let mut reader = BufReader::new(stream);
    let mut line = String::new();

    // Read initial reply acknowledging the EventStream
    reader.read_line(&mut line).await?;
    line.clear();

    while reader.read_line(&mut line).await? > 0 {
        if let Ok(event) = serde_json::from_str::<Event>(&line) {
            let mut st = state.lock().await;
            match event {
                Event::WindowsChanged { windows } => {
                    st.windows.clear();
                    for w in windows {
                        st.windows.insert(w.id, w);
                    }
                }
                Event::WindowOpenedOrChanged { window } => {
                    st.windows.insert(window.id, window);
                }
                Event::WindowClosed { id } => {
                    st.remove_window(id);
                }
                _ => {}
            }
        }
        line.clear();
    }

    Ok(())
}

fn slot_index(slot: usize) -> Result<usize> {
    anyhow::ensure!(
        (1..=MAX_SLOTS).contains(&slot),
        "slot must be between 1 and {MAX_SLOTS}"
    );
    Ok(slot - 1)
}

async fn move_window(window_id: u64, reference: WorkspaceReferenceArg, focus: bool) -> Result<()> {
    match send_niri_request(Request::Action(Action::MoveWindowToWorkspace {
        window_id: Some(window_id),
        reference,
        focus,
    }))
    .await?
    {
        Response::Handled => Ok(()),
        _ => anyhow::bail!("Niri returned an unexpected response while moving window {window_id}"),
    }
}

async fn capture_window(window_id: u64) -> Result<PathBuf> {
    let path = get_thumbnail_path(window_id).map_err(anyhow::Error::msg)?;
    anyhow::ensure!(path.is_absolute(), "thumbnail cache path must be absolute");
    let parent = path.parent().context("thumbnail path has no parent")?;
    tokio::fs::create_dir_all(parent).await?;
    if tokio::fs::try_exists(&path).await? {
        tokio::fs::remove_file(&path).await?;
    }

    match send_niri_request(Request::Action(Action::ScreenshotWindow {
        id: Some(window_id),
        write_to_disk: true,
        path: Some(path.to_string_lossy().into_owned()),
    }))
    .await?
    {
        Response::Handled => {}
        _ => {
            anyhow::bail!("Niri returned an unexpected response while capturing window {window_id}")
        }
    }

    let deadline = Instant::now() + Duration::from_secs(10);
    let mut previous_size = None;
    let mut stable_checks = 0;
    loop {
        if let Ok(metadata) = tokio::fs::metadata(&path).await {
            if metadata.is_file() && metadata.len() > 0 {
                if previous_size == Some(metadata.len()) {
                    stable_checks += 1;
                    if stable_checks >= 2 {
                        return Ok(path);
                    }
                } else {
                    previous_size = Some(metadata.len());
                    stable_checks = 0;
                }
            }
        }
        if Instant::now() >= deadline {
            anyhow::bail!(
                "timed out waiting for Niri to save window preview: {}",
                path.display()
            );
        }
        sleep(Duration::from_millis(50)).await;
    }
}

async fn focused_window() -> Result<Window> {
    match send_niri_request(Request::FocusedWindow).await? {
        Response::FocusedWindow(Some(window)) => Ok(window),
        Response::FocusedWindow(None) => anyhow::bail!("no focused window"),
        _ => anyhow::bail!("Niri returned an unexpected response for focused-window request"),
    }
}

async fn process_command(
    cmd: ClientCommand,
    state: Arc<Mutex<AppState>>,
) -> Result<ClientResponse> {
    match cmd {
        ClientCommand::Store { slot } => {
            let index = slot_index(slot)?;
            let window = focused_window().await?;
            let return_workspace_id = window
                .workspace_id
                .context("focused window is not on a workspace")?;
            {
                let st = state.lock().await;
                if st.slots[index].is_some_and(|entry| entry.window_id != window.id) {
                    anyhow::bail!("slot {slot} is occupied; fetch or swap it first");
                }
            }

            capture_window(window.id).await?;
            move_window(
                window.id,
                WorkspaceReferenceArg::Name(SHELF_WORKSPACE_NAME.to_string()),
                false,
            )
            .await?;

            state.lock().await.assign_slot(
                index,
                ShelvedWindow {
                    window_id: window.id,
                    return_workspace_id,
                },
            );
            Ok(ClientResponse::Success)
        }

        ClientCommand::Fetch { slot } => {
            let index = slot_index(slot)?;
            let entry =
                state.lock().await.slots[index].with_context(|| format!("slot {slot} is empty"))?;

            move_window(
                entry.window_id,
                WorkspaceReferenceArg::Id(entry.return_workspace_id),
                true,
            )
            .await?;

            match send_niri_request(Request::Action(Action::FocusWindow {
                id: entry.window_id,
            }))
            .await?
            {
                Response::Handled => {
                    state.lock().await.clear_slot_window(entry.window_id);
                    Ok(ClientResponse::Success)
                }
                _ => anyhow::bail!(
                    "Niri returned an unexpected response while focusing window {}",
                    entry.window_id
                ),
            }
        }

        ClientCommand::Swap { slot } => {
            let index = slot_index(slot)?;
            let window = focused_window().await?;
            let previous_window = state.lock().await.slots[index];

            if previous_window.is_some_and(|entry| entry.window_id == window.id) {
                return Ok(ClientResponse::Success);
            }

            let return_workspace_id = window
                .workspace_id
                .context("focused window is not on a workspace")?;

            capture_window(window.id).await?;

            if let Some(previous_entry) = previous_window {
                move_window(
                    previous_entry.window_id,
                    WorkspaceReferenceArg::Id(previous_entry.return_workspace_id),
                    true,
                )
                .await?;
            }

            if let Err(move_error) = move_window(
                window.id,
                WorkspaceReferenceArg::Name(SHELF_WORKSPACE_NAME.to_string()),
                false,
            )
            .await
            {
                if let Some(previous_entry) = previous_window {
                    if let Err(rollback_error) = move_window(
                        previous_entry.window_id,
                        WorkspaceReferenceArg::Name(SHELF_WORKSPACE_NAME.to_string()),
                        false,
                    )
                    .await
                    {
                        state
                            .lock()
                            .await
                            .clear_slot_window(previous_entry.window_id);
                        anyhow::bail!(
                            "failed to store focused window ({move_error}); rollback also failed ({rollback_error}); slot {slot} was cleared"
                        );
                    }
                    return Err(move_error);
                }
                return Err(move_error);
            }

            let mut st = state.lock().await;
            if let Some(previous_entry) = previous_window {
                st.clear_slot_window(previous_entry.window_id);
            }
            st.assign_slot(
                index,
                ShelvedWindow {
                    window_id: window.id,
                    return_workspace_id,
                },
            );
            drop(st);

            if let Some(previous_entry) = previous_window {
                match send_niri_request(Request::Action(Action::FocusWindow {
                    id: previous_entry.window_id,
                }))
                .await?
                {
                    Response::Handled => {}
                    _ => anyhow::bail!(
                        "Niri returned an unexpected response while focusing window {}",
                        previous_entry.window_id
                    ),
                }
            }
            Ok(ClientResponse::Success)
        }

        ClientCommand::Status => {
            let st = state.lock().await;
            let status_list = st
                .slots
                .iter()
                .enumerate()
                .map(|(idx, entry)| SlotInfo {
                    slot: idx + 1,
                    window_id: entry.map(|entry| entry.window_id),
                    title: entry.and_then(|entry| {
                        st.windows
                            .get(&entry.window_id)
                            .and_then(|window| window.title.clone())
                    }),
                })
                .collect();
            Ok(ClientResponse::Status(status_list))
        }
    }
}

/// Handles incoming commands from niri-shelf-ctl.
async fn handle_client(mut stream: UnixStream, state: Arc<Mutex<AppState>>) -> Result<()> {
    let mut reader = BufReader::new(&mut stream);
    let mut line = String::new();
    reader.read_line(&mut line).await?;
    let cmd: ClientCommand = serde_json::from_str(&line)?;
    let response = match process_command(cmd, state).await {
        Ok(response) => response,
        Err(error) => ClientResponse::Error {
            message: error.to_string(),
        },
    };
    let mut response = serde_json::to_string(&response)?;
    response.push('\n');
    stream.write_all(response.as_bytes()).await?;
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let state = Arc::new(Mutex::new(AppState::new()));

    // Spawn background task listening to niri events
    let event_state = Arc::clone(&state);
    tokio::spawn(async move {
        if let Err(e) = niri_event_listener(event_state).await {
            eprintln!("Niri event listener failed: {e}");
        }
    });

    // Setup local control socket
    let ctl_socket = get_control_socket_path();
    if ctl_socket.exists() {
        std::fs::remove_file(&ctl_socket)?;
    }

    let listener = UnixListener::bind(&ctl_socket)?;
    println!("niri-shelf-daemon listening on {:?}", ctl_socket);

    loop {
        let (stream, _) = listener.accept().await?;
        let client_state = Arc::clone(&state);
        tokio::spawn(async move {
            if let Err(e) = handle_client(stream, client_state).await {
                eprintln!("Error handling client command: {e}");
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slot_retains_the_original_workspace_for_restore() {
        let mut state = AppState::new();
        let entry = ShelvedWindow {
            window_id: 17,
            return_workspace_id: 42,
        };
        state.assign_slot(0, entry);

        assert_eq!(state.slots[0], Some(entry));
    }

    #[test]
    fn assigning_window_to_another_slot_preserves_only_one_slot_mapping() {
        let mut state = AppState::new();
        state.assign_slot(
            0,
            ShelvedWindow {
                window_id: 17,
                return_workspace_id: 42,
            },
        );
        state.assign_slot(
            1,
            ShelvedWindow {
                window_id: 17,
                return_workspace_id: 99,
            },
        );

        assert_eq!(state.slots[0], None);
        assert_eq!(
            state.slots[1],
            Some(ShelvedWindow {
                window_id: 17,
                return_workspace_id: 99,
            })
        );
    }
}
