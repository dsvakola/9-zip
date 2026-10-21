use serde::Serialize;
use std::fmt;

#[derive(Debug, Serialize)]
pub struct AppError {
    pub message: String,
    /// Lets the window react to some errors, such as asking for a password:
    /// "password_required" or "wrong_password".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<&'static str>,
}

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for AppError {}

impl AppError {
    pub fn new(msg: impl Into<String>) -> Self {
        Self {
            message: msg.into(),
            code: None,
        }
    }

    pub fn password_required() -> Self {
        Self {
            message: "This archive is protected with a password.".to_string(),
            code: Some("password_required"),
        }
    }

    pub fn wrong_password() -> Self {
        Self {
            message: "The password is wrong, or the archive is damaged.".to_string(),
            code: Some("wrong_password"),
        }
    }
}

impl From<std::io::Error> for AppError {
    fn from(err: std::io::Error) -> Self {
        AppError::new(format!("I/O Error: {}", err))
    }
}

impl From<zip::result::ZipError> for AppError {
    fn from(err: zip::result::ZipError) -> Self {
        use zip::result::ZipError;
        match err {
            ZipError::UnsupportedArchive(msg) if msg == ZipError::PASSWORD_REQUIRED => {
                AppError::password_required()
            }
            ZipError::InvalidPassword => AppError::wrong_password(),
            other => AppError::new(format!("ZIP Error: {}", other)),
        }
    }
}

impl From<sevenz_rust2::Error> for AppError {
    fn from(err: sevenz_rust2::Error) -> Self {
        match err {
            sevenz_rust2::Error::PasswordRequired => AppError::password_required(),
            sevenz_rust2::Error::MaybeBadPassword(_) => AppError::wrong_password(),
            other => AppError::new(format!("7Z Error: {}", other)),
        }
    }
}

impl From<serde_json::Error> for AppError {
    fn from(err: serde_json::Error) -> Self {
        AppError::new(format!("JSON Error: {}", err))
    }
}

impl From<String> for AppError {
    fn from(err: String) -> Self {
        AppError::new(err)
    }
}

impl From<&str> for AppError {
    fn from(err: &str) -> Self {
        AppError::new(err)
    }
}
