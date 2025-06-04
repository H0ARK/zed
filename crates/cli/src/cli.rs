use collections::HashMap;
pub use ipc_channel::ipc;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub struct IpcHandshake {
    pub requests: ipc::IpcSender<CliRequest>,
    pub responses: ipc::IpcReceiver<CliResponse>,
}

#[derive(Debug, Serialize, Deserialize)]
pub enum CliRequest {
    Open {
        paths: Vec<String>,
        urls: Vec<String>,
        wait: bool,
        open_new_workspace: Option<bool>,
        env: Option<HashMap<String, String>>,
        user_data_dir: Option<String>,
    },
    Agent {
        message: String,
        working_directory: Option<String>,
        provider: Option<String>,
        model: Option<String>,
        copilot_auth: bool,
        session: Option<String>,
    },
    CopilotAuth,
    ListSessions,
    RemoveSession {
        session_id: String,
    },
    SessionStats,
    CreateSession {
        name: String,
        provider: Option<String>,
        model: Option<String>,
        working_directory: Option<String>,
        auto_continue: bool,
        context_sharing: bool,
    },
    JoinSession {
        session_id: String,
    },
    SetActiveSession {
        session_id: String,
    },
    CloneSession {
        source_session_id: String,
        new_name: String,
    },
    SendMessageToSession {
        session_id: String,
        message: String,
    },
    ExportSession {
        session_id: String,
        output_path: Option<String>,
    },
    ImportSession {
        input_path: String,
    },
    SessionInfo {
        session_id: String,
    },
}

#[derive(Debug, Serialize, Deserialize)]
pub enum CliResponse {
    Ping,
    Stdout { message: String },
    Stderr { message: String },
    Exit { status: i32 },
    SessionCreated { session_id: String },
    SessionJoined { session_id: String },
    SessionActivated { session_id: String },
    SessionCloned { session_id: String },
    SessionExported { path: String },
    SessionImported { session_id: String },
    SessionDetails { json_data: String },
    MessageSent { session_id: String, message_id: Option<String> },
}

/// When Zed started not as an *.app but as a binary (e.g. local development),
/// there's a possibility to tell it to behave "regularly".
pub const FORCE_CLI_MODE_ENV_VAR_NAME: &str = "ZED_FORCE_CLI_MODE";
