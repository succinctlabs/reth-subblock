use thiserror::Error;

/// Network Errors
#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum NetworkError {
    /// Indicates that the sender has been dropped.
    #[error("sender has been dropped")]
    ChannelClosed,
}
