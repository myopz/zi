// Agent runtime errors.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum AgentError {
    #[error("agent is already processing a prompt")]
    AlreadyRunning,

    #[error("no messages to continue from")]
    NothingToContinue,

    #[error("cannot continue from assistant message")]
    ContinueFromAssistant,

    #[error("{0}")]
    Message(String),
}
