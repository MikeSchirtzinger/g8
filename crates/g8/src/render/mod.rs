//! Output rendering — pretty (Markdown/ANSI) and JSON.

pub mod json;
pub mod pretty;

/// Map the store's pairing-error reason vocabulary onto the shared
/// `g8_core::contract::PairingReason` both renderers emit.
///
/// A free function here rather than a `From` impl on either type: `g8-core`
/// is the root of the crate DAG and cannot depend on `g8-store` (see
/// `g8_core::contract`'s module docs), so the mapping has to live in a
/// crate that already depends on both — that's `g8`, not either type's
/// own crate.
pub(crate) fn to_pairing_reason(
    reason: g8_store::PairingErrorReason,
) -> g8_core::contract::PairingReason {
    use g8_core::contract::PairingReason as Contract;
    use g8_store::PairingErrorReason as Store;
    match reason {
        Store::NoMatchingConvergenceTest => Contract::NoMatchingConvergenceTest,
        Store::StubExpired => Contract::StubExpired,
        Store::StubMissingSince => Contract::StubMissingSince,
        Store::DuplicateCapabilityName => Contract::DuplicateCapabilityName,
    }
}
