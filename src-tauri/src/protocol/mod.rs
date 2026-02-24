//! Claude CLI Control Protocol
//!
//! This module provides the implementation of the Claude CLI control protocol
//! for enabling tool approval, agent interruption, and permission mode switching.

pub mod client;
pub mod peer;
pub mod types;

pub use client::{ApprovalClient, ApprovalRequest, RuleBasedApprovalClient};
pub use peer::{ControlRequestHandler, DefaultHandler, ProtocolPeer};
pub use types::{ApprovalStatus, ControlRequestType, PermissionMode};
