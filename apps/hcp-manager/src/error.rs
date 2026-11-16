//! Errors with the exit codes shared by all fastroute programs.

use std::fmt;

pub mod exit {
    pub const FAILED: u8 = 1;
    pub const USAGE: u8 = 2;
    pub const INPUT: u8 = 3;
    pub const OUTPUT: u8 = 4;
    pub const PARTIAL: u8 = 5;
}

#[derive(Debug)]
pub struct Error {
    pub code: u8,
    pub message: String,
}

pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    pub fn new(code: u8, message: impl Into<String>) -> Error {
        Error { code, message: message.into() }
    }
    pub fn usage(m: impl Into<String>) -> Error {
        Error::new(exit::USAGE, m)
    }
    pub fn input(m: impl Into<String>) -> Error {
        Error::new(exit::INPUT, m)
    }
    pub fn output(m: impl Into<String>) -> Error {
        Error::new(exit::OUTPUT, m)
    }
    pub fn failed(m: impl Into<String>) -> Error {
        Error::new(exit::FAILED, m)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}
