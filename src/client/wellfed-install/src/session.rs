//! A device API session as the shared steps use it: the backup's read and
//! the update key's enrollment run over this computer's own session during
//! an install, and over whatever session a CLI command holds (the bench
//! server's among them) on their own.

use wellfed_formats::private;
use wellfed_model::{Answer, ClientRole, Op};

use crate::Error;
use crate::platform::PlatformError;

/// An open session with a feeder.
#[expect(
    async_fn_in_trait,
    reason = "the run is driven on one thread (`run`'s docs), so no Send bound is asked of a \
              session's futures"
)]
pub trait Session {
    fn role(&self) -> ClientRole;

    /// Whether the feeder runs a pre-revision build (`Skew::Legacy`), which
    /// takes only the frozen update path: no client list, no status.
    fn legacy(&self) -> bool;

    /// One request under a fresh id, and its answer.
    async fn ask(&mut self, op: &Op) -> Result<Answer, PlatformError>;
}

impl Session for wellfed_connect::Connection {
    fn role(&self) -> ClientRole {
        Self::role(self)
    }

    fn legacy(&self) -> bool {
        self.skew() == wellfed_model::Skew::Legacy
    }

    async fn ask(&mut self, op: &Op) -> Result<Answer, PlatformError> {
        Ok(Self::ask(self, random()?, op).await?)
    }
}

/// Fresh bytes from the system's random source: request ids, ephemeral
/// keys, code bits.
pub fn random<const N: usize>() -> Result<[u8; N], Error> {
    private::random().map_err(Error::Random)
}
