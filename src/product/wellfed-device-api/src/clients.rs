//! The enrolled clients as the server admits them: a client is its static
//! key, a label the owner sees and a role the server enforces. The feeder's
//! client store keeps the table on flash in its own layout; these are the
//! values it holds.

use wellfed_session::named_enum;

/// Enrolled clients, apart from the four concurrent sessions.
pub const CLIENTS: usize = 8;
pub const LABEL_LEN: usize = 16;

named_enum! {
    /// What a client may do; the device API enforces it.
    pub enum Role {
        /// Everything, including inviting and revoking clients.
        Owner = "owner",
        /// Feeding and settings, but no client management.
        Member = "member",
        /// The owner's optional server: feeding, settings, plans and the
        /// lock as a member, but not the portion weight, the meal call or
        /// invites (the device API's access classes say exactly).
        Server = "server",
        /// An owner's Ed25519 key for update signatures.
        UpdateSigner = "update-signer",
    }
}

impl Role {
    pub const fn code(self) -> u8 {
        match self {
            Self::Owner => 1,
            Self::Member => 2,
            Self::Server => 3,
            Self::UpdateSigner => 4,
        }
    }

    /// Whether a key holding this role may already do everything `asked`
    /// grants: the same role, or an owner's for a role that opens a
    /// session. An enrollment that finds its key already enrolled has
    /// granted nothing, so it stands only if this holds.
    pub const fn covers(self, asked: Self) -> bool {
        matches!(
            (self, asked),
            (Self::Owner, Self::Owner | Self::Member | Self::Server)
                | (Self::Member, Self::Member)
                | (Self::Server, Self::Server)
                | (Self::UpdateSigner, Self::UpdateSigner)
        )
    }

    pub const fn from_byte(code: u8) -> Option<Self> {
        match code {
            1 => Some(Self::Owner),
            2 => Some(Self::Member),
            3 => Some(Self::Server),
            4 => Some(Self::UpdateSigner),
            _ => None,
        }
    }
}

/// A client's name as the owner sees it: 1-16 bytes, no NUL.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Label {
    bytes: [u8; LABEL_LEN],
    len: u8,
}

impl Label {
    pub fn new(text: &[u8]) -> Option<Self> {
        if text.is_empty() || text.len() > LABEL_LEN || text.contains(&0) {
            return None;
        }
        let mut bytes = [0; LABEL_LEN];
        bytes[..text.len()].copy_from_slice(text);
        Some(Self {
            bytes,
            len: u8::try_from(text.len()).ok()?,
        })
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes[..usize::from(self.len)]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Client {
    pub key: [u8; 32],
    pub label: Label,
    pub role: Role,
    /// The store generation that enrolled it.
    pub enrolled: u64,
}
