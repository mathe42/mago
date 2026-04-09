use std::fmt;

/// Errors that can occur in the LSP server.
#[derive(Debug)]
pub enum ServerError {
    /// IO error (transport, file operations).
    Io(std::io::Error),
    /// Protocol error from lsp-server.
    Protocol(lsp_server::ProtocolError),
    /// JSON serialization/deserialization error.
    Json(serde_json::Error),
    /// Database error from orchestrator.
    Orchestrator(mago_orchestrator::OrchestratorError),
    /// Generic error with message.
    Message(String),
}

impl fmt::Display for ServerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "IO error: {e}"),
            Self::Protocol(e) => write!(f, "protocol error: {e}"),
            Self::Json(e) => write!(f, "JSON error: {e}"),
            Self::Orchestrator(e) => write!(f, "orchestrator error: {e}"),
            Self::Message(msg) => f.write_str(msg),
        }
    }
}

impl std::error::Error for ServerError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            Self::Protocol(e) => Some(e),
            Self::Json(e) => Some(e),
            _ => None,
        }
    }
}

impl From<std::io::Error> for ServerError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<lsp_server::ProtocolError> for ServerError {
    fn from(e: lsp_server::ProtocolError) -> Self {
        Self::Protocol(e)
    }
}

impl From<serde_json::Error> for ServerError {
    fn from(e: serde_json::Error) -> Self {
        Self::Json(e)
    }
}

impl From<lsp_server::ExtractError<lsp_server::Request>> for ServerError {
    fn from(e: lsp_server::ExtractError<lsp_server::Request>) -> Self {
        Self::Message(format!("extract request error: {e}"))
    }
}

impl From<lsp_server::ExtractError<lsp_server::Notification>> for ServerError {
    fn from(e: lsp_server::ExtractError<lsp_server::Notification>) -> Self {
        Self::Message(format!("extract notification error: {e}"))
    }
}

impl From<mago_orchestrator::OrchestratorError> for ServerError {
    fn from(e: mago_orchestrator::OrchestratorError) -> Self {
        Self::Orchestrator(e)
    }
}
