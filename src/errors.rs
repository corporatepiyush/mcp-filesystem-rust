use thiserror::Error;

#[derive(Error, Debug)]
pub enum MCSError {
    #[error("Parse error: {0}")]
    ParseError(String),

    #[error("Method not found: {0}")]
    MethodNotFound(String),

    #[error("Invalid params: {0}")]
    InvalidParams(String),

    #[error("Filesystem error: {0}")]
    FilesystemError(String),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    JsonError(#[from] serde_json::Error),

    #[error("Path not allowed: {0}")]
    PathNotAllowed(String),

    #[error("Path not found: {0}")]
    PathNotFound(String),
}

impl MCSError {
    pub const fn error_code(&self) -> i64 {
        match self {
            MCSError::ParseError(_) => -32700,
            MCSError::MethodNotFound(_) => -32601,
            MCSError::InvalidParams(_) => -32602,
            MCSError::FilesystemError(_) => -32000,
            MCSError::IoError(_) => -32003,
            MCSError::JsonError(_) => -32700,
            MCSError::PathNotAllowed(_) => -32004,
            MCSError::PathNotFound(_) => -32005,
        }
    }
}

pub type Result<T> = std::result::Result<T, MCSError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_codes() {
        assert_eq!(MCSError::ParseError("".into()).error_code(), -32700);
        assert_eq!(MCSError::MethodNotFound("".into()).error_code(), -32601);
        assert_eq!(MCSError::InvalidParams("".into()).error_code(), -32602);
        assert_eq!(MCSError::FilesystemError("".into()).error_code(), -32000);
        assert_eq!(MCSError::PathNotAllowed("".into()).error_code(), -32004);
        assert_eq!(MCSError::PathNotFound("".into()).error_code(), -32005);
    }

    #[test]
    fn test_io_error_conversion() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "file not found");
        let mcs_err: MCSError = io_err.into();
        assert_eq!(mcs_err.error_code(), -32003);
    }
}
