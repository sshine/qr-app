//! Who is looking.

use crate::admin::AdminIdentity;

/// Passed to every render, because the forwarding handler shows its editor at the same
/// URL an anonymous visitor is redirected from, and that is the only thing that tells
/// the two apart.
#[derive(Debug, Clone, Default)]
pub struct Viewer {
    pub admin: Option<AdminIdentity>,
}

impl Viewer {
    pub fn is_admin(&self) -> bool {
        self.admin.is_some()
    }
}
