//! Owner — optional attribution attached to Intent records.
//!
//! An owner can reference a Claude Code subagent (via `.claude/agents/<name>.md`),
//! a human team label, and/or a contact channel (email, Slack, etc.).  All
//! three fields are optional; an Owner with all-`None` fields is still valid
//! (it functions as a placeholder for future attribution).

use serde::{Deserialize, Serialize};

/// Optional attribution for an intent or capability.
///
/// # Examples
///
/// ```
/// use g8_core::Owner;
///
/// let owner = Owner {
///     agent:   Some("scorer-specialist".into()),
///     team:    Some("core-ranker".into()),
///     contact: Some("#checkout-api-core".into()),
/// };
/// assert_eq!(owner.agent.as_deref(), Some("scorer-specialist"));
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Owner {
    /// References `.claude/agents/<name>.md` (without the `.md` suffix).
    pub agent: Option<String>,
    /// Human team name.
    pub team: Option<String>,
    /// Email, Slack channel, or other contact string.
    pub contact: Option<String>,
}

impl Owner {
    /// Returns `true` if all fields are `None`.
    pub fn is_empty(&self) -> bool {
        self.agent.is_none() && self.team.is_none() && self.contact.is_none()
    }
}
