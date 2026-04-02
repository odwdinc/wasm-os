#![no_std]

// Example shared types
pub type Result<T> = core::result::Result<T, Error>;

#[derive(Debug)]
pub enum Error {
    InvalidInput,
    FileNotFound,
    RuntimeError,
}