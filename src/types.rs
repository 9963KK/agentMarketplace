use std::error::Error;
use std::fmt;

macro_rules! id_type {
    ($name:ident, $display:literal, $error:ident) => {
        #[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Self {
                Self::try_new(value).expect(concat!($display, " must not be empty"))
            }

            pub fn try_new(value: impl Into<String>) -> Result<Self, CommonTypeError> {
                let value = value.into();
                if value.trim().is_empty() {
                    return Err(CommonTypeError::$error);
                }

                Ok(Self(value))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl From<&str> for $name {
            fn from(value: &str) -> Self {
                Self::new(value)
            }
        }

        impl From<String> for $name {
            fn from(value: String) -> Self {
                Self::new(value)
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

id_type!(TaskId, "task id", EmptyTaskId);
id_type!(SessionId, "session id", EmptySessionId);
id_type!(AssignmentId, "assignment id", EmptyAssignmentId);
id_type!(OutputHash, "output hash", EmptyOutputHash);

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct Timestamp(pub u64);

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CommonTypeError {
    EmptyTaskId,
    EmptySessionId,
    EmptyAssignmentId,
    EmptyOutputHash,
}

impl fmt::Display for CommonTypeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CommonTypeError::EmptyTaskId => f.write_str("task id must not be empty"),
            CommonTypeError::EmptySessionId => f.write_str("session id must not be empty"),
            CommonTypeError::EmptyAssignmentId => f.write_str("assignment id must not be empty"),
            CommonTypeError::EmptyOutputHash => f.write_str("output hash must not be empty"),
        }
    }
}

impl Error for CommonTypeError {}
