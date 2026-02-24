//! Approval system for agent tool execution
//!
//! This module provides the approval system for controlling agent tool execution,
//! including permission checking and manual approval workflows.

pub mod manager;
pub mod service;

pub use manager::{AgentPermissionConfig, ApprovalManager};
pub use service::{ApprovalEvent, ApprovalEventSender, ApprovalMode, ExecutorApprovalError, ExecutorApprovalService};
