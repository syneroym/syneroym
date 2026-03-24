//! Core traits and types for Syneroym.

use anyhow::Result;

pub mod config;

pub trait SubstrateSubsystem {
    fn init(&mut self) -> impl std::future::Future<Output = Result<()>> + Send;
    fn run(&mut self) -> impl std::future::Future<Output = Result<()>> + Send;
    fn shutdown(&mut self) -> impl std::future::Future<Output = Result<()>> + Send;
}
